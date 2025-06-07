use crate::store;

pub type Result<T> = std::result::Result<T, Error>;

/// 错误类型
#[derive(Debug)]
pub enum Error {
    IoError(std::io::Error),
    ConnectionError,
    TimeoutError,
    HandshakeError,
    TryFromError,
    HandleError(String),
    ResponseDataIncomplete,
    BitfieldError,
    ResponsePieceError,
    PieceCheckoutError(u32),
    PieceWriteError(u32, u32),
    StoreError(store::error::Error),
    SendError(tokio::sync::mpsc::error::SendError<Vec<u8>>)
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::IoError(e) => write!(f, "IO error: {}", e),
            Error::ConnectionError => write!(f, "connection error"),
            Error::TimeoutError => write!(f, "timeout error"),
            Error::HandshakeError => write!(f, "handshake error"),
            Error::TryFromError => write!(f, "try from error"),
            Error::HandleError(s) => write!(f, "handle error: {}", s),
            Error::ResponseDataIncomplete => write!(f, "response data incomplete - 响应数据不完整"),
            Error::BitfieldError => write!(f, "bitfield 有问题"),
            Error::ResponsePieceError => write!(f, "response piece error - 响应分块错误"),
            Error::PieceCheckoutError(index) => write!(f, "第 {} 个分块校验有问题", index),
            Error::PieceWriteError(index, offset) => write!(f, "piece: {}\toffset: {} 写入失败", index, offset),
            Error::StoreError(e) => write!(f, "store error: {}", e),
            Error::SendError(e) => write!(f, "send error: {}", e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<store::error::Error> for Error {
    fn from(e: store::error::Error) -> Self {
        Error::StoreError(e)
    }
}

impl From<tokio::sync::mpsc::error::SendError<Vec<u8>>> for Error {
    fn from(e: tokio::sync::mpsc::error::SendError<Vec<u8>>) -> Self {
        Error::SendError(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IoError(e) => Some(e),
            Error::StoreError(e) => Some(e),
            Error::SendError(e) => Some(e),
            _ => None,
        }
    }
}
