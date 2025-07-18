use anyhow::{Result, anyhow};
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use dashmap::DashMap;
use dorodoro_bangumi::bytes_util::Bytes2Int;
use dorodoro_bangumi::control::{CODE_SIZE, LENGTH_SIZE, STATUS_SIZE, Status, TRAN_ID_SIZE, TranId, ControlStatus};
use dorodoro_bangumi::net::{FutureRet, ReaderHandle};
use dorodoro_bangumi::protocol::remote_control::{PASSWORD_LEN_SIZE, USERNAME_LEN_SIZE};
use dorodoro_bangumi::protocol::{PROTOCOL_SIZE, REMOTE_CONTROL_PROTOCOL};
use dorodoro_bangumi::router::Code;
use dorodoro_bangumi::{is_disconnect, pin_poll};
use serde::Serialize;
use std::io;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll, Waker};
use tokio::io::{AsyncRead, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub struct Auth {
    pub username: String,
    pub password: String,
}

/// 循环 id
struct CycleId {
    val: AtomicU32,
}

impl CycleId {
    fn new() -> Self {
        Self {
            val: AtomicU32::new(0),
        }
    }

    fn next_tran_id(&self) -> u32 {
        self.val.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Debug)]
pub struct Ret {
    pub code: Code,
    pub status: Status,
    pub body: Bytes,
}

pub struct Client {
    /// 写入器端口
    write: OwnedWriteHalf,

    /// 循环 id
    cycle_id: Arc<CycleId>,

    /// 正在等待响应的结果
    inflight: Arc<DashMap<u32, Option<Waker>>>,

    /// 响应结果
    result_store: Arc<DashMap<u32, (Code, Status, Bytes)>>,

    /// 取消 token
    cancel_token: CancellationToken,
}

impl Client {
    pub async fn request<T: Serialize>(&mut self, code: Code, body: T) -> Result<ResponseFuture> {
        let tran_id = self.cycle_id.next_tran_id();
        if self.inflight.contains_key(&tran_id) {
            return Err(anyhow!("重复的请求"));
        }

        let mut buf = Vec::with_capacity(CODE_SIZE + TRAN_ID_SIZE + LENGTH_SIZE);
        let data = serde_json::to_vec(&body)?;
        WriteBytesExt::write_u32::<BigEndian>(&mut buf, code)?;
        WriteBytesExt::write_u32::<BigEndian>(&mut buf, tran_id)?;
        WriteBytesExt::write_u32::<BigEndian>(&mut buf, data.len() as u32)?;
        buf.extend_from_slice(&data);

        self.inflight.insert(tran_id, None);
        self.write.write_all(&buf).await?;

        Ok(ResponseFuture::new(
            tran_id,
            self.inflight.clone(),
            self.result_store.clone(),
        ))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

pub struct ClientHandle {
    /// 读取器端口
    read: OwnedReadHalf,

    /// 正在等待响应的结果
    inflight: Arc<DashMap<u32, Option<Waker>>>,

    /// 响应结果
    result_store: Arc<DashMap<u32, (Code, Status, Bytes)>>,

    /// 取消 token
    cancel_token: CancellationToken,
}

impl ClientHandle {
    pub async fn new(addr: SocketAddr, auth: Auth) -> Result<Client> {
        let mut stream = TcpStream::connect(addr).await?;
        Self::handshake(&mut stream, &auth).await?;

        let (read, write) = stream.into_split();
        let cycle_id = Arc::new(CycleId::new());
        let inflight = Arc::new(DashMap::new());
        let result_store = Arc::new(DashMap::new());
        let cancel_token = CancellationToken::new();

        let client_handle = ClientHandle {
            read,
            inflight: inflight.clone(),
            result_store: result_store.clone(),
            cancel_token: cancel_token.clone(),
        };

        tokio::spawn(client_handle.run());

        Ok(Client {
            write,
            cycle_id,
            inflight,
            result_store,
            cancel_token,
        })
    }

    async fn handshake(stream: &mut TcpStream, auth: &Auth) -> Result<()> {
        let username = auth.username.as_bytes();
        let password = auth.password.as_bytes();
        let mut buf = Vec::with_capacity(
            PROTOCOL_SIZE
                + REMOTE_CONTROL_PROTOCOL.len()
                + USERNAME_LEN_SIZE
                + username.len()
                + PASSWORD_LEN_SIZE
                + password.len(),
        );

        WriteBytesExt::write_u8(&mut buf, REMOTE_CONTROL_PROTOCOL.len() as u8)?;
        buf.extend_from_slice(REMOTE_CONTROL_PROTOCOL);
        WriteBytesExt::write_u16::<BigEndian>(&mut buf, username.len() as u16)?;
        buf.extend_from_slice(username);
        WriteBytesExt::write_u16::<BigEndian>(&mut buf, password.len() as u16)?;
        buf.extend_from_slice(password);
        stream.write_all(&buf).await?;

        let addr = stream.peer_addr()?;
        if let FutureRet::Err(e) = HandshakeParse::new(stream, &addr).await {
            return Err(e.into());
        }

        Ok(())
    }

    pub async fn run(mut self) {
        let addr = &self.read.peer_addr().unwrap();
        let mut response_pares = ResponseParse::new(&mut self.read, addr);
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                result = &mut response_pares => {
                    match result {
                        FutureRet::Ok((code, tran_id, status, data)) => {
                            if let Some((_, resp)) = self.inflight.remove(&tran_id) {
                                self.result_store.insert(tran_id, (code, status, data.clone()));
                                resp.map(|waker| waker.wake());
                            }
                        }
                        FutureRet::Err(e) => {
                            if is_disconnect!(e) {
                                info!("控制器断开链接");
                            } else {
                                error!("控制器响应解析错误: {}", e)
                            }
                            break;
                        }
                    }
                    response_pares = ResponseParse::new(&mut self.read, addr);
                }
            }
        }
    }
}

enum ResponseParseState {
    /// 等待 code + tran_id + status + length 头部
    Head,

    /// 等待 content 内容
    Content,

    /// 解析完成
    Finished,
}

pub struct ResponseParse<'a, T: AsyncRead + Unpin> {
    reader_handle: ReaderHandle<'a, T>,
    state: ResponseParseState,
    code: Option<Code>,
    tran_id: Option<TranId>,
    status: Option<Status>,
}

impl<'a, T: AsyncRead + Unpin> ResponseParse<'a, T> {
    pub fn new(read: &'a mut T, addr: &'a SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(
                read,
                addr,
                CODE_SIZE + TRAN_ID_SIZE + STATUS_SIZE + LENGTH_SIZE,
            ),
            state: ResponseParseState::Head,
            code: None,
            tran_id: None,
            status: None,
        }
    }
}

impl<T: AsyncRead + Unpin> Future for ResponseParse<'_, T> {
    type Output = FutureRet<(Code, TranId, Status, Bytes)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let buf = pin_poll!(&mut this.reader_handle, cx);

        match this.state {
            ResponseParseState::Head => {
                let mut offset: usize = 0;
                let code = Code::from_be_slice(&buf[offset..CODE_SIZE]);
                offset += CODE_SIZE;
                let tran_id = TranId::from_be_slice(&buf[offset..offset + TRAN_ID_SIZE]);
                offset += TRAN_ID_SIZE;
                let status = Status::from_be_slice(&buf[offset..offset + STATUS_SIZE]);
                offset += STATUS_SIZE;
                let length = u32::from_be_slice(&buf[offset..offset + LENGTH_SIZE]);

                if length == 0 {
                    this.state = ResponseParseState::Finished;
                    return Poll::Ready(FutureRet::Ok((code, tran_id, status, Bytes::new())));
                }
                this.code = Some(code);
                this.tran_id = Some(tran_id);
                this.status = Some(status);
                this.reader_handle.reset(length as usize);
                this.state = ResponseParseState::Content;
            }
            ResponseParseState::Content => {
                this.state = ResponseParseState::Finished;
                return Poll::Ready(FutureRet::Ok((
                    this.code.unwrap(),
                    this.tran_id.unwrap(),
                    this.status.unwrap(),
                    buf,
                )));
            }
            ResponseParseState::Finished => {
                return Poll::Ready(FutureRet::Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "parse finished",
                )));
            }
        }
        pin!(this).poll(cx)
    }
}

pub struct ResponseFuture {
    tran_id: u32,
    inflight: Arc<DashMap<u32, Option<Waker>>>,
    result_store: Arc<DashMap<u32, (Code, Status, Bytes)>>,
}

impl ResponseFuture {
    fn new(
        tran_id: u32,
        inflight: Arc<DashMap<u32, Option<Waker>>>,
        result_store: Arc<DashMap<u32, (Code, Status, Bytes)>>,
    ) -> Self {
        Self {
            tran_id,
            inflight,
            result_store,
        }
    }
}

impl Future for ResponseFuture {
    type Output = Ret;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        match this.result_store.remove(&this.tran_id) {
            Some((_, (code, status, data))) => Poll::Ready(Ret {
                code,
                status,
                body: data,
            }),
            None => {
                this.inflight.insert(this.tran_id, Some(cx.waker().clone()));
                Poll::Pending
            }
        }
    }
}

impl Drop for ResponseFuture {
    fn drop(&mut self) {
        self.inflight.remove(&self.tran_id);
        self.result_store.remove(&self.tran_id);
    }
}

enum HandshakeState {
    ProtocolLen,
    Protocol,
    Status,
    ErrorMsgLen,
    ErrorMsg,
    Finished,
}

struct HandshakeParse<'a, T: AsyncRead + Unpin> {
    reader_handle: ReaderHandle<'a, T>,
    state: HandshakeState,
}

impl<'a, T: AsyncRead + Unpin> HandshakeParse<'a, T> {
    pub fn new(read: &'a mut T, addr: &'a SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(read, addr, PROTOCOL_SIZE),
            state: HandshakeState::ProtocolLen,
        }
    }
}

impl<T: AsyncRead + Unpin> Future for HandshakeParse<'_, T> {
    type Output = FutureRet<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let buf: Bytes = pin_poll!(&mut this.reader_handle, cx);

        match this.state {
            HandshakeState::ProtocolLen => {
                let protocol_len = buf[0] as usize;
                this.reader_handle.reset(protocol_len);
                this.state = HandshakeState::Protocol;
            }
            HandshakeState::Protocol => {
                let protocol = buf.as_ref();
                if protocol != REMOTE_CONTROL_PROTOCOL {
                    return Poll::Ready(FutureRet::Err(io::Error::new(
                        ErrorKind::InvalidData,
                        "协议错误"
                    )));
                }
                this.reader_handle.reset(STATUS_SIZE);
                this.state = HandshakeState::Status;
            }
            HandshakeState::Status => {
                let status = Status::from_be_slice(&buf);
                if status == ControlStatus::Ok as u32 {
                    return Poll::Ready(FutureRet::Ok(()))
                }
                this.reader_handle.reset(LENGTH_SIZE);
                this.state = HandshakeState::ErrorMsgLen;
            }
            HandshakeState::ErrorMsgLen => {
                let error_msg_len = u32::from_be_slice(&buf);
                this.reader_handle.reset(error_msg_len as usize);
                this.state = HandshakeState::ErrorMsg;
            }
            HandshakeState::ErrorMsg => {
                this.state = HandshakeState::Finished;
                return match String::from_utf8(buf.to_vec()) {
                    Ok(error_msg) => {
                        Poll::Ready(FutureRet::Err(io::Error::new(
                            ErrorKind::PermissionDenied,
                            error_msg
                        )))
                    }
                    Err(_) => {
                        Poll::Ready(FutureRet::Err(io::Error::new(
                            ErrorKind::InvalidData,
                            "错误信息格式错误"
                        )))
                    }
                }
            }
            HandshakeState::Finished => {
                return Poll::Ready(FutureRet::Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "parse finished",
                )));
            }
        }

        pin!(this).poll(cx)
    }
}