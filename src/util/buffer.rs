//! 字节流缓冲区，应该配合 [`bytes::Bytes`] 使用。相较于直接使用 `Vec<u8>`，`ByteBuffer` 减少了数据初始化，最多
//! 可提升五倍的性能。见 `benches/socket_buffer_benchmark.rs`

use std::alloc::Layout;
use std::ptr::NonNull;

pub struct ByteBuffer {
    bytes: NonNull<u8>,
    size: usize,
    capacity: usize,
}

unsafe impl Send for ByteBuffer {}
unsafe impl Sync for ByteBuffer {}

impl ByteBuffer {

    /// 创建一个新的 ByteBuffer，分配 `capacity` 字节的内存
    ///
    /// # Examples
    /// ```
    /// use dorodoro_bangumi::util::buffer::ByteBuffer;
    /// let buffer = ByteBuffer::new(1024);
    /// ```
    pub fn new(capacity: usize) -> Self {
        if capacity == 0 {
            Self { bytes: NonNull::dangling(), size: 0, capacity: 0 }
        } else {
            let layout = Layout::array::<u8>(capacity).unwrap();
            let ptr = unsafe { std::alloc::alloc(layout) };
            Self { bytes: NonNull::new(ptr).unwrap(), size: capacity, capacity }
        }
    }

    /// 设置有效的大小，不会重新分配内存
    ///
    /// # Examples
    /// ```
    /// use dorodoro_bangumi::util::buffer::ByteBuffer;
    /// let mut buffer = ByteBuffer::new(1024);
    /// buffer.resize(512);
    /// ```
    pub fn resize(&mut self, size: usize) {
        let size = size.min(self.capacity);
        self.size = size;
    }

    /// self.size 必须小于 capacity 的一半时，缩减分配的多余容量
    ///
    /// # Examples
    /// ```
    /// use dorodoro_bangumi::util::buffer::ByteBuffer;
    /// let mut buffer = ByteBuffer::new(1024);
    /// buffer.shrink(512);
    /// ```
    pub fn shrink(&mut self, capacity: usize) {
        if self.size < capacity { return; }
        let old_layout = Layout::array::<u8>(self.capacity).unwrap();
        let new_layout = Layout::array::<u8>(capacity).unwrap();
        let ptr = unsafe { std::alloc::realloc(self.bytes.as_ptr(), old_layout, new_layout.size()) };
        self.bytes = NonNull::new(ptr).unwrap();
        self.capacity = capacity;
        self.resize(capacity);
    }
}

impl AsRef<[u8]> for ByteBuffer {
    fn as_ref(&self) -> &[u8] {
        unsafe {
            let layout = Layout::array::<u8>(self.size).unwrap();
            std::slice::from_raw_parts(self.bytes.as_ptr(), layout.size())
        }
    }
}

impl AsMut<[u8]> for ByteBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        unsafe {
            let layout = Layout::array::<u8>(self.size).unwrap();
            std::slice::from_raw_parts_mut(self.bytes.as_ptr(), layout.size())
        }
    }
}

impl Drop for ByteBuffer {
    fn drop(&mut self) {
        if self.capacity == 0 { return; }
        unsafe {
            let layout = Layout::array::<u8>(self.capacity).unwrap();
            std::alloc::dealloc(self.bytes.as_ptr(), layout);
        }
    }
}