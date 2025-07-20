//! 写日志基准性能测试

use criterion::{Criterion, criterion_group, criterion_main};
use tracing::{info, Level};
use doro_util::log;

fn write_log(c: &mut Criterion) {
    let _guard =
        log::register_logger("logs", "dorodoro-bangumi", 10 << 20, 2, Level::INFO).unwrap();

    c.bench_function("write_log", |b| {
        b.iter(|| {
            // 日志写入代码
            info!("Hello, world!");
        });
    });
}

criterion_group!(benches, write_log);
criterion_main!(benches);
