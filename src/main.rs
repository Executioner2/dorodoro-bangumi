use dorodoro_bangumi::{bootstrap, default_logger};
use tracing::Level;

default_logger!(Level::DEBUG);

#[tokio::main]
async fn main() {
    bootstrap::start().await;
}
