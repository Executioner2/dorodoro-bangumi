use crate::tracker::udp_tracker::error::SocketError::*;
use core::fmt::Display;
use std::error::Error;

pub type Result<T> = std::result::Result<T, SocketError>;

#[derive(Debug)]
pub enum SocketError {
    /// 连接超时
    Timeout,
    TransactionIdMismatching(u32, u32),
    IoError(std::io::Error),
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
        }
    }
}

impl Error for SocketError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            IoError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SocketError {
    fn from(e: std::io::Error) -> Self {
        IoError(e)
    }
}
