use crate::core::control::{CODE_SIZE, LENGTH_SIZE, TRAN_ID_SIZE, TranId};
use crate::router::Code;
use bytes::Bytes;
use doro_util::bytes_util::Bytes2Int;
use doro_util::net::{FutureRet, ReaderHandle};
use doro_util::pin_poll;
use std::io;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::io::AsyncRead;

enum State {
    /// 等待 code + tran_id + length 头部
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
    tran_id: Option<TranId>,
}

impl<'a, T: AsyncRead + Unpin> RequestParse<'a, T> {
    pub fn new(read: &'a mut T, addr: &'a SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(read, addr, CODE_SIZE + TRAN_ID_SIZE + LENGTH_SIZE),
            state: State::Head,
            code: None,
            tran_id: None,
        }
    }
}

impl<T: AsyncRead + Unpin> Future for RequestParse<'_, T> {
    type Output = FutureRet<(Code, TranId, Option<Bytes>)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let buf = pin_poll!(&mut this.reader_handle, cx);

        match this.state {
            State::Head => {
                let mut offset: usize = 0;
                let code = Code::from_be_slice(&buf[offset..CODE_SIZE]);
                offset += CODE_SIZE;
                let tran_id = TranId::from_be_slice(&buf[offset..offset + TRAN_ID_SIZE]);
                offset += TRAN_ID_SIZE;
                let length = u32::from_be_slice(&buf[offset..offset + LENGTH_SIZE]);

                if length == 0 {
                    this.state = State::Finished;
                    return Poll::Ready(FutureRet::Ok((code, tran_id, None)));
                }
                this.code = Some(code);
                this.tran_id = Some(tran_id);
                this.reader_handle.reset(length as usize);
                this.state = State::Content;
            }
            State::Content => {
                this.state = State::Finished;
                return Poll::Ready(FutureRet::Ok((
                    this.code.unwrap(),
                    this.tran_id.unwrap(),
                    Some(buf),
                )));
            }
            State::Finished => {
                return Poll::Ready(FutureRet::Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "parse finished",
                )));
            }
        }
        pin!(this).poll(cx)
    }
}
