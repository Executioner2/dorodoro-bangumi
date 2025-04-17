//! 向调度器发送的指令

use crate::core::alias::SenderController;
use crate::core::command::peer_manager::Command as PeerManagerCommand;
use crate::core::command::{CommandHandler, peer_manager};
use crate::core::scheduler::SchedulerContext;
use crate::torrent::TorrentArc;
use crate::tracker;
use crate::tracker::Event;
use core::fmt::{Debug, Formatter};
use tracing::info;

#[derive(Debug)]
pub enum Command {
    /// 关闭调度器
    Shutdown(Shutdown),

    /// 种子
    TorrentAdd(TorrentAdd),

    /// Peer管理
    PeerManager(PeerManager),
}

impl CommandHandler<'_> for Command {
    type Target = SchedulerContext;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::Shutdown(v) => v.handle(context).await,
            Command::PeerManager(v) => v.handle(context).await,
            Command::TorrentAdd(v) => v.handle(context).await,
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
impl CommandHandler<'_> for Shutdown {
    type Target = SchedulerContext;

    async fn handle(self, context: Self::Target) {
        if !context.cancel_token.is_cancelled() {
            context.cancel_token.cancel();
        }
    }
}

/// peer管理器指令
#[derive(Debug)]
pub struct PeerManager {
    peer_manager_command: PeerManagerCommand,
}
impl PeerManager {
    pub fn new(pmc: PeerManagerCommand) -> Self {
        Self {
            peer_manager_command: pmc,
        }
    }
}
impl Into<Command> for PeerManager {
    fn into(self) -> Command {
        Command::PeerManager(self)
    }
}
impl CommandHandler<'_> for PeerManager {
    type Target = SchedulerContext;

    async fn handle(self, context: Self::Target) {
        context.spm.send(self.peer_manager_command).await.unwrap();
    }
}

/// 添加种子
pub struct TorrentAdd(pub SenderController, pub TorrentArc, pub String);
impl Debug for TorrentAdd {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Torrent Add")
    }
}
impl Into<Command> for TorrentAdd {
    fn into(self) -> Command {
        Command::TorrentAdd(self)
    }
}
impl CommandHandler<'_> for TorrentAdd {
    type Target = SchedulerContext;

    async fn handle(self, context: Self::Target) {
        async fn task(_resp: SenderController, torrent: TorrentArc, download_path: String, context: SchedulerContext) {
            // 添加到下载列表
            info!("开始解析tracker");
            let download = 0; // 查询数据库
            let upload = 0; // 查询数据库
            let port = context.config.tcp_server_addr().port();
            let peers = tracker::discover_peer(torrent.clone(), download, upload, port, Event::None).await;
            // let peers = vec![];
            let spm = context.spm;
            spm.send(peer_manager::NewDownloadTask::new(torrent.clone(), peers, download_path, port).into())
                .await
                .unwrap();
        }
        tokio::spawn(task(self.0, self.1, self.2, context));
    }
}
