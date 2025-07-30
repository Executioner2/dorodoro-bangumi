//! remote control 协议相关
//!
//! 握手请求组成：
//!
//!     <protocol_len:u8><protocol_name:bytes><username_len:u16><username:bytes><password_len:u16><password:bytes>
//!
//! 握手响应组成：
//!
//!     <protocol_len:u8><protocol_name:bytes><status:u32>[length:u32][error_msg:bytes]
//!

use std::io;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};

use anyhow::{Result, anyhow};
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use doro_util::bytes_util::Bytes2Int;
use doro_util::net::{FutureRet, ReaderHandle};
use doro_util::pin_poll;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::config::ClientAuth;
use crate::control::{ControlStatus, LENGTH_SIZE, STATUS_SIZE};
use crate::protocol::{PROTOCOL_SIZE, REMOTE_CONTROL_PROTOCOL};

#[derive(Debug)]
pub struct AuthInfo {
    username: Bytes,
    password: Bytes,
}

/// 用户名长度的字节数
pub const USERNAME_LEN_SIZE: usize = 2;

/// 密码长度的字节数
pub const PASSWORD_LEN_SIZE: usize = 2;

enum State {
    UsernameLen,
    Username,
    PasswordLen,
    Password,
    Finished,
}

/// 解析 remote control 握手协议后半部分
pub struct HandshakeParse<'a, T: AsyncRead + Unpin> {
    reader_handle: ReaderHandle<'a, T>,
    state: State,
    username: Option<Bytes>,
}

impl<'a, T: AsyncRead + Unpin> HandshakeParse<'a, T> {
    pub fn new(reader: &'a mut T, addr: &'a SocketAddr) -> Self {
        Self {
            reader_handle: ReaderHandle::new(reader, addr, USERNAME_LEN_SIZE),
            state: State::UsernameLen,
            username: None,
        }
    }
}

impl<'a, T: AsyncRead + Unpin> Future for HandshakeParse<'a, T> {
    type Output = FutureRet<AuthInfo>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let buf = pin_poll!(&mut this.reader_handle, cx);

        match this.state {
            State::UsernameLen => {
                let username_len = u16::from_be_slice(&buf[..USERNAME_LEN_SIZE]);
                this.reader_handle.reset(username_len as usize);
                this.state = State::Username;
            }
            State::Username => {
                this.username = Some(buf);
                this.reader_handle.reset(PASSWORD_LEN_SIZE);
                this.state = State::PasswordLen;
            }
            State::PasswordLen => {
                let password_len = u16::from_be_slice(&buf[..PASSWORD_LEN_SIZE]);
                this.reader_handle.reset(password_len as usize);
                this.state = State::Password;
            }
            State::Password => {
                this.state = State::Finished;
                return Poll::Ready(FutureRet::Ok(AuthInfo {
                    username: this.username.take().unwrap(),
                    password: buf,
                }));
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

pub fn auth_verify(client_auth: &ClientAuth, auth_info: &AuthInfo) -> Result<()> {
    let username =
        String::from_utf8(auth_info.username.to_vec()).map_err(|_| anyhow!("username invalid"))?;
    let password =
        String::from_utf8(auth_info.password.to_vec()).map_err(|_| anyhow!("password invalid"))?;

    if client_auth.username != username || client_auth.password != password {
        return Err(anyhow!("auth failed"));
    }

    Ok(())
}

pub async fn send_handshake_success<T: AsyncWrite + Unpin>(send: &mut T) -> Result<()> {
    let mut buf = Vec::with_capacity(PROTOCOL_SIZE + REMOTE_CONTROL_PROTOCOL.len() + STATUS_SIZE);
    WriteBytesExt::write_u8(&mut buf, REMOTE_CONTROL_PROTOCOL.len() as u8)?;
    buf.extend_from_slice(REMOTE_CONTROL_PROTOCOL);
    WriteBytesExt::write_u32::<BigEndian>(&mut buf, ControlStatus::Ok as u32)?;
    send.write_all(&buf).await?;
    Ok(())
}

pub async fn send_handshake_failed<T: AsyncWrite + Unpin>(
    send: &mut T, status: ControlStatus, error_msg: String,
) -> Result<()> {
    let mut buf = Vec::with_capacity(
        PROTOCOL_SIZE + REMOTE_CONTROL_PROTOCOL.len() + STATUS_SIZE + LENGTH_SIZE + error_msg.len(),
    );
    let error_msg = error_msg.as_bytes();
    WriteBytesExt::write_u8(&mut buf, REMOTE_CONTROL_PROTOCOL.len() as u8)?;
    buf.extend_from_slice(REMOTE_CONTROL_PROTOCOL);
    WriteBytesExt::write_u32::<BigEndian>(&mut buf, status as u32)?;
    WriteBytesExt::write_u32::<BigEndian>(&mut buf, error_msg.len() as u32)?;
    buf.extend_from_slice(error_msg);
    send.write_all(&buf).await?;
    Ok(())
}
