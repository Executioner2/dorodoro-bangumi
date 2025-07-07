//! 接收客户端发来的控制信号

use crate::core::context::Context;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::{CONTROLLER_PREFIX, SCHEDULER, TCP_SERVER};
use crate::core::scheduler::command;
use crate::core::scheduler::command::Shutdown;
use crate::core::tcp_server::command::Exit;
use crate::torrent::{Parse, TorrentArc};
use std::fs;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc::channel;
use tracing::{info, trace, warn};
use crate::config::CHANNEL_BUFFER;

/// 响应给客户端的内容
pub struct _RespClient {}

pub struct Controller {
    /// 控制器id
    id: u64,

    /// 接收控制客户端请求的 tcp stream
    socket: TcpStream,

    /// 全局上下文
    context: Context,

    /// 指令发射器
    emitter: Emitter,
}

impl Controller {
    pub fn new(id: u64, socket: TcpStream, context: Context, emitter: Emitter) -> Self {
        Self {
            id,
            socket,
            context,
            emitter,
        }
    }

    fn get_transfer_id(&self) -> String {
        format!("{}{}", CONTROLLER_PREFIX, self.id)
    }

    async fn shutdown(self) {
        let transfer_id = self.get_transfer_id();
        self.emitter.remove(&transfer_id);
        info!("控制器[{}]已退出", transfer_id);
    }
}

impl Controller {
    pub async fn run(mut self) {
        let (send, mut recv) = channel(CHANNEL_BUFFER);
        let transfer_id = self.get_transfer_id();
        self.emitter.register(transfer_id, send);

        info!("控制器启动");
        loop {
            select! {
                _ = self.context.cancelled() => {
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
                            // let data = fs::read("tests/resources/test6.torrent").unwrap(); // test6 是 17GB 多文件的
                            let data = fs::read("tests/resources/test5.torrent").unwrap();
                            let path = PathBuf::from("./download/");
                            let torrent = TorrentArc::parse_torrent(data).unwrap();
                            let cmd = command::TorrentAdd {
                                torrent,
                                path
                            };
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
