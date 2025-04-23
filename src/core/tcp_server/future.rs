use crate::net::ReaderHandle;
use crate::protocol;
use crate::protocol::{Identifier, Protocol};
use bytes::Bytes;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tracing::warn;

pub struct Accept<'a> {
    reader_handle: ReaderHandle<'a, TcpStream>,
    protocol: Option<Bytes>,
    protocol_id: Option<Identifier>,
    state: State,
}

#[derive(Eq, PartialEq)]
enum State {
    ProtocolLen,
    Protocol,
    ParseProtocol,
    Finished,
}

impl<'a> Accept<'a> {
    pub fn new(socket: &'a mut TcpStream) -> Self {
        let addr = socket.peer_addr().unwrap();
        Self {
            reader_handle: ReaderHandle::new(socket, addr, 1),
            protocol: None,
            protocol_id: None,
            state: State::ProtocolLen,
        }
    }

    /// 解析出协议和协议载荷长度
    fn parse_protocol(&self, protocol: &[u8]) -> Option<(Identifier, usize)> {
        match protocol {
            protocol::BIT_TORRENT_PROTOCOL => {
                Some((Identifier::BitTorrent, protocol::BIT_TORRENT_PAYLOAD_LEN))
            }
            protocol::REMOTE_CONTROL_PROTOCOL => Some((
                Identifier::RemoteControl,
                protocol::REMOTE_CONTROL_PAYLOAD_LEN,
            )),
            _ => None,
        }
    }
}

impl<'a> Future for Accept<'_> {
    type Output = Option<Protocol>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let accept = unsafe { self.get_unchecked_mut() };

        let buf = match pin!(&mut accept.reader_handle).poll(cx) {
            Poll::Ready(Ok(buf)) => buf,
            Poll::Ready(Err(_e)) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        };

        match accept.state {
            State::ProtocolLen => {
                accept.reader_handle.reset(buf[0] as usize);
                accept.state = State::Protocol;
            }
            State::Protocol => {
                accept.protocol = Some(buf);
                let protocol = accept.protocol.as_ref().unwrap();
                accept.state = State::ParseProtocol;
                if let Some((id, size)) = accept.parse_protocol(protocol) {
                    accept.reader_handle.reset(size);
                    accept.protocol_id = Some(id);
                } else {
                    warn!("未知协议: {}", String::from_utf8_lossy(protocol));
                    return Poll::Ready(None);
                }
            }
            State::ParseProtocol => {
                let id = accept.protocol_id.take().unwrap();
                let protocol = Protocol { id, payload: buf };
                accept.state = State::Finished;
                return Poll::Ready(Some(protocol));
            }
            State::Finished => {
                return Poll::Ready(None);
            }
        }
        pin!(accept).poll(cx)
    }
}
