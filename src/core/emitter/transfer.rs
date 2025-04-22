use crate::command::CommandHandler;

#[derive(Eq, PartialEq, Debug)]
pub struct TransferPtr {
    pub inner: *const (),
}
impl TransferPtr {
    pub fn new(inner: *const ()) -> Self {
        Self { inner }
    }
    
    pub fn instance<T: CommandHandler>(self) -> T {
        unsafe {(self.inner as *const T).read()}
    }
}

unsafe impl Send for TransferPtr {}
unsafe impl Sync for TransferPtr {}

impl<T: CommandHandler> From<T> for TransferPtr {
    fn from(value: T) -> Self {
        let data = Box::new(value);
        let inner = Box::into_raw(data) as *const ();
        Self::new(inner)
    }
}