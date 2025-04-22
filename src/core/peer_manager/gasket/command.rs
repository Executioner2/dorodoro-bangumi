use crate::command::CommandHandler;
use crate::peer_manager::gasket::{ExitReason, GasketContext};
use tracing::info;

#[derive(Debug)]
pub enum Command {
    PeerJoin(PeerJoin),
    PeerExit(PeerExit),
}

impl CommandHandler for Command {
    type Target = GasketContext;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::PeerJoin(cmd) => cmd.handle(context).await,
            Command::PeerExit(cmd) => cmd.handle(context).await,
        }
    }
}

/// Peer 完成握手
#[derive(Debug)]
pub struct PeerJoin {
    id: u64,
}
impl PeerJoin {
    pub fn new(id: u64) -> Self {
        Self { id }
    }
}
impl CommandHandler for PeerJoin {
    type Target = GasketContext;

    async fn handle(self, _context: Self::Target) {
        info!("peer完成了握手")
    }
}

/// Peer 退出
#[derive(Debug)]
pub struct PeerExit {
    pub peer_no: u64,
    pub reasone: ExitReason,
}
impl CommandHandler for PeerExit {
    type Target = GasketContext;

    async fn handle(self, mut context: Self::Target) {
        context.peer_exit(self.peer_no, self.reasone).await;
    }
}
