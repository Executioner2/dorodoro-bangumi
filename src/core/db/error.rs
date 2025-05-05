use tokio::sync::AcquireError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    IoError(std::io::Error),
    SqliteError(rusqlite::Error),
    AcquireError(AcquireError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::IoError(e) => write!(f, "IO error: {}", e),
            Error::SqliteError(e) => write!(f, "sqlite error: {}", e),
            Error::AcquireError(e) => write!(f, "acquire error: {}", e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::SqliteError(e)
    }
}

impl From<AcquireError> for Error {
    fn from(e: AcquireError) -> Self {
        Error::AcquireError(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IoError(e) => Some(e),
            Error::SqliteError(e) => Some(e),
            Error::AcquireError(e) => Some(e),
        }
    }
}
