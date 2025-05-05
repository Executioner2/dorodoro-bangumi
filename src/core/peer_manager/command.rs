use std::path::PathBuf;
use crate::command_system;
use crate::core::command::CommandHandler;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::peer_manager::gasket::Gasket;
use crate::peer_manager::{GasketInfo, PeerManager};
use crate::runtime::Runnable;
use crate::torrent::TorrentArc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::trace;
use super::error::Result;

command_system! {
    ctx: PeerManager,
    Command {
        NewDownloadTask,
        GasketExit,
    }
}

/// 新的下载任务
#[derive(Debug)]
pub struct NewDownloadTask {
    pub torrent: TorrentArc,
    pub download_path: PathBuf,
}
impl<'a> CommandHandler<'a, Result<()>> for NewDownloadTask {
    type Target = &'a mut PeerManager;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        trace!("收到新的下载任务");
        let context = ctx.get_context();

        let download = 0;
        let uploaded = 0;
        let mut emitter = Emitter::new();
        context.emitter.get(PEER_MANAGER).map(async |send| {
            emitter.register(PEER_MANAGER, send);
        });

        let gasket_id = context.gasket_id.fetch_add(1, Ordering::Relaxed);
        let peer_id = Arc::new(context.get_peer_id().await);
        let gasket = Gasket::new(
            gasket_id,
            self.torrent.clone(),
            context.clone(),
            peer_id.clone(),
            self.download_path,
            download,
            uploaded,
            emitter,
            context.store.clone(),
        );

        let join_handle = tokio::spawn(gasket.run());
        let gasket_info = GasketInfo {
            id: gasket_id,
            peer_id,
            join_handle,
        };

        context.add_gasket(gasket_info).await;
        Ok(())
    }
}

/// Gasket 退出
#[derive(Debug)]
pub struct GasketExit(pub u64);
impl<'a> CommandHandler<'a, Result<()>> for GasketExit {
    type Target = &'a mut PeerManager;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        let context = ctx.get_context();
        context.remove_gasket(self.0).await;
        Ok(())
    }
}
