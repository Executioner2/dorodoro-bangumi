//! 向调度器发送的指令

use crate::core::command::CommandHandler;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::peer_manager::command;
use crate::core::scheduler::SchedulerContext;
use crate::torrent::TorrentArc;
use core::fmt::{Debug, Formatter};
use tracing::info;

#[derive(Debug)]
pub enum Command {
    /// 关闭调度器
    Shutdown(Shutdown),

    /// 种子
    TorrentAdd(TorrentAdd),
}

impl CommandHandler for Command {
    type Target = SchedulerContext;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::Shutdown(v) => v.handle(context).await,
            Command::TorrentAdd(v) => v.handle(context).await,
        }
    }
}

/// 关机指令
#[derive(Debug)]
pub struct Shutdown;
impl CommandHandler for Shutdown {
    type Target = SchedulerContext;

    async fn handle(self, context: Self::Target) {
        if !context.cancel_token.is_cancelled() {
            context.cancel_token.cancel();
        }
    }
}

/// 添加种子
pub struct TorrentAdd(pub TorrentArc, pub String);
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
impl CommandHandler for TorrentAdd {
    type Target = SchedulerContext;

    async fn handle(self, context: Self::Target) {
        async fn task(torrent: TorrentArc, download_path: String, context: SchedulerContext) {
            // 添加到下载列表
            info!("开始查询历史下载量等操作");
            let emitter = context.emitter;
            emitter
                .send(
                    PEER_MANAGER,
                    command::NewDownloadTask {
                        torrent: torrent.clone(),
                        download_path,
                    }
                    .into(),
                )
                .await
                .unwrap();
        }
        tokio::spawn(task(self.0, self.1, context));
    }
}
