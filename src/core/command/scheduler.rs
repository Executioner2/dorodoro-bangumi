//! 向调度器发送的指令

use crate::core::command::CommandHandler;
use crate::core::command::peer_manager::Command as PeerManagerCommand;
use crate::core::scheduler::Scheduler;
use tracing::info;

#[derive(Debug)]
pub enum Command {
    /// 关闭调度器
    Shutdown(Shutdown),

    PeerManager(PeerManager),
}

impl CommandHandler for Command {
    type Target = Scheduler;

    async fn handle(self, context: &mut Self::Target) {
        match self {
            Command::Shutdown(v) => v.handle(context).await,
            Command::PeerManager(v) => v.handle(context).await,
        }
    }
}

/// 关机指令
#[derive(Debug)]
pub struct Shutdown;
impl Into<Command> for Shutdown {
    fn into(self) -> Command {
        Command::Shutdown(self)
    }
}
impl CommandHandler for Shutdown {
    type Target = Scheduler;

    async fn handle(self, context: &mut Self::Target) {
        context.shutdown();
    }
}

/// 种子管理器指令
#[derive(Debug)]
pub struct PeerManager {
    peer_manager_command: PeerManagerCommand,
}
impl Into<Command> for PeerManager {
    fn into(self) -> Command {
        Command::PeerManager(self)
    }
}
impl CommandHandler for PeerManager {
    type Target = Scheduler;

    async fn handle(self, context: &mut Self::Target) {
        info!("PeerManager command received");
    }
}
