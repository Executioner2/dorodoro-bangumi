//! 接收客户端发来的控制信号

use crate::core::scheduler::command;
use crate::core::scheduler::command::Shutdown;
use crate::core::tcp_server::command::Exit;
use crate::core::config::Config;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::{CONTROLLER_PREFIX, PEER_MANAGER, SCHEDULER, TCP_SERVER};
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
    config: Config,
    emitter: Emitter,
}

impl Controller {
    pub fn new(
        id: u64,
        socket: TcpStream,
        cancel_token: CancellationToken,
        config: Config,
        emitter: Emitter,
    ) -> Self {
        Self {
            id,
            socket,
            cancel_token,
            config,
            emitter,
        }
    }
    
    fn get_transfer_id(&self) -> String {
        format!("{}{}", CONTROLLER_PREFIX, self.id)
    }
    
    async fn shutdown(self) {
        let transfer_id = self.get_transfer_id();
        self.emitter.remove(&transfer_id).await.unwrap();
        info!("控制器[{}]已退出", transfer_id);
    }
}

impl Runnable for Controller {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.config.channel_buffer());
        let transfer_id = self.get_transfer_id();
        self.emitter.register(transfer_id, send).await.unwrap();

        info!("控制器启动");
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
                            self.emitter.send(SCHEDULER, Shutdown.into()).await.unwrap();
                            break;
                        },
                        Ok(1) => {
                            trace!("接收到了添加种子文件的指令");
                            let data = fs::read("tests/resources/test4.torrent").unwrap();
                            let download_path = String::from("./");
                            let torrent = TorrentArc::parse_torrent(data).unwrap();
                            let cmd = command::TorrentAdd(torrent, download_path);
                            self.emitter.send(SCHEDULER, cmd.into()).await.unwrap();
                            break;
                        },
                        Ok(_) => {
                            warn!("暂不支持的操作");
                        }
                        Err(_) => {
                            trace!("Controller socket closed");
                            self.emitter.send(TCP_SERVER, Exit(self.id).into()).await.unwrap();
                            break;
                        }
                    }
                },
                _result = recv.recv() => {
                    todo!("")
                }
            }
        }
        
        self.shutdown().await;
    }
}
