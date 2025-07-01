use crate::core::command::CommandHandler;
use crate::core::context::Context;
use crate::core::controller::Controller;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::TCP_SERVER;
use crate::core::protocol::{Identifier, Protocol};
use crate::core::runtime::Runnable;
use crate::core::tcp_server::command::Command;
use crate::core::tcp_server::future::Accept;
use dashmap::DashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio::task::JoinHandle;
use tokio_util::sync::WaitForCancellationFuture;
use tracing::{info, trace, warn};
use crate::emitter::transfer::TransferPtr;
use crate::runtime::{CommandHandleResult, CustomTaskResult, ExitReason, RunContext};
use anyhow::{anyhow, Result};
use futures::stream::FuturesUnordered;

pub mod command;
mod future;

/// 链接id，一般需要 TcpServer 管理资源释放的才需要这个
type ConnId = u64;

struct ConnInfo {
    join_handle: JoinHandle<()>,
}

/// 多线程下的共享数据
#[derive(Clone)]
pub struct TcpServerContext {
    /// 链接增长 id
    conn_id: Arc<AtomicU64>,

    /// 链接
    conns: Arc<DashMap<ConnId, ConnInfo>>,

    /// 全局上下文
    context: Context,
    
    /// 命令发射器
    emitter: Emitter,
}

impl TcpServerContext {
    fn new(context: Context, emitter: Emitter) -> Self {
        Self {
            conn_id: Arc::new(AtomicU64::new(0)),
            conns: Arc::new(DashMap::default()),
            context,
            emitter,
        }    
    }
    
    pub async fn remove_conn(&self, conn_id: ConnId) {
        self.conns.remove(&conn_id);
    }
}

pub struct TcpServer {
    /// tcp server 上下文
    tsc: TcpServerContext,
    
    /// 监听地址
    addr: SocketAddr,

    /// Tcp Server 监听器
    listener: Option<TcpListener>
}

impl TcpServer {
    pub fn new(context: Context, emitter: Emitter) -> Self {
        let addr = context.get_config().tcp_server_addr();
        let tsc = TcpServerContext::new(context, emitter);
        
        TcpServer {
            tsc,
            addr,
            listener: None,
        }
    }

    async fn accept(mut socket: TcpStream, tc: TcpServerContext) {
        let addr = socket.peer_addr().unwrap();
        let mut accept = Accept::new(&mut socket, &addr);
        select! {
            _ = tc.context.cancelled() => {
                trace!("accpet socket 接收到关机信号");
            },
            result = &mut accept => {
                match result {
                    Some(protocol) => {
                        Self::protocol_dispatch(tc, socket, protocol).await;
                    },
                    None => {
                        trace!("accpet socket 接收到关闭信号");
                    }
                }
            }
        }
    }

    /// 协议分发，根据协议 ID 分发到不同的处理函数
    #[inline(always)]
    async fn protocol_dispatch(tc: TcpServerContext, socket: TcpStream, protocol: Protocol) {
        match protocol.id {
            Identifier::BitTorrent => {
                trace!("接收到 BitTorrent 协议");
            }
            Identifier::RemoteControl => {
                trace!("接收到 RemoteControl 协议");
                let id = tc
                    .conn_id
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let controller = Controller::new(id, socket, tc.context, tc.emitter);
                controller.run().await;
            }
        }
    }
    
    /// 监听器接收连接
    fn listener_accept(&mut self) -> Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>> {
        let listener = self.listener.take().unwrap();
        let tsc = self.tsc.clone();
        Box::pin(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, addr)) => {
                        trace!("tcp server 接收到连接: {}", addr);
                        tokio::spawn(Self::accept(socket, tsc.clone()));
                    },
                    Err(e) => {
                        warn!("tcp server 接收连接错误: {}", e);
                    }
                }
            }
        })
    }
}

impl Runnable for TcpServer {
    fn emitter(&self) -> &Emitter {
        &self.tsc.emitter
    }

    fn get_transfer_id<T: ToString>(_suffix: T) -> String {
        TCP_SERVER.to_string()
    }

    fn register_lt_future(&mut self) -> FuturesUnordered<Pin<Box<dyn Future<Output=CustomTaskResult> + Send + 'static>>> {
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
                self.tsc.context.cancel();
                return Err(anyhow!("tcp server 绑定地址失败: {}", e));
            }
        }
        self.listener = Some(listener);
        Ok(())
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.tsc.context.cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }

    async fn shutdown(&mut self, _reason: ExitReason) {
        trace!("等待关闭的子线程数量: {}", self.tsc.conns.len());
        for mut conn in self.tsc.conns.iter_mut() {
            let join_handle = &mut conn.join_handle;
            join_handle.await.unwrap()
        }
    }
}
