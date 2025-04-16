use crate::core::alias::SenderPeer;
use crate::core::command::CommandHandler;
use crate::core::peer_manager::{Gasket, PeerManagerContext};
use crate::torrent::TorrentArc;
use crate::tracker::Host;
use std::fmt;
use tracing::info;

pub enum Command {
    NewDownloadTask(NewDownloadTask),
    PeerJoin(PeerJoin),
    PeerExit(PeerExit),
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::NewDownloadTask(_) => write!(f, "NewDownloadTask"),
            Command::PeerJoin(_) => write!(f, "PeerJoin"),
            Command::PeerExit(_) => write!(f, "PeerExit"),
        }
    }
}

impl CommandHandler<'_> for Command {
    type Target = PeerManagerContext;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::NewDownloadTask(cmd) => cmd.handle(context).await,
            Command::PeerJoin(cmd) => cmd.handle(context).await,
            Command::PeerExit(cmd) => cmd.handle(context).await,
        }
    }
}

/// 新的下载任务
#[derive(Debug)]
pub struct NewDownloadTask {
    torrent: TorrentArc,
    hosts: Vec<Host>,
}
impl NewDownloadTask {
    pub fn new(torrent: TorrentArc, hosts: Vec<Host>) -> Self {
        Self { torrent, hosts }
    }
}
impl From<NewDownloadTask> for Command {
    fn from(value: NewDownloadTask) -> Self {
        Command::NewDownloadTask(value)
    }
}
impl CommandHandler<'_> for NewDownloadTask {
    type Target = PeerManagerContext;

    async fn handle(self, context: Self::Target) {
        let mut gasket = Gasket::new(
            self.torrent,
            context.spm.clone(),
            context.cancle_token.clone(),
            context.config.channel_buffer(),
            self.hosts,
        );
        if let Ok(id) = gasket.start().await {
            context.gaskets.lock().await.insert(id, gasket);
        }
    }
}

/// Peer 完成握手
#[derive(Debug)]
pub struct PeerJoin {
    id: u64,
    sender_peer: SenderPeer,
}
impl PeerJoin {
    pub fn new(id: u64, sender_peer: SenderPeer) -> Self {
        Self { id, sender_peer }
    }
}
impl From<PeerJoin> for Command {
    fn from(value: PeerJoin) -> Self {
        Command::PeerJoin(value)
    }
}
impl CommandHandler<'_> for PeerJoin {
    type Target = PeerManagerContext;

    async fn handle(self, _context: Self::Target) {
        info!("peer完成了握手")
    }
}

/// Peer 退出
#[derive(Debug)]
pub struct PeerExit(pub u64);
impl From<PeerExit> for Command {
    fn from(value: PeerExit) -> Self {
        Command::PeerExit(value)
    }
}
impl CommandHandler<'_> for PeerExit {
    type Target = PeerManagerContext;

    async fn handle(self, _context: Self::Target) {
        info!("peer退出")
    }
}
