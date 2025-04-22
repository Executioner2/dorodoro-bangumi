use core::fmt::{Display, Formatter};
use tokio::sync::mpsc::error::SendError;
use super::transfer::TransferPtr;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(PartialEq, Eq, Debug)]
pub enum Error {
    NotFindEmitterType,
    RepeatRegister,
    SendError(SendError<TransferPtr>),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Error::NotFindEmitterType => write!(f, "not find emitter type"),
            Error::RepeatRegister  => write!(f, "repeat register"),
            Error::SendError(e) => write!(f, "send error: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::SendError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SendError<TransferPtr>> for Error {
    fn from(value: SendError<TransferPtr>) -> Self {
        Error::SendError(value)
    }
}