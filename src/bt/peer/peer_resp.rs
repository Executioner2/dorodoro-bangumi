use crate::bytes_util::Bytes2Int;
use crate::net::ReaderHandle;
use crate::peer::MsgType;
use bytes::Bytes;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::io::AsyncRead;
use tracing::{error, trace};
use crate::peer::peer_resp::RespType::{Heartbeat, Normal, Unoknown};

#[derive(Debug)]
pub enum RespType {
    /// 普通消息
    Normal(MsgType, Bytes),
    
    /// 心跳包
    Heartbeat,
    
    /// 未知消息
    Unoknown,
}

enum State {
    /// Length + MsgType 的长度
    Head,

    /// 等待内容数据
    Content,

    /// Future 已完成
    Finished,
}

/// peer 响应处理
pub struct PeerResp<'a, T: AsyncRead + Unpin> {
    reader_handle: ReaderHandle<'a, T>,
    state: State,
    msg_type: Option<MsgType>,
}

impl<'a, T: AsyncRead + Unpin> PeerResp<'a, T> {
    pub fn new(read: &'a mut T, addr: &'a SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(read, addr, 5),
            state: State::Head,
            msg_type: None,
        }
    }
}

impl<T: AsyncRead + Unpin> Future for PeerResp<'_, T> {
    type Output = RespType;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let buf = match pin!(&mut this.reader_handle).poll(cx) {
            Poll::Ready(Ok(buf)) => buf,
            Poll::Ready(Err(_e)) => return Poll::Ready(Unoknown),
            Poll::Pending => return Poll::Pending,
        };

        match this.state {
            State::Head => {
                if buf.len() < 5 { // 视为心跳包
                    return Poll::Ready(Heartbeat)
                }
                let length = u32::from_be_slice(&buf[..4]);
                if let Ok(msg_type) = MsgType::try_from(buf[4]) {
                    trace!("取得消息类型: {:?}", msg_type);
                    if length > 1 {
                        this.reader_handle.reset(length as usize - 1);
                        this.msg_type = Some(msg_type);
                        this.state = State::Content;
                    } else {
                        return Poll::Ready(Normal(msg_type, Bytes::new()));
                    }
                } else {
                    error!("未知的消息类型\tmsg_type value: {}", buf[0]);
                    return Poll::Ready(Unoknown);
                }
            }
            State::Content => {
                this.state = State::Finished;
                return Poll::Ready(Normal(this.msg_type.take().unwrap(), buf));
            }
            State::Finished => {
                return Poll::Ready(Unoknown);
            }
        }
        pin!(this).poll(cx)
    }
}
