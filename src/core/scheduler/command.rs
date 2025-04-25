//! 向调度器发送的指令

use crate::command_system;
use crate::core::command::CommandHandler;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::peer_manager::command;
use crate::core::scheduler::{Scheduler, SchedulerContext};
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::torrent::TorrentArc;
use core::fmt::{Debug, Formatter};
use tracing::info;

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
impl<'a> CommandHandler<'a> for Shutdown {
    type Target = &'a Scheduler;

    async fn handle(self, ctx: Self::Target) {
        if !ctx.cancel_token.is_cancelled() {
            ctx.cancel_token.cancel();
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
impl<'a> CommandHandler<'a> for TorrentAdd {
    type Target = &'a Scheduler;

    async fn handle(self, ctx: Self::Target) {
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
        let context = ctx.get_context();
        tokio::spawn(task(self.0, self.1, context));
    }
}
