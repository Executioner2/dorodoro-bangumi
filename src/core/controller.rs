//! 接收客户端发来的控制信号

use crate::core::alias::{ReceiverController, SenderController, SenderScheduler, SenderTcpServer};
use crate::core::command::scheduler;
use crate::core::command::scheduler::Shutdown;
use crate::core::command::tcp_server::Exit;
use crate::core::config::Config;
use crate::core::runtime::Runnable;
use crate::torrent::{Parse, TorrentArc};
use std::fs;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace, warn};

/// 响应给客户端的内容
pub struct RespClient {}

pub struct Controller {
    id: u64,
    socket: TcpStream,
    cancel_token: CancellationToken,
    ss: SenderScheduler,
    sts: SenderTcpServer,
    channel: (SenderController, ReceiverController),
    config: Config,
}

impl Controller {
    pub fn new(
        id: u64,
        socket: TcpStream,
        cancel_token: CancellationToken,
        ss: SenderScheduler,
        sts: SenderTcpServer,
        config: Config,
    ) -> Self {
        Self {
            id,
            socket,
            cancel_token,
            ss,
            sts,
            channel: channel(config.channel_buffer()),
            config,
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
                        Ok(1) => {
                            trace!("接收到了添加种子文件的指令");
                            let data = fs::read("tests/resources/test3.torrent").unwrap();
                            let download_path = String::from("./");
                            let torrent = TorrentArc::parse_torrent(data).unwrap();
                            let cmd = scheduler::TorrentAdd(self.channel.0.clone(), torrent, download_path).into();
                            self.ss.send(cmd).await.unwrap();
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
