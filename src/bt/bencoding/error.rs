use Error::*;
use core::fmt::{Display, Formatter};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(PartialEq, Eq, Debug)]
pub enum Error {
    InvalidByte(usize),
    UnexpectedEndOfStream,
    InvalidUtf8,
    TransformError,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            InvalidByte(pos) => write!(f, "invalid byte at position {}", pos),
            UnexpectedEndOfStream => write!(f, "unexpected end of stream"),
            InvalidUtf8 => write!(f, "invalid utf-8"),
            TransformError => write!(f, "transform error"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}
