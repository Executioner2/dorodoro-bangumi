use std::io;
use std::io::ErrorKind;
use std::net::SocketAddr;
use doro_util::net::{FutureRet, ReaderHandle};
use crate::protocol::{Identifier, Protocol, PROTOCOL_SIZE};
use bytes::Bytes;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tracing::warn;
use doro_util::pin_poll;
use crate::protocol;

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
    pub fn new(socket: &'a mut TcpStream, addr: &'a SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(socket, addr, PROTOCOL_SIZE),
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
    type Output = FutureRet<Protocol>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        let buf = pin_poll!(&mut this.reader_handle, cx);

        match this.state {
            State::ProtocolLen => {
                this.reader_handle.reset(buf[0] as usize);
                this.state = State::Protocol;
            }
            State::Protocol => {
                this.protocol = Some(buf);
                let protocol = this.protocol.as_ref().unwrap();
                this.state = State::ParseProtocol;
                if let Some((id, size)) = this.parse_protocol(protocol) {
                    this.reader_handle.reset(size);
                    this.protocol_id = Some(id);
                } else {
                    warn!("未知协议: {}", String::from_utf8_lossy(protocol));
                    return Poll::Ready(FutureRet::Err(io::Error::new(
                        ErrorKind::InvalidData,
                       "unknown protocol",
                    )));
                }
            }
            State::ParseProtocol => {
                let id = this.protocol_id.take().unwrap();
                let protocol = Protocol { id, payload: buf };
                this.state = State::Finished;
                return Poll::Ready(FutureRet::Ok(protocol));
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
