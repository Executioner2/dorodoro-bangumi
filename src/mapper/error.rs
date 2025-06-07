pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    MappingError(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::MappingError(msg) => write!(_f, "Mapping error: {}", msg),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            _ => None,
        }
    }
}
