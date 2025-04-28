pub type Result<T> = std::result::Result<T, Error>;

/// 错误类型
#[derive(Debug)]
pub enum Error {
    IoError(std::io::Error),
    AcquireError(tokio::sync::AcquireError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::IoError(e) => write!(f, "IO error: {}", e),
            Error::AcquireError(e) => write!(f, "acquire error: {}", e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<tokio::sync::AcquireError> for Error {
    fn from(e: tokio::sync::AcquireError) -> Self {
        Error::AcquireError(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IoError(e) => Some(e),
            Error::AcquireError(e) => Some(e),
        }
    }
}
