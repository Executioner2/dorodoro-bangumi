//! 写日志基准性能测试

use criterion::{criterion_group, criterion_main, Criterion};
use dorodoro_bangumi::log;
use tracing::{info, Level};

fn write_log(c: &mut Criterion) {
    let _guard = log::register_logger("logs", "dorodoro-bangumi", 10 << 20, 2, Level::INFO).unwrap();

    c.bench_function("write_log", |b| {
        b.iter(|| {
            // 日志写入代码
            info!("Hello, world!");
        });
    });
}

criterion_group!(benches, write_log);
criterion_main!(benches);