use crate::tracker::udp_tracker::error::SocketError::*;
use core::fmt::Display;
use std::error::Error;
use crate::tracker;

pub type Result<T> = std::result::Result<T, SocketError>;

#[derive(Debug)]
pub enum SocketError {
    /// 连接超时
    Timeout,

    /// 传输 ID 不匹配
    TransactionIdMismatching(u32, u32),

    /// IO 错误
    IoError(std::io::Error),

    /// 响应长度错误
    ResponseLengthError(usize),

    PeerHostError(tracker::PeerHostError)
}

impl Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Timeout => write!(f, "连接超时"),
            TransactionIdMismatching(req, resp) => write!(
                f,
                "传输 ID 不匹配，req tran id: {req}, resp tran id: {resp}"
            ),
            IoError(e) => write!(f, "IO 错误: {}", e),
            ResponseLengthError(len) => write!(f, "响应长度错误: {}", len),
            PeerHostError(e) => write!(f, "PeerHostError: {}", e),
        }
    }
}

impl Error for SocketError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            IoError(e) => Some(e),
            PeerHostError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SocketError {
    fn from(e: std::io::Error) -> Self {
        IoError(e)
    }
}

impl From<tracker::PeerHostError> for SocketError {
    fn from(e: tracker::PeerHostError) -> Self {
        PeerHostError(e)
    }
}
