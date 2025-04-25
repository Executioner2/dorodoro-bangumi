pub type Result<T> = std::result::Result<T, Error>;

/// 错误类型
#[derive(Debug)]
pub enum Error {
    IoError(std::io::Error),
    HandshakeError,
    TryFromError,
    HandleError(String),
    ResponseDataIncomplete,
    BitfieldError,
    PieceCheckoutError(u32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::IoError(e) => write!(f, "IO error: {}", e),
            Error::HandshakeError => write!(f, "handshake error"),
            Error::TryFromError => write!(f, "try from error"),
            Error::HandleError(s) => write!(f, "handle error: {}", s),
            Error::ResponseDataIncomplete => write!(f, "response data incomplete - 响应数据不完整"),
            Error::BitfieldError => write!(f, "bitfield 有问题"),
            Error::PieceCheckoutError(index) => write!(f, "第 {} 个分块校验有问题", index),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IoError(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IoError(e) => Some(e),
            _ => None,
        }
    }
}
