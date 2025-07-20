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
        // 补药使用 read，因为我们用 Box 在堆上创建的数据
        // 这里只是复制指针数据，并没有建立所有权关系，也就不会自动释放内存
        // unsafe { (self.inner as *const T).read() } ❌
        unsafe { *Box::from_raw(self.inner as *mut T) }
    }
}

unsafe impl Send for TransferPtr {}

/// 命令枚举的标记
pub trait CommandEnum {}

impl<T: CommandEnum> From<T> for TransferPtr {
    fn from(value: T) -> Self {
        let data = Box::new(value);
        let inner = Box::into_raw(data) as *const ();
        Self::new(inner)
    }
}
