use crate::peer_manager::PeerManager;
use crate::torrent::TorrentArc;
use tracing::trace;
use anyhow::Result;
use doro_util::command_system;
use doro_util::global::Id;

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
}
impl<'a> CommandHandler<'a, Result<()>> for NewDownloadTask {
    type Target = &'a mut PeerManager;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        trace!("收到新的下载任务");
        ctx.start_gasket(self.torrent).await;
        Ok(())
    }
}

/// Gasket 退出
#[derive(Debug)]
pub struct GasketExit(pub Id);
impl<'a> CommandHandler<'a, Result<()>> for GasketExit {
    type Target = &'a mut PeerManager;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.pmc.remove_gasket(self.0).await;
        Ok(())
    }
}
