use doro::bootstrap;
use doro_util::default_logger;
use tracing::Level;

default_logger!(Level::DEBUG);

#[tokio::main]
async fn main() {
    bootstrap::start().await;
}
