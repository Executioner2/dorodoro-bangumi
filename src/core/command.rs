pub mod peer;
pub mod peer_manager;
pub mod scheduler;

/// 命令处理器
pub trait CommandHandler {
    type Target;

    async fn handle(self, context: &mut Self::Target);
}
