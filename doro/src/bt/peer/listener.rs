use super::peer_resp::RespType::*;
use crate::bt::socket::{OwnedReadHalfExt, OwnedWriteHalfExt};
use crate::emitter::transfer::TransferPtr;
use crate::peer::command::{Exit, PeerTransfer};
use crate::peer::peer_resp::PeerResp;
use crate::peer::rate_control::PacketAck;
use crate::peer::{command, MsgType};
use crate::peer_manager::gasket::PeerExitReason;
use std::net::SocketAddr;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, trace};
use doro_util::is_disconnect;
use doro_util::net::FutureRet;

pub struct WriteFuture {
    pub(super) no: u64,
    pub(super) writer: OwnedWriteHalfExt,
    pub(super) cancel_token: CancellationToken,
    pub(super) addr: SocketAddr,
    pub(super) peer_sender: Sender<TransferPtr>,
    pub(super) recv: Receiver<Vec<u8>>,
}

impl WriteFuture {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    debug!("send [{}]\t addr: [{}] 退出", self.no, self.addr);
                    break;
                }
                data = self.recv.recv() => {
                    if let Some(mut data) = data {
                        if self.writer.write_all(&mut data).await.is_err() {
                            error!("[{}] 消息发送失败!", self.no);
                            let reason = PeerExitReason::Exception;
                            self.peer_sender.send(Exit{ reason }.into()).await.unwrap();
                        }
                    } else {
                        break;
                    }
                }
            }
        }
    }
}

pub struct ReadFuture<T: PacketAck + Send> {
    pub(super) no: u64,
    pub(super) reader: OwnedReadHalfExt,
    pub(super) cancel_token: CancellationToken,
    pub(super) addr: SocketAddr,
    pub(super) peer_sender: Sender<TransferPtr>,
    pub(super) rc: T,
}

impl<T: PacketAck + Send> ReadFuture<T> {
    pub async fn run(mut self) {
        let mut bt_resp = PeerResp::new(&mut self.reader, &self.addr);
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    debug!("recv [{}] 退出", self.no);
                    break;
                }
                result = &mut bt_resp => {
                    match result {
                        FutureRet::Ok(Normal(msg_type, buf)) => {
                            let buf_len = buf.len() as u64;
                            if msg_type == MsgType::Piece {
                                self.rc.ack((5 + buf_len) as u32);
                            }
                            self.peer_sender.send(PeerTransfer {
                                msg_type,
                                buf,
                                read_size: 5 + buf_len
                            }.into()).await.unwrap();
                        },
                        FutureRet::Ok(Heartbeat) => {
                            self.peer_sender.send(command::Heartbeat.into()).await.unwrap();
                        }
                        FutureRet::Err(e) => {
                            let reason;
                            if is_disconnect!(e) {
                                trace!("断开了链接，终止 {} - {} 的数据监听", self.no, self.addr);
                                reason = PeerExitReason::ClientExit;
                            } else {
                                error!("{} - {} 的数据监听出错: {}", self.no, self.addr, e);
                                reason = PeerExitReason::Exception;
                            }
                            self.peer_sender.send(Exit{ reason }.into()).await.unwrap();
                            break;
                        }
                    }
                    bt_resp = PeerResp::new(&mut self.reader, &self.addr);
                }
            }
        }

        debug!("recv [{}] 已退出！", self.no);
    }
}
