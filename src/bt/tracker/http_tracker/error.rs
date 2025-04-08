use core::fmt::Display;
use crate::bencoding::ParseError;
use crate::tracker;

pub type Result<T> = std::result::Result<T, HttpTrackerError>;

#[derive(Debug)]
pub enum HttpTrackerError {
    RequestError(reqwest::Error),
    ResponseStatusNotOk(reqwest::StatusCode, String),
    ResponseParseError(ParseError),
    MissingField(&'static str),
    FieldValueError(String),
    PeerHostError(tracker::PeerHostError),
    SerdeQsError(serde_qs::Error)
}

impl Display for HttpTrackerError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            HttpTrackerError::RequestError(e) => write!(f, "Request error: {}", e),
            HttpTrackerError::ResponseStatusNotOk(status, msg) => {
                write!(f, "Response status not ok. code: {}\tmgs: {}", status, msg)
            },
            HttpTrackerError::ResponseParseError(e) => write!(f, "Response parse error: {}", e),
            HttpTrackerError::MissingField(field) => write!(f, "Missing field: {}", field),
            HttpTrackerError::FieldValueError(msg) => {
                write!(f, "Field value error: {}", msg)
            },
            HttpTrackerError::PeerHostError(e) => write!(f, "Peer host error: {}", e),
            HttpTrackerError::SerdeQsError(e) => write!(f, "Serde qs error: {}", e),
        }
    }
}

impl From<reqwest::Error> for HttpTrackerError {
    fn from(e: reqwest::Error) -> Self {
        HttpTrackerError::RequestError(e)
    }
}

impl From<ParseError> for HttpTrackerError {
    fn from(e: ParseError) -> Self {
        HttpTrackerError::ResponseParseError(e)
    }
}

impl From<tracker::PeerHostError> for HttpTrackerError {
    fn from(e: tracker::PeerHostError) -> Self {
        HttpTrackerError::PeerHostError(e)
    }
}

impl From<serde_qs::Error> for HttpTrackerError {
    fn from(e: serde_qs::Error) -> Self {
        HttpTrackerError::SerdeQsError(e)
    }
}

impl std::error::Error for HttpTrackerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HttpTrackerError::RequestError(e) => Some(e),
            HttpTrackerError::ResponseParseError(e) => Some(e),
            HttpTrackerError::PeerHostError(e) => Some(e),
            HttpTrackerError::SerdeQsError(e) => Some(e),
            _ => None,
        }
    }
}
