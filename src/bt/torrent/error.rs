use crate::bt::bencoding;
use Error::*;
use core::fmt::Display;

pub type Result<T> = std::result::Result<T, Error>;

/// 错误类型
#[derive(Debug)]
pub enum Error {
    InvalidTorrent(&'static str),
    BencodingError(bencoding::error::Error),
    Utf8Error(std::string::FromUtf8Error),
    FileError(std::io::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            InvalidTorrent(msg) => write!(f, "invalid torrent file: {}", msg),
            BencodingError(e) => write!(f, "bencoding error: {}", e),
            Utf8Error(e) => write!(f, "utf8 error: {}", e),
            FileError(e) => write!(f, "file error: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BencodingError(e) => Some(e),
            Utf8Error(e) => Some(e),
            FileError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<bencoding::error::Error> for Error {
    fn from(e: bencoding::error::Error) -> Self {
        BencodingError(e)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(e: std::string::FromUtf8Error) -> Self {
        Utf8Error(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        FileError(e)
    }
}
