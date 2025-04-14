//! 接收客户端发来的控制信号

use crate::core::alias::{SenderScheduler, SenderTcpServer};
use crate::core::command::scheduler::Shutdown;
use crate::core::command::tcp_server::Exit;
use crate::core::runtime::Runnable;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace, warn};

pub struct Controller {
    id: u64,
    socket: TcpStream,
    cancel_token: CancellationToken,
    ss: SenderScheduler,
    sts: SenderTcpServer,
}

impl Controller {
    pub fn new(
        id: u64,
        socket: TcpStream,
        cancel_token: CancellationToken,
        ss: SenderScheduler,
        sts: SenderTcpServer,
    ) -> Self {
        Self {
            id,
            socket,
            cancel_token,
            ss,
            sts,
        }
    }
}

impl Runnable for Controller {
    async fn run(mut self) {
        loop {
            select! {
                _ = self.cancel_token.cancelled() => {
                    trace!("控制器接收到退出指令");
                    break;
                },
                result = self.socket.read_u8() => {
                    match result {
                        Ok(0) => {
                            trace!("接收到了关机指令");
                            self.ss.send(Shutdown.into()).await.unwrap();
                            break;
                        },
                        Ok(_) => {
                            warn!("暂不支持的操作");
                        }
                        Err(_) => {
                            trace!("Controller socket closed");
                            self.sts.send(Exit(self.id).into()).await.unwrap();
                            break;
                        }
                    }
                }
            }
        }
        info!("控制器退出");
    }
}
