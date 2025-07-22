extern crate alloc;

pub mod bt;
pub mod core;
pub mod mapper;
pub mod service;
pub mod api;

pub use bt::*;
pub use core::*;
pub use service::*;

pub trait Integer: Copy + Sized {}
impl Integer for i8 {}
impl Integer for u8 {}
impl Integer for i16 {}
impl Integer for u16 {}
impl Integer for i32 {}
impl Integer for u32 {}
impl Integer for i64 {}
impl Integer for u64 {}
impl Integer for i128 {}
impl Integer for u128 {}
impl Integer for isize {}
impl Integer for usize {}

pub trait BoxWrapper {
    fn to_box(self) -> Box<Self>;
}

impl<T> BoxWrapper for T {
    fn to_box(self) -> Box<Self> {
        Box::new(self)
    }
}
