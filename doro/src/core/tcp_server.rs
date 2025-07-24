use crate::control::ControlStatus;
use crate::core::context::Context;
use crate::core::control::Dispatcher;
use crate::core::emitter::constant::TCP_SERVER;
use crate::core::protocol::{Identifier, Protocol};
use crate::core::runtime::Runnable;
use crate::core::tcp_server::future::Accept;
use crate::emitter::transfer::TransferPtr;
use doro_util::net::FutureRet;
use crate::protocol::remote_control;
use crate::protocol::remote_control::HandshakeParse;
use crate::runtime::{CommandHandleResult, CustomTaskResult, ExitReason, FuturePin, RunContext};
use anyhow::{Result, anyhow};
use dashmap::DashMap;
use futures::stream::FuturesUnordered;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio::task::JoinHandle;
use tokio_util::sync::WaitForCancellationFuture;
use tracing::{error, info, trace, warn};
use doro_util::global::{GlobalId, Id};
use doro_util::sync::wait_join_handle_close;

mod future;

struct ConnInfo {
    join_handle: JoinHandle<()>,
}

struct ConnContext {
    id: Id,
    socket: Option<TcpStream>,
    conns: Arc<DashMap<Id, ConnInfo>>,
}

impl ConnContext {
    fn remove_conn(&self) {
        self.conns.remove(&self.id);
    }
}

pub struct TcpServer {
    /// 监听地址
    addr: SocketAddr,

    /// Tcp Server 监听器
    listener: Option<TcpListener>,

    /// 链接
    conns: Arc<DashMap<Id, ConnInfo>>,
}

impl TcpServer {
    pub fn new() -> Self {
        let addr = Context::global().get_config().tcp_server_addr();
        TcpServer {
            addr,
            listener: None,
            conns: Arc::new(DashMap::default())
        }
    }

    fn get_conn_context(socket: TcpStream, cc: Arc<DashMap<Id, ConnInfo>>) -> ConnContext {
        ConnContext {
            id: GlobalId::global().next_id(),
            socket: Some(socket),
            conns: cc,
        }
    }

    async fn accept(mut cc: ConnContext) {
        let mut socket = cc.socket.take().unwrap();
        let addr = socket.peer_addr().unwrap();
        let mut accept = Accept::new(&mut socket, &addr);
        select! {
            _ = Context::global().cancelled() => {
                trace!("accpet socket 接收到关机信号");
            },
            result = &mut accept => {
                match result {
                    FutureRet::Ok(protocol) => {
                        if let Err(e) = Self::protocol_dispatch(socket, protocol, cc.id).await {
                            error!("处理协议失败: {}", e);
                        }
                    },
                    res => {
                        trace!("断开此链接，因为得到了预期之外的结果: {:?}", res);
                    }
                }
            }
        }

        cc.remove_conn();
    }

    /// 协议分发，根据协议 ID 分发到不同的处理函数
    #[inline(always)]
    async fn protocol_dispatch(mut socket: TcpStream, protocol: Protocol, id: Id) -> Result<()> {
        match protocol.id {
            Identifier::BitTorrent => {
                trace!("接收到 BitTorrent 协议");
            }
            Identifier::RemoteControl => {
                trace!("接收到 RemoteControl 协议");
                // 解析 RemoteControl 的握手协议附加数据
                let addr = socket.peer_addr()?;
                match HandshakeParse::new(&mut socket, &addr).await {
                    #[rustfmt::skip]
                    FutureRet::Ok(auth_info) => {
                        let client_auth = Context::global().get_config().client_auth();
                        if let Err(e) = remote_control::auth_verify(client_auth, &auth_info) {
                            remote_control::send_handshake_failed(
                                &mut socket,
                                ControlStatus::AuthFailed,
                                e.to_string(),
                            ).await?;
                        } else {
                            remote_control::send_handshake_success(&mut socket).await?;
                        }

                        let dispatcher = Dispatcher::new(id, socket);
                        dispatcher.run().await;
                    }
                    FutureRet::Err(e) => {
                        return Err(anyhow!("解析 RemoteControl 握手协议失败: {}", e));
                    }
                }
            }
        }
        Ok(())
    }

    /// 监听器接收连接
    fn listener_accept(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>> {
        let listener = self.listener.take().unwrap();
        let conns = self.conns.clone();
        Box::pin(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, addr)) => {
                        trace!("tcp server 接收到连接: {}", addr);
                        let cc = Self::get_conn_context(socket, conns.clone());
                        let id = cc.id;
                        let join_handle = tokio::spawn(Self::accept(cc).pin());
                        let conn_info = ConnInfo { join_handle };
                        conns.insert(id, conn_info);
                    }
                    Err(e) => {
                        warn!("tcp server 接收连接错误: {}", e);
                    }
                }
            }
        })
    }
}

impl Runnable for TcpServer {
    fn get_transfer_id<T: ToString>(_suffix: T) -> String {
        TCP_SERVER.to_string()
    }

    fn register_lt_future(
        &mut self,
    ) -> FuturesUnordered<Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>>> {
        let futures = FuturesUnordered::new();
        futures.push(self.listener_accept());
        futures
    }

    async fn run_before_handle(&mut self, _rc: RunContext) -> Result<()> {
        let listener: TcpListener;
        match TcpListener::bind(&self.addr).await {
            Ok(l) => {
                info!("tcp server 正在监听 {}", self.addr);
                listener = l;
            }
            Err(e) => {
                Context::global().cancel();
                return Err(anyhow!("tcp server 绑定地址失败: {}", e));
            }
        }
        self.listener = Some(listener);
        Ok(())
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        Context::global().cancelled()
    }

    async fn command_handle(&mut self, _cmd: TransferPtr) -> Result<CommandHandleResult> {
        // tcp_server 暂时没有实现 command 处理
        // let cmd: Command = cmd.instance();
        // cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }

    async fn shutdown(&mut self, _reason: ExitReason) {
        trace!("等待关闭的子线程数量: {}", self.conns.len());
        for mut conn in self.conns.iter_mut() {
            wait_join_handle_close(&mut conn.join_handle).await;
        }
    }
}
