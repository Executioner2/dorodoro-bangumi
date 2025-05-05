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
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio::task::JoinHandle;
use tracing::{error, info, trace, warn};

pub mod command;
mod error;
mod future;

/// 链接id，一般需要 TcpServer 管理资源释放的才需要这个
type ConnId = u64;

struct ConnInfo {
    join_handle: JoinHandle<()>,
}

/// 多线程下的共享数据
pub struct TcpServerContext {
    conn_id: Arc<AtomicU64>,
    conns: Arc<DashMap<ConnId, ConnInfo>>,
    context: Context,
    emitter: Emitter,
}

impl TcpServerContext {
    pub async fn remove_conn(&self, conn_id: ConnId) {
        self.conns.remove(&conn_id);
    }
}

pub struct TcpServer {
    /// 监听地址
    addr: SocketAddr,

    /// 链接增长 id
    conn_id: Arc<AtomicU64>,

    /// 链接
    conns: Arc<DashMap<ConnId, ConnInfo>>,

    /// 全局上下文
    context: Context,

    /// 命令发射器
    emitter: Emitter,
}

impl TcpServer {
    pub fn new(context: Context, emitter: Emitter) -> Self {
        TcpServer {
            addr: context.get_config().tcp_server_addr(),
            conn_id: Arc::new(AtomicU64::new(0)),
            conns: Arc::new(DashMap::default()),
            context,
            emitter,
        }
    }

    fn get_context(&self) -> TcpServerContext {
        TcpServerContext {
            conn_id: self.conn_id.clone(),
            conns: self.conns.clone(),
            context: self.context.clone(),
            emitter: self.emitter.clone(),
        }
    }

    async fn shutdown(self) {
        self.emitter.remove(TCP_SERVER);
        trace!("等待关闭的子线程数量: {}", self.conns.len());
        for mut conn in self.conns.iter_mut() {
            let join_handle = &mut conn.join_handle;
            join_handle.await.unwrap()
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
}

impl Runnable for TcpServer {
    async fn run(mut self) {
        let listener: TcpListener;
        match TcpListener::bind(&self.addr).await {
            Ok(l) => {
                info!("tcp server 正在监听 {}", self.addr);
                listener = l;
            }
            Err(e) => {
                error!("tcp server 绑定地址失败: {}", e);
                self.context.cancel();
                return;
            }
        }

        // 注册接收器
        let (send, mut recv) = channel(self.context.get_config().channel_buffer());
        self.emitter.register(TCP_SERVER, send);

        loop {
            select! {
                _ = self.context.cancelled() => {
                    break;
                }
                res = listener.accept() => {
                    match res {
                        Ok((socket, addr)) => {
                            trace!("tcp server 接收到连接: {}", addr);
                            let context = self.get_context();
                            tokio::spawn(Self::accept(socket, context));
                        },
                        Err(e) => {
                            warn!("tcp server 接收连接错误: {}", e);
                        }
                    }
                }
                res = recv.recv() => {
                    if let Some(cmd) = res {
                        let cmd: Command = cmd.instance();
                        trace!("tcp server 收到了消息: {:?}", cmd);
                        if let Err(e) = cmd.handle(&mut self).await {
                            error!("处理指令出现错误\t{}", e);
                            break;
                        }
                    }
                }
            }
        }

        info!("tcp server 等待资源释放");
        self.shutdown().await;
        info!("tcp server 已关闭");
    }
}
