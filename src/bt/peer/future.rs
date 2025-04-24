use crate::bytes::Bytes2Int;
use crate::net::ReaderHandle;
use crate::peer::MsgType;
use bytes::Bytes;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::net::tcp::OwnedReadHalf;
use tracing::{error, trace};

enum State {
    Length,   // 等待长度数据
    MsgType,  // 等待消息类型
    Content,  // 等待内容数据
    Finished, // Future 已完成
}

/// bt 响应处理
pub struct BtResp<'a> {
    reader_handle: ReaderHandle<'a, OwnedReadHalf>,
    state: State,
    msg_type: Option<MsgType>,
    length: Option<u32>,
}

impl<'a> BtResp<'a> {
    pub fn new(read: &'a mut OwnedReadHalf, addr: SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(read, addr, 4),
            state: State::Length,
            msg_type: None,
            length: None,
        }
    }
}

impl Future for BtResp<'_> {
    type Output = Option<(MsgType, Bytes)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let buf = match pin!(&mut this.reader_handle).poll(cx) {
            Poll::Ready(Ok(buf)) => buf,
            Poll::Ready(Err(_e)) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        };

        match this.state {
            State::Length => {
                let length = u32::from_be_slice(&buf[..4]);
                if length > 1 {
                    this.length = Some(length - 1);
                }
                this.reader_handle.reset(1);
                this.state = State::MsgType;
            }
            State::MsgType => {
                if let Ok(msg_type) = MsgType::try_from(buf[0]) {
                    trace!("取得消息类型: {:?}", msg_type);
                    if let Some(length) = this.length {
                        this.reader_handle.reset(length as usize);
                        this.msg_type = Some(msg_type);
                        this.state = State::Content;
                    } else {
                        return Poll::Ready(Some((msg_type, Bytes::new())));
                    }
                } else {
                    error!("未知的消息类型\tmsg_type value: {}", buf[0]);
                    return Poll::Ready(None);
                }
            }
            State::Content => {
                this.state = State::Finished;
                return Poll::Ready(Some((this.msg_type.take().unwrap(), buf)));
            }
            State::Finished => {
                return Poll::Ready(None);
            }
        }
        pin!(this).poll(cx)
    }
}
