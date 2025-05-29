use crate::emitter::transfer::TransferPtr;
use crate::peer::MsgType;
use crate::peer::command::{Exit, PeerTransfer};
use crate::peer::future::BtResp;
use crate::peer::rate_control::PacketAck;
use crate::peer_manager::gasket::ExitReason;
use crate::runtime::Runnable;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

pub struct WriteFuture {
    pub(super) no: u64,
    pub(super) writer: OwnedWriteHalf,
    pub(super) cancel_token: CancellationToken,
    pub(super) addr: Arc<SocketAddr>,
    pub(super) peer_sender: Sender<TransferPtr>,
    pub(super) recv: Receiver<Vec<u8>>,
}

impl Runnable for WriteFuture {
    async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    debug!("send [{}]\t addr: [{}] 退出", self.no, self.addr);
                    break;
                }
                data = self.recv.recv() => {
                    if let Some(data) = data {
                        if self.writer.write_all(&data).await.is_err() {
                            error!("[{}] 消息发送失败!", self.no);
                            let reason = ExitReason::Exception;
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
    pub(super) reader: OwnedReadHalf,
    pub(super) cancel_token: CancellationToken,
    pub(super) addr: Arc<SocketAddr>,
    pub(super) peer_sender: Sender<TransferPtr>,
    pub(super) rc: T,
}

impl<T: PacketAck + Send> Runnable for ReadFuture<T> {
    async fn run(mut self) {
        let mut bt_resp = BtResp::new(&mut self.reader, &self.addr);
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    debug!("recv [{}] 退出", self.no);
                    break;
                }
                result = &mut bt_resp => {
                    match result {
                        Some((msg_type, buf)) => {
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
                        None => {
                            warn!("断开了链接，终止 {} - {} 的数据监听", self.no, self.addr);
                            let reason = ExitReason::Exception;
                            self.peer_sender.send(Exit{ reason }.into()).await.unwrap();
                            break;
                        }
                    }
                    bt_resp = BtResp::new(&mut self.reader, &self.addr);
                }
            }
        }

        debug!("recv [{}] 已退出！", self.no);
    }
}
