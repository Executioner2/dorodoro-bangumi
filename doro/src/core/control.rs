//! 接收客户端发来的控制信号

mod command;
mod request_parse;

use crate::command::CommandHandler;
use crate::core::context::Context;
use crate::core::control::command::{Command, Response};
use crate::core::emitter::constant::CONTROLLER_PREFIX;
use crate::core::emitter::Emitter;
use crate::emitter::transfer::TransferPtr;
use crate::router::Code;
use crate::runtime::{CommandHandleResult, CustomTaskResult, ExitReason, Runnable};
use anyhow::Result;
use bytes::Bytes;
use futures::stream::FuturesUnordered;
use std::pin::Pin;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio_util::sync::WaitForCancellationFuture;
use tracing::info;
use doro_util::global::Id;
use doro_util::is_disconnect;
use crate::core::control::request_parse::RequestParse;
use doro_util::net::FutureRet;
use crate::router;

/// code 字段占用字节数
pub const CODE_SIZE: usize = 4;

/// tran_id 字段占用字节数
pub const TRAN_ID_SIZE: usize = 4;

/// length 字段占用字节数
pub const LENGTH_SIZE: usize = 4;

/// 状态码占用字节数
pub const STATUS_SIZE: usize = 4;

pub type TranId = u32;

pub type Status = u32;

pub enum ControlStatus {
    /// 成功
    Ok = 200,

    /// 认证失败，未认证
    AuthFailed = 401,

    /// 服务器错误
    ServerError = 500,
}

enum ControlResponsePacket {
    /// 成功的响应
    Ok(Code, TranId, Vec<u8>),

    /// 错误的响应
    Error(Code, TranId, ControlStatus, String),
}

pub struct Dispatcher {
    /// 控制器id
    id: Id,

    /// socket 读取
    socket_read: Option<OwnedReadHalf>,
    
    /// socket 写入
    socket_write: OwnedWriteHalf,

    /// 指令发射器
    emitter: Emitter,
}

impl Dispatcher {
    pub fn new(id: Id, socket: TcpStream, emitter: Emitter) -> Self {
        let (read, write) = socket.into_split();
        Self {
            id,
            socket_read: Some(read),
            socket_write: write,
            emitter,
        }
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        self.socket_write.write_all(data).await.map_err(|e| e.into())
    }

    fn dispatch(&self, code: Code, tran_id: TranId, data: Option<Bytes>) {
        let send = self
            .emitter
            .get(&Self::get_transfer_id(self.get_suffix()))
            .unwrap();
        tokio::spawn(async move {
            let data = data.as_ref().map(|d| d.as_ref());
            let crp = {
                match router::handle_request(code, data).await {
                    Ok(data) => ControlResponsePacket::Ok(code, tran_id, data),
                    Err(e) => ControlResponsePacket::Error(
                        code,
                        tran_id,
                        ControlStatus::ServerError,
                        e.to_string(),
                    ),
                }
            };
            let packet = Dispatcher::pack_response(crp);
            let response = Response { data: packet };
            send.send(response.into()).await.unwrap();
        });
    }

    fn pack_response(crp: ControlResponsePacket) -> Vec<u8> {
        match crp {
            ControlResponsePacket::Ok(code, tran_id, data) => {
                let len = data.len();
                let mut packet = Vec::with_capacity(CODE_SIZE + TRAN_ID_SIZE + STATUS_SIZE + LENGTH_SIZE + len);
                packet.extend_from_slice(&code.to_be_bytes());
                packet.extend_from_slice(&tran_id.to_be_bytes());
                packet.extend_from_slice(&(ControlStatus::Ok as u32).to_be_bytes());
                packet.extend_from_slice(&(len as u32).to_be_bytes());
                packet.extend(data);
                packet
            }
            ControlResponsePacket::Error(code, tran_id, control_status, message) => {
                let mut packet =
                    Vec::with_capacity(CODE_SIZE + TRAN_ID_SIZE + STATUS_SIZE + LENGTH_SIZE + message.len());
                packet.extend_from_slice(&code.to_be_bytes());
                packet.extend_from_slice(&tran_id.to_be_bytes());
                packet.extend_from_slice(&(control_status as u32).to_be_bytes());
                packet.extend_from_slice(&(message.len() as u32).to_be_bytes());
                packet.extend_from_slice(message.as_bytes());
                packet
            }
        }
    }

    fn register_read_future(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>> {
        let mut socket_read = self.socket_read.take().unwrap();
        let cancel_token = Context::global().cancel_token();
        let send = self.emitter.get(&Self::get_transfer_id(self.get_suffix())).unwrap();
        Box::pin(async move {
            let addr = socket_read.peer_addr().unwrap();
            let mut request_parse = RequestParse::new(&mut socket_read, &addr);
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        return CustomTaskResult::Exit(ExitReason::Normal);
                    }
                    result = &mut request_parse => {
                        match result {
                            FutureRet::Ok((code, tran_id, data)) => {
                                let cmd = command::Request { code, tran_id, data };
                                send.send(cmd.into()).await.unwrap();
                            }
                            FutureRet::Err(e) => {
                                if is_disconnect!(e) {
                                    info!("控制器断开链接");
                                    return CustomTaskResult::Exit(ExitReason::Normal);
                                }
                                return CustomTaskResult::Exit(ExitReason::Error(e.into()));
                            }
                        }
                        request_parse = RequestParse::new(&mut socket_read, &addr);
                    }
                }
            }
        })
    }
}

impl Runnable for Dispatcher {
    fn emitter(&self) -> &Emitter {
        &self.emitter
    }

    fn get_transfer_id<T: ToString>(suffix: T) -> String {
        format!("{}{}", CONTROLLER_PREFIX, suffix.to_string())
    }

    fn get_suffix(&self) -> String {
        self.id.to_string()
    }

    fn register_lt_future(
        &mut self,
    ) -> FuturesUnordered<Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>>> {
        let futures = FuturesUnordered::new();
        futures.push(self.register_read_future());
        futures
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        Context::global().cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }
}
