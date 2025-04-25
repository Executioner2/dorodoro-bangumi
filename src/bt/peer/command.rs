use crate::core::command::CommandHandler;
use tracing::trace;
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::peer::Peer;
use crate::peer_manager::gasket::ExitReason;

pub enum Command {
    Download(Download),
    Exit(Exit),
}

impl CommandEnum for Command {}

impl<'a> CommandHandler<'a> for Command {
    type Target = &'a mut Peer;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::Download(cmd) => cmd.handle(context).await,
            Command::Exit(cmd) => cmd.handle(context).await,
        }
    }
}

pub struct Download;
impl From<Download> for Command {
    fn from(value: Download) -> Self {
        Command::Download(value)
    }
}
impl Into<TransferPtr> for Download {
    fn into(self) -> TransferPtr {
        Command::Download(self).into()
    }
}
impl<'a> CommandHandler<'a> for Download {
    type Target = &'a mut Peer;

    async fn handle(self, _context: Self::Target) {
        trace!("被唤醒，执行下载任务");
        unimplemented!()
        // let _ = context.try_find_downloadable_pices().await;
    }
}

pub struct Exit {
    pub reason: ExitReason
}
impl Into<TransferPtr> for Exit {
    fn into(self) -> TransferPtr {
        Command::Exit(self).into()
    }
}
impl<'a> CommandHandler<'a> for Exit {
    type Target = &'a mut Peer;

    /// 什么都不需要做，在 peer 中会处理的
    async fn handle(self, _context: Self::Target) {
        unimplemented!()
    }
}
