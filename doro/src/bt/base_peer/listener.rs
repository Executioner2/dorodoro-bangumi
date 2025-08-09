use std::net::SocketAddr;

use anyhow::anyhow;
use doro_util::global::Id;
use doro_util::is_disconnect;
use doro_util::net::FutureRet;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, trace};

use super::command::{Exit, PeerTransfer};
use super::peer_resp::PeerResp;
use super::peer_resp::RespType::*;
use super::rate_control::PacketAck;
use super::{MsgType, command};
use crate::base_peer::error::{PeerExitReason, exception};
use crate::bt::socket::{OwnedReadHalfExt, OwnedWriteHalfExt};
use crate::emitter::transfer::TransferPtr;

pub struct WriteFuture {
    pub(super) id: Id,
    pub(super) writer: OwnedWriteHalfExt,
    pub(super) addr: SocketAddr,
    pub(super) peer_sender: Sender<TransferPtr>,
    pub(super) recv: Receiver<Vec<u8>>,
}

impl WriteFuture {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                data = self.recv.recv() => {
                    if let Some(mut data) = data {
                        if self.writer.write_all(&mut data).await.is_err() {
                            let reason = exception(anyhow!("[{}] 消息发送失败!", self.addr));

                            // 忽略掉发送失败，这里的失败只用考虑 peer 已经退出了，
                            // peer 退出会释放掉 WriteFuture，这里就不用处理 break 了。
                            let _ = self.peer_sender.send(Exit{ id: self.id, reason }.into()).await;
                        }
                    } else {
                        trace!("{} 的消息发送通道已关闭！", self.addr);
                        break;
                    }
                }
            }
        }
    }
}

pub struct ReadFuture<T> {
    pub(super) id: Id,
    pub(super) reader: OwnedReadHalfExt,
    pub(super) addr: SocketAddr,
    pub(super) peer_sender: Sender<TransferPtr>,
    pub(super) rc: T,
}

impl<T: PacketAck + Send> ReadFuture<T> {
    pub async fn run(mut self) {
        let mut bt_resp = PeerResp::new(&mut self.reader, &self.addr);
        loop {
            tokio::select! {
                result = &mut bt_resp => {
                    match result {
                        FutureRet::Ok(Normal(msg_type, buf)) => {
                            let buf_len = buf.len() as u64;
                            if msg_type == MsgType::Piece {
                                self.rc.ack((5 + buf_len) as u32);
                            }

                            // 忽略掉发送失败，这里的失败只用考虑 peer 已经退出了，
                            // peer 退出会释放掉 ReadFuture，这里就不用处理 break 了。
                            let _ = self.peer_sender.send(PeerTransfer {
                                id: self.id,
                                msg_type,
                                buf,
                                read_size: 5 + buf_len
                            }.into()).await;
                        },
                        FutureRet::Ok(Heartbeat) => {
                            let _ = self.peer_sender.send(command::Heartbeat{id: self.id}.into()).await;
                        }
                        FutureRet::Err(e) => {
                            let reason =
                                if is_disconnect!(e) {
                                    trace!("断开了链接，终止 {} - {} 的数据监听", self.id, self.addr);
                                    PeerExitReason::ClientExit
                                } else {
                                    exception(anyhow!("{} - {} 的数据监听出错: {}", self.id, self.addr, e))
                                };
                            let _ = self.peer_sender.send(Exit{ id: self.id, reason }.into()).await;
                            break;
                        }
                    }
                    bt_resp = PeerResp::new(&mut self.reader, &self.addr);
                }
            }
        }

        debug!("recv [{}] 已退出！", self.id);
    }
}
