use crate::core::command::CommandHandler;
use crate::core::peer_manager::PeerManager;
use crate::torrent::Torrent;
use tracing::info;

#[derive(Debug)]
pub enum Command {
    TorrentAdd(TorrentAdd),
}

impl CommandHandler for Command {
    type Target = PeerManager;

    async fn handle(self, context: &mut Self::Target) {
        match self {
            Command::TorrentAdd(cmd) => cmd.handle(context).await,
        }
    }
}

#[derive(Debug, Hash)]
pub struct TorrentAdd {
    torrent: Torrent,
}

/// 添加种子命令处理
impl CommandHandler for TorrentAdd {
    type Target = PeerManager;

    async fn handle(self, context: &mut Self::Target) {
        info!("TorrentAdd command received")
    }
}
