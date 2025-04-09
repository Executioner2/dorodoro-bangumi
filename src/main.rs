use dorodoro_bangumi::log;
use tracing::{Level, debug, error, info, trace, warn};
use tracing_appender::non_blocking::WorkerGuard;

/// 全局初始化
fn global_init() -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    let guard = log::register_logger("logs", "dorodoro-bangumi", 10 << 20, 2, Level::INFO)?;
    Ok(guard)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = global_init()?; // guard 一定要接收，保证写日志正常

    trace!("Hello, world!");
    debug!("Hello, world!");
    info!("Hello, world!");
    warn!("Hello, world!");
    error!("Hello, world!");

    Ok(())
}
