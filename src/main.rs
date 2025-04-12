use dorodoro_bangumi::core::bootstrap::Bootstrap;
use dorodoro_bangumi::log;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;

/// 全局初始化
fn global_init() -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    // warning - 这个日志占用了 4MB 的内存，后续看看有没有平替
    let guard = log::register_logger("logs", "dorodoro-bangumi", 10 << 20, 2, Level::INFO)?;
    Ok(guard)
}

#[tokio::main]
async fn main() {
    let _guard = global_init().unwrap(); // guard 一定要接收，保证写日志正常
    Bootstrap::start().await;
}
