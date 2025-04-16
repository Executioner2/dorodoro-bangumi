use crate::tracker;
use Error::*;
use core::fmt::Display;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// 连接超时
    Timeout,

    /// 传输 ID 不匹配
    TransactionIdMismatching(u32, u32),

    /// IO 错误
    IoError(std::io::Error),

    /// 响应长度错误
    ResponseLengthError(usize),

    /// Tracker 错误
    TrackerError(tracker::error::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Timeout => write!(f, "connection timeout"),
            TransactionIdMismatching(req, resp) => write!(
                f,
                "传输 ID 不匹配，req tran id: {req}, resp tran id: {resp}"
            ),
            IoError(e) => write!(f, "io error: {}", e),
            ResponseLengthError(len) => write!(f, "response length error: {}", len),
            TrackerError(e) => write!(f, "tracker error: {}", e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        IoError(e)
    }
}

impl From<tracker::error::Error> for Error {
    fn from(e: tracker::error::Error) -> Self {
        TrackerError(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IoError(e) => Some(e),
            TrackerError(e) => Some(e),
            _ => None,
        }
    }
}
