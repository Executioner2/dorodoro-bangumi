//! 向调度器发送的指令

use crate::command_system;
use crate::core::command::CommandHandler;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::peer_manager::command;
use crate::core::scheduler::{Scheduler, SchedulerContext};
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::torrent::TorrentArc;
use core::fmt::{Debug, Formatter};
use std::path::PathBuf;
use tracing::info;
use super::error::Result;

command_system! {
    ctx: Scheduler,
    Command {
        Shutdown,
        TorrentAdd,
    }
}

/// 关机指令
#[derive(Debug)]
pub struct Shutdown;
impl<'a> CommandHandler<'a, Result<()>> for Shutdown {
    type Target = &'a Scheduler;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        if !ctx.cancel_token.is_cancelled() {
            ctx.cancel_token.cancel();
        }
        Ok(())
    }
}

/// 添加种子
pub struct TorrentAdd {
    pub torrent: TorrentArc,
    pub path: PathBuf   
}
impl Debug for TorrentAdd {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Torrent Add")
    }
}
impl<'a> CommandHandler<'a, Result<()>> for TorrentAdd {
    type Target = &'a Scheduler;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        async fn task(torrent: TorrentArc, download_path: PathBuf, context: SchedulerContext) {
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
        let context = ctx.get_context();
        tokio::spawn(task(self.torrent, self.path, context));
        Ok(())
    }
}
