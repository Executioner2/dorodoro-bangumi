use crate::core::command::CommandHandler;
use crate::core::tcp_server::TcpServerContext;
use tracing::trace;

#[derive(Debug)]
pub enum Command {
    Exit(Exit),
}

impl CommandHandler<'_> for Command {
    type Target = TcpServerContext;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::Exit(v) => v.handle(context).await,
        }
    }
}

#[derive(Debug, Hash)]
pub struct Exit(pub u64);
impl From<Exit> for Command {
    fn from(value: Exit) -> Self {
        Command::Exit(value)
    }
}
impl CommandHandler<'_> for Exit {
    type Target = TcpServerContext;

    async fn handle(self, context: Self::Target) {
        trace!("Removing connection with id: {}", self.0);
        context.remove_conn(self.0).await;
    }
}
