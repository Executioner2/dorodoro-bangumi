extern crate alloc;
extern crate core;

pub mod bt;
pub mod util;
pub mod mapper;

pub use bt::*;
pub use util::*;

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
