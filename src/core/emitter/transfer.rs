use crate::command::CommandHandler;

#[derive(Eq, PartialEq, Debug)]
pub struct TransferPtr {
    pub inner: *const (),
}
impl TransferPtr {
    pub fn new(inner: *const ()) -> Self {
        Self { inner }
    }

    pub fn instance<'a, T: CommandHandler<'a, U>, U>(self) -> T {
        unsafe { (self.inner as *const T).read() }
    }
}

unsafe impl Send for TransferPtr {}
unsafe impl Sync for TransferPtr {}

/// 命令枚举的标记
pub trait CommandEnum {}

impl<T: CommandEnum> From<T> for TransferPtr {
    fn from(value: T) -> Self {
        let data = Box::new(value);
        let inner = Box::into_raw(data) as *const ();
        Self::new(inner)
    }
}
