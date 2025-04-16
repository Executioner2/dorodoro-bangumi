pub mod gasket;
pub mod peer;
pub mod peer_manager;
pub mod scheduler;
pub mod tcp_server;

/// 命令处理器
pub trait CommandHandler<'a> {
    type Target;

    async fn handle(self, context: Self::Target);
}
