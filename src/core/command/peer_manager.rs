use crate::core::alias::SenderPeer;
use crate::core::command::CommandHandler;
use crate::core::peer_manager::{Gasket, PeerManagerContext};
use crate::torrent::TorrentArc;
use crate::tracker::Host;
use std::fmt;
use std::sync::Arc;
use tracing::{info, trace};

pub enum Command {
    NewDownloadTask(NewDownloadTask),
    PeerJoin(PeerJoin),
    PeerExit(PeerExit),
    FlushAnnounce(FlushAnnounce),
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::NewDownloadTask(_) => write!(f, "NewDownloadTask"),
            Command::PeerJoin(_) => write!(f, "PeerJoin"),
            Command::PeerExit(_) => write!(f, "PeerExit"),
            Command::FlushAnnounce(_) => write!(f, "FlushAnnounce"),
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
            Command::FlushAnnounce(cmd) => cmd.handle(context).await,
        }
    }
}

/// 新的下载任务
#[derive(Debug)]
pub struct NewDownloadTask {
    torrent: TorrentArc,
    hosts: Vec<Host>,
    download_path: String,
    port: u16,
}
impl NewDownloadTask {
    pub fn new(torrent: TorrentArc, hosts: Vec<Host>, download_path: String, port: u16) -> Self {
        Self { torrent, hosts, download_path, port }
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
        trace!("收到新的下载任务");
        let mut gasket = Gasket::new(
            self.torrent,
            context.spm.clone(),
            context.cancle_token.clone(),
            context.config.channel_buffer(),
            self.hosts,
            Arc::new(self.download_path),
            self.port,
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


/// 刷新 Announce
#[derive(Debug)]
pub struct FlushAnnounce(pub [u8; 20]);
impl From<FlushAnnounce> for Command {
    fn from(value: FlushAnnounce) -> Self {
        Command::FlushAnnounce(value)
    }
}
impl CommandHandler<'_> for FlushAnnounce {
    type Target = PeerManagerContext;

    async fn handle(self, context: Self::Target) {
        if let Some(gasket) = context.gaskets.lock().await.get(&self.0) {
            gasket.start_peer().await;
        }
    }
}