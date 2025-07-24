use doro::bootstrap;
use doro_util::default_logger;
use tracing::Level;


#[cfg(not(feature = "dev"))]
default_logger!();

#[cfg(not(feature = "dev"))]
#[tokio::main]
async fn main() {
    bootstrap::start().await;
}

// 启动 tokio console
#[cfg(feature = "dev")]
fn main() {
    console_subscriber::ConsoleLayer::builder()
        .retention(std::time::Duration::from_secs(60))
        .server_addr(([127, 0, 0, 1], 9090))
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(Box::pin(bootstrap::start()));
}
