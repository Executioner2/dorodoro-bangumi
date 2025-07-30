use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;

use super::ByteBuffer;

/// 测试是否能正常回收内存，因为有 println，需要添加 --nocapture 保证不会被兜住
#[test]
#[ignore]
fn test_drop_safe() {
    for _ in 0..10000 {
        let buf: Bytes;
        {
            let mut bytes = ByteBuffer::new(1024);
            bytes.as_mut().iter_mut().for_each(|x| *x = 1);
            buf = bytes.take();
        }
        assert_eq!(buf.as_ref(), &[1u8; 1024]);
        assert!(GLOBAL_ALLOCATOR.get_bytes_allocated() < 1024 * 15);
    }

    assert!(GLOBAL_ALLOCATOR.get_bytes_allocated() < 1024 * 15);
}

// ===========================================================================
// 内存使用量跟踪
// ===========================================================================

struct TrackingAllocator<A: GlobalAlloc>(A, AtomicU64);

unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.1.fetch_add(layout.size() as u64, Ordering::SeqCst);
        unsafe { self.0.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(ptr, layout) };
        self.1.fetch_sub(layout.size() as u64, Ordering::SeqCst);
    }
}

impl<A: GlobalAlloc> TrackingAllocator<A> {
    fn get_bytes_allocated(&self) -> u64 {
        self.1.load(Ordering::SeqCst)
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: TrackingAllocator<System> = TrackingAllocator(System, AtomicU64::new(0));
