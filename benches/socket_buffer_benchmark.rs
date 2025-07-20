//! 针对 socket 中，对接收数据的 buffer 做性能测试。分别测试了手动分配，重新分配，自动分配，
//! 以及返回 Vec<u8>（对比 Bytes 是否有性能差异） 的性能。
//!
//! 测试平台：
//! - MacBook Air M3 16GB RAM Darwin Kernel Version 24.4.0 macOS Sequoia 15.4
//! - rustc 1.85.0 (4d91de4e4 2025-02-17)
//! - cargo 1.85.0 (d73d2caf9 2024-12-31)
//!
//! 测试结果：
//!
//! ```
//! buffer_default                  time:   [618.91 ns 624.28 ns 629.96 ns]
//! buffer_default_return_vec       time:   [619.14 ns 625.56 ns 631.76 ns]
//! buffer_alloc                    time:   [127.10 ns 129.61 ns 132.27 ns]
//! buffer_realloc                  time:   [286.99 ns 290.55 ns 294.41 ns]
//! ```
//! 结论：
//!
//! 从测试结果来看，封装 ByteBuffer 可以减少初始化带来的性能损失。不重新分配内存的情况下，
//! 最大带来 5 倍的性能提升。
use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use doro_util::buffer::ByteBuffer;

const EXPECT_SIZE: usize = 65507;
const RECV_SIZE: usize = 9572;

fn recv_from(bytes: &mut [u8], len: usize) -> usize {
    bytes[0..len].iter_mut().map(|b| *b = 1).count()
}

/// 手动分配，重新缩容量
fn buffer_realloc() -> Bytes {
    let mut buffer = ByteBuffer::new(EXPECT_SIZE);

    let size = recv_from(buffer.as_mut(), RECV_SIZE);

    buffer.shrink(size);
    Bytes::from_owner(buffer)
}

/// 手动分配，但不重新缩容量
fn buffer_alloc() -> Bytes {
    let mut buffer = ByteBuffer::new(EXPECT_SIZE);

    let size = recv_from(buffer.as_mut(), RECV_SIZE);

    buffer.resize(size);
    Bytes::from_owner(buffer)
}

/// 自动分配且初始化缓冲区，返回 Bytes
fn buffer_default() -> Bytes {
    let mut buffer = vec![0u8; EXPECT_SIZE];

    let _size = recv_from(&mut buffer, RECV_SIZE);

    Bytes::from_owner(buffer)
}

/// 自动分配且初始化缓冲区，返回 Vec<u8>
/// 主要对比 Bytes 是否有性能损失
fn buffer_default_return_vec() -> Vec<u8> {
    let mut buffer = vec![0u8; EXPECT_SIZE];

    let _size = recv_from(&mut buffer, RECV_SIZE);

    buffer
}

/// 性能测试
fn criterion_benchmark(c: &mut Criterion) {
    // time: [618.91 ns 624.28 ns 629.96 ns]
    c.bench_function("buffer_default", |b| b.iter(|| buffer_default()));

    // time: [619.14 ns 625.56 ns 631.76 ns]
    c.bench_function("buffer_default_return_vec", |b| {
        b.iter(|| buffer_default_return_vec())
    });

    // time: [127.10 ns 129.61 ns 132.27 ns]
    c.bench_function("buffer_alloc", |b| b.iter(|| buffer_alloc()));

    // time: [286.99 ns 290.55 ns 294.41 ns]
    c.bench_function("buffer_realloc", |b| b.iter(|| buffer_realloc()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
