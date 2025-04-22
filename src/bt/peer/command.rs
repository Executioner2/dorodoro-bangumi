use crate::core::command::CommandHandler;
use tracing::trace;

pub enum Command {
    Download(Download),
    Exit(Exit),
}

impl CommandHandler for Command {
    type Target = ();

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
impl CommandHandler for Download {
    type Target = ();

    async fn handle(self, _context: Self::Target) {
        trace!("被唤醒，执行下载任务");
        unimplemented!()
        // let _ = context.try_find_downloadable_pices().await;
    }
}

pub struct Exit;
impl CommandHandler for Exit {
    type Target = ();

    async fn handle(self, _context: Self::Target) {
        todo!()
    }
}
