use std::io;
use std::io::ErrorKind;
use crate::bytes_util::Bytes2Int;
use crate::core::control::{CODE_SIZE, LENGTH_SIZE};
use crate::net::ReaderHandle;
use crate::router::Code;
use bytes::Bytes;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::io::AsyncRead;

pub enum Request {
    /// 正常请求
    Noraml(Code, Option<Bytes>),

    /// 处理请求错误
    Err(io::Error),
}

enum State {
    /// 等待 code + length 头部
    Head,

    /// 等待 content 内容
    Content,

    /// 解析完成
    Finished,
}

pub struct RequestParse<'a, T: AsyncRead + Unpin> {
    reader_handle: ReaderHandle<'a, T>,
    state: State,
    code: Option<Code>,
}

impl<'a, T: AsyncRead + Unpin> RequestParse<'a, T> {
    pub fn new(read: &'a mut T, addr: &'a SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(read, addr, CODE_SIZE + LENGTH_SIZE),
            state: State::Head,
            code: None,
        }
    }
}

impl<T: AsyncRead + Unpin> Future for RequestParse<'_, T> {
    type Output = Request;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let buf: Bytes = match pin!(&mut this.reader_handle).poll(cx) {
            Poll::Ready(Ok(buf)) => buf,
            Poll::Ready(Err(e)) => return Poll::Ready(Request::Err(e.into())),
            Poll::Pending => return Poll::Pending,
        };

        match this.state {
            State::Head => {
                let code = Code::from_be_slice(&buf[..CODE_SIZE]);
                let length = u32::from_be_slice(&buf[CODE_SIZE..CODE_SIZE + LENGTH_SIZE]);
                if length == 0 {
                    this.state = State::Finished;
                    return Poll::Ready(Request::Noraml(code, None))
                }
                this.code = Some(code);
                this.reader_handle.reset(length as usize);
                this.state = State::Content;
            }
            State::Content => {
                this.state = State::Finished;
                return Poll::Ready(Request::Noraml(this.code.unwrap(), Some(buf)))
            }
            State::Finished => return Poll::Ready(Request::Err(io::Error::new(ErrorKind::PermissionDenied, "parse finished"))),
        }
        pin!(this).poll(cx)
    }
}
