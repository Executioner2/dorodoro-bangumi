use crate::core::command::CommandHandler;
use crate::core::peer_manager::PeerManager;
use crate::torrent::Torrent;
use tracing::info;

#[derive(Debug)]
pub enum Command {
    TorrentAdd(TorrentAdd),
}

impl<'a> CommandHandler<'a> for Command {
    type Target = &'a mut PeerManager;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::TorrentAdd(cmd) => cmd.handle(context).await,
        }
    }
}

/// 添加种子命令处理
#[derive(Debug, Hash)]
pub struct TorrentAdd {
    torrent: Torrent,
}
impl From<TorrentAdd> for Command {
    fn from(value: TorrentAdd) -> Self {
        Command::TorrentAdd(value)
    }
}
impl<'a> CommandHandler<'a> for TorrentAdd {
    type Target = &'a mut PeerManager;

    async fn handle(self, _context: Self::Target) {
        info!("TorrentAdd command received")
    }
}
