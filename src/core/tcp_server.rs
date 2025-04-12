use crate::core::command;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace, warn};
use crate::core::runtime::Runnable;

type SenderArc = Arc<Sender<command::scheduler::Command>>;

pub struct TcpServer {
    /// 向调度器发送命令
    send: SenderArc,

    /// 监听地址
    addr: SocketAddr,

    /// 关机信号
    cancel_token: CancellationToken,
}

impl TcpServer {
    pub fn new(addr: SocketAddr, cancel_token: CancellationToken, send: SenderArc) -> Self {
        TcpServer {
            send,
            addr,
            cancel_token,
        }
    }

    async fn accept(_socket: TcpStream, _send: SenderArc) {
        todo!()
    }
}


impl Runnable for TcpServer {
    async fn run(self) {
        let listener = TcpListener::bind(&self.addr).await.unwrap();
        info!("tcp server 正在监听 {}", self.addr);
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    trace!("tcp server 接收到关机信号");
                    break;
                }
                recv = listener.accept() => {
                    match recv {
                        Ok((socket, addr)) => {
                            trace!("tcp server 接收到连接: {}", addr);
                            tokio::spawn(Self::accept(socket, self.send.clone()));
                        },
                        Err(e) => {
                            warn!("tcp server 接收连接错误: {}", e);
                        }
                    }
                }
            }
        }
        info!("tcp server 已关闭");
    }
}