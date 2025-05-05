use crate::db;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    DbError(db::error::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::DbError(e) => write!(f, "db error: {}", e),
        }
    }
}

impl From<db::error::Error> for Error {
    fn from(e: db::error::Error) -> Self {
        Error::DbError(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::DbError(e) => Some(e),
        }
    }
}
