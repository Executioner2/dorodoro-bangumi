//! 解析 remote control 握手协议后半部分

use crate::bytes_util::Bytes2Int;
use crate::net::{FutureRet, ReaderHandle};
use bytes::Bytes;
use std::io;
use std::io::ErrorKind;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::io::AsyncRead;
use crate::pin_poll;

/// 用户名长度的字节数
const USERNAME_LEN_SIZE: usize = 2;

/// 密码长度的字节数
const PASSWORD_LEN_SIZE: usize = 2;

#[allow(dead_code)]
enum State {
    UsernameLen,
    Username,
    PasswordLen,
    Password,
    Finished,
}

struct HandshakeParse<'a, T: AsyncRead + Unpin> {
    reader_handle: ReaderHandle<'a, T>,
    state: State,
    username: Option<Bytes>,
    password: Option<Bytes>,
}

impl<'a, T: AsyncRead + Unpin> HandshakeParse<'a, T> {
    #[allow(dead_code)] // todo - 待使用
    pub fn new(reader_handle: ReaderHandle<'a, T>) -> Self {
        Self {
            reader_handle,
            state: State::UsernameLen,
            username: None,
            password: None,
        }
    }
}

impl<'a, T: AsyncRead + Unpin> Future for HandshakeParse<'a, T> {
    type Output = FutureRet<(Bytes, Bytes)>;

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
                this.state = State::PasswordLen;
            }
            State::PasswordLen => {
                let password_len = u16::from_be_slice(&buf[..PASSWORD_LEN_SIZE]);
                this.reader_handle.reset(password_len as usize);
                this.state = State::Password;
            }
            State::Password => {
                this.password = Some(buf);
                this.state = State::Finished;
                return Poll::Ready(FutureRet::Ok((
                    this.username.take().unwrap(),
                    this.password.take().unwrap(),
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
