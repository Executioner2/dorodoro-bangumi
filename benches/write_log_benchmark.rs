//! 写日志基准性能测试

use criterion::{Criterion, criterion_group, criterion_main};
use tracing::{info, Level};
use doro_util::default_logger;

default_logger!(Level::INFO);

fn write_log(c: &mut Criterion) {
    c.bench_function("write_log", |b| {
        b.iter(|| {
            // 日志写入代码
            info!("Hello, world!");
        });
    });
}

criterion_group!(benches, write_log);
criterion_main!(benches);
