use crate::{bencoding, tracker};
use Error::*;
use core::fmt::Display;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    RequestError(reqwest::Error),
    ResponseStatusNotOk(reqwest::StatusCode, String),
    BencodingError(bencoding::error::Error),
    MissingField(&'static str),
    FieldValueError(String),
    TrackerError(tracker::error::Error),
    SerdeQsError(serde_qs::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            RequestError(e) => write!(f, "request error: {}", e),
            ResponseStatusNotOk(status, msg) => {
                write!(f, "response status not ok. code: {}\tmgs: {}", status, msg)
            }
            BencodingError(e) => write!(f, "bencoding error: {}", e),
            MissingField(field) => write!(f, "missing field: {}", field),
            FieldValueError(msg) => {
                write!(f, "field value error: {}", msg)
            }
            TrackerError(e) => write!(f, "tracker error: {}", e),
            SerdeQsError(e) => write!(f, "serde qs error: {}", e),
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        RequestError(e)
    }
}

impl From<bencoding::error::Error> for Error {
    fn from(e: bencoding::error::Error) -> Self {
        BencodingError(e)
    }
}

impl From<tracker::error::Error> for Error {
    fn from(e: tracker::error::Error) -> Self {
        TrackerError(e)
    }
}

impl From<serde_qs::Error> for Error {
    fn from(e: serde_qs::Error) -> Self {
        SerdeQsError(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RequestError(e) => Some(e),
            BencodingError(e) => Some(e),
            TrackerError(e) => Some(e),
            SerdeQsError(e) => Some(e),
            _ => None,
        }
    }
}
