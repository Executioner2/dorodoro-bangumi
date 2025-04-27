use core::fmt::{Display, Formatter};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error {}

impl Display for Error {
    fn fmt(&self, _f: &mut Formatter<'_>) -> core::fmt::Result {
        unimplemented!()
    }
}
