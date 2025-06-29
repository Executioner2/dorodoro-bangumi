//! 向调度器发送的指令

use crate::command_system;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::peer_manager::command;
use crate::core::scheduler::{Scheduler, SchedulerContext};
use crate::mapper::torrent::TorrentMapper;
use crate::torrent::TorrentArc;
use core::fmt::{Debug, Formatter};
use std::path::PathBuf;
use anyhow::Result;

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
        if !ctx.context.is_cancelled() {
            ctx.context.cancel();
        }
        Ok(())
    }
}

/// 添加种子
pub struct TorrentAdd {
    pub torrent: TorrentArc,
    pub path: PathBuf,
}
impl Debug for TorrentAdd {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Torrent Add")
    }
}
impl<'a> CommandHandler<'a, Result<()>> for TorrentAdd {
    type Target = &'a Scheduler;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        async fn task(torrent: TorrentArc, sc: SchedulerContext) {
            // 添加到下载列表，新的下载任务才需要添加，如果添加的是已经添加了的旧的任务，忽略掉
            let emitter = sc.emitter;
            let cmd = command::NewDownloadTask {
                torrent: torrent.clone(),
            }
            .into();
            emitter.send(PEER_MANAGER, cmd).await.unwrap();
        }

        let sc = ctx.get_context();
        let conn = sc.context.get_conn().await.unwrap();
        if conn.add_torrent(self.torrent.clone(), self.path) {
            tokio::spawn(task(self.torrent, sc));
        }

        Ok(())
    }
}
