//! 向调度器发送的指令

use super::error::Result;
use crate::command_system;
use crate::core::command::CommandHandler;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::peer_manager::command;
use crate::core::scheduler::{Scheduler, SchedulerContext};
use crate::db::ConnWrapper;
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::peer_manager::gasket::PieceStatus;
use crate::torrent::{TorrentArc, TorrentStatus};
use bincode::config;
use core::fmt::{Debug, Formatter};
use std::path::PathBuf;
use tracing::warn;

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
impl TorrentAdd {
    fn save_if_absent(&self, conn: ConnWrapper) -> bool {
        let mut stmt = conn
            .prepare_cached("select count(*) from torrent where info_hash = ?1")
            .unwrap();
        let info_hash = &self.torrent.info_hash;
        let count: u32 = stmt.query_row([info_hash], |row| row.get(0)).unwrap();
        if count > 0 {
            warn!("重复添加的 torrent");
            return false;
        }

        let mut stmt = conn.prepare_cached("insert into torrent(info_hash, serial, status, bytefield, underway_bytefield, save_path) values (?1, ?2, ?3, ?4, ?5, ?6)").unwrap();
        let serial = bincode::encode_to_vec(self.torrent.inner(), config::standard()).unwrap();
        let bytefield = vec![0u8; ((self.torrent.info.pieces.len() / 20) + 7) / 8];
        let underway_bytefield: Vec<(u32, PieceStatus)> = vec![];
        let underway_bytefield =
            bincode::encode_to_vec(underway_bytefield, config::standard()).unwrap();
        stmt.execute((
            info_hash,
            &serial,
            TorrentStatus::Download as usize,
            bytefield,
            underway_bytefield,
            self.path.to_str().unwrap(),
        ))
        .unwrap();
        true
    }
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
        if self.save_if_absent(conn) {
            tokio::spawn(task(self.torrent, sc));
        }

        Ok(())
    }
}
