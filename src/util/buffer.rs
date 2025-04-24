//! 字节流缓冲区，应该配合 [`bytes::Bytes`] 使用。相较于直接使用 `Vec<u8>`，`ByteBuffer` 减少了数据初始化，最多
//! 可提升五倍的性能。见 `benches/socket_buffer_benchmark.rs`

#[cfg(test)]
mod tests;

use bytes::Bytes;
use std::alloc::Layout;
use std::mem;
use std::ops::{Index, IndexMut, Range, RangeFrom};
use std::ptr::NonNull;

#[derive(Debug)]
struct BufInner {
    ptr: NonNull<u8>,
    size: usize,
    capacity: usize,
}

unsafe impl Send for BufInner {}
unsafe impl Sync for BufInner {}

impl BufInner {
    pub fn new(capacity: usize) -> Self {
        if capacity == 0 {
            Self {
                ptr: NonNull::dangling(),
                size: 0,
                capacity: 0,
            }
        } else {
            let layout = Layout::array::<u8>(capacity).unwrap();
            let ptr = unsafe { std::alloc::alloc(layout) };
            Self {
                ptr: NonNull::new(ptr).unwrap(),
                size: capacity,
                capacity,
            }
        }
    }

    pub fn resize(&mut self, size: usize) {
        let size = size.min(self.capacity);
        self.size = size;
    }

    pub fn shrink(&mut self, capacity: usize) {
        if self.size < capacity {
            return;
        }
        let old_layout = Layout::array::<u8>(self.capacity).unwrap();
        let new_layout = Layout::array::<u8>(capacity).unwrap();
        let ptr = unsafe { std::alloc::realloc(self.ptr.as_ptr(), old_layout, new_layout.size()) };
        self.ptr = NonNull::new(ptr).unwrap();
        self.capacity = capacity;
        self.resize(capacity);
    }
}

impl AsRef<[u8]> for BufInner {
    fn as_ref(&self) -> &[u8] {
        let layout = Layout::array::<u8>(self.size).unwrap();
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), layout.size()) }
    }
}

impl AsMut<[u8]> for BufInner {
    fn as_mut(&mut self) -> &mut [u8] {
        let layout = Layout::array::<u8>(self.size).unwrap();
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), layout.size()) }
    }
}

impl Index<Range<usize>> for BufInner {
    type Output = [u8];

    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.as_ref()[range]
    }
}

impl IndexMut<Range<usize>> for BufInner {
    fn index_mut(&mut self, range: Range<usize>) -> &mut Self::Output {
        &mut self.as_mut()[range]
    }
}

impl Index<RangeFrom<usize>> for BufInner {
    type Output = [u8];

    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        &self.as_ref()[range]
    }
}

impl IndexMut<RangeFrom<usize>> for BufInner {
    fn index_mut(&mut self, range: RangeFrom<usize>) -> &mut Self::Output {
        &mut self.as_mut()[range]
    }
}

impl Drop for BufInner {
    fn drop(&mut self) {
        if self.capacity == 0 {
            return;
        }
        unsafe {
            let layout = Layout::array::<u8>(self.capacity).unwrap();
            std::alloc::dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

#[derive(Debug)]
pub struct ByteBuffer {
    inner: BufInner,
}

impl ByteBuffer {
    /// 创建一个新的 ByteBuffer，分配 `capacity` 字节的内存
    ///
    /// # Examples
    /// ```
    /// use dorodoro_bangumi::util::buffer::ByteBuffer;
    /// let buffer = ByteBuffer::new(1024);
    /// ```
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: BufInner::new(capacity),
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
        self.inner.resize(size)
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
        self.inner.shrink(capacity)
    }

    pub fn len(&self) -> usize {
        self.inner.size
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }

    pub fn take(&mut self) -> Bytes {
        let mut inner = BufInner::new(0);
        mem::swap(&mut self.inner, &mut inner);
        Bytes::from_owner(inner)
    }
}

impl AsRef<[u8]> for ByteBuffer {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_ref()
    }
}

impl AsMut<[u8]> for ByteBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.inner.as_mut()
    }
}

impl Index<Range<usize>> for ByteBuffer {
    type Output = [u8];

    fn index(&self, range: Range<usize>) -> &Self::Output {
        self.inner.index(range)
    }
}

impl IndexMut<Range<usize>> for ByteBuffer {
    fn index_mut(&mut self, range: Range<usize>) -> &mut Self::Output {
        self.inner.index_mut(range)
    }
}

impl Index<RangeFrom<usize>> for ByteBuffer {
    type Output = [u8];

    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        self.inner.index(range)
    }
}

impl IndexMut<RangeFrom<usize>> for ByteBuffer {
    fn index_mut(&mut self, range: RangeFrom<usize>) -> &mut Self::Output {
        self.inner.index_mut(range)
    }
}
