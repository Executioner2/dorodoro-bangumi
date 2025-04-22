use crate::core::command::CommandHandler;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::peer_manager::PeerManagerContext;
use crate::peer_manager::GasketInfo;
use crate::peer_manager::gasket::Gasket;
use crate::runtime::Runnable;
use crate::torrent::TorrentArc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::trace;

#[derive(Debug)]
pub enum Command {
    NewDownloadTask(NewDownloadTask),
    GasketExit(GasketExit),
}

impl CommandHandler for Command {
    type Target = PeerManagerContext;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::NewDownloadTask(cmd) => cmd.handle(context).await,
            Command::GasketExit(cmd) => cmd.handle(context).await,
        }
    }
}

/// 新的下载任务
#[derive(Debug)]
pub struct NewDownloadTask {
    pub torrent: TorrentArc,
    pub download_path: String,
}
impl CommandHandler for NewDownloadTask {
    type Target = PeerManagerContext;

    async fn handle(self, context: Self::Target) {
        trace!("收到新的下载任务");
        let download = 0;
        let uploaded = 0;
        let mut emitter = Emitter::new();
        emitter
            .register(
                PEER_MANAGER,
                context.emitter.get(PEER_MANAGER).await.unwrap(),
            )
            .await
            .unwrap();

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
            context.config.clone(),
        );

        let join_handle = tokio::spawn(gasket.run());
        let gasket_info = GasketInfo {
            id: gasket_id,
            torrent: self.torrent,
            peer_id,
            join_handle,
        };

        context.add_gasket(gasket_info).await;
    }
}

#[derive(Debug)]
pub struct GasketExit(pub u64);
impl CommandHandler for GasketExit {
    type Target = PeerManagerContext;

    async fn handle(self, context: Self::Target) {
        context.remove_gasket(self.0).await;
    }
}
