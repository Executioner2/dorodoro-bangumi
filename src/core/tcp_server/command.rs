use crate::core::command::CommandHandler;
use crate::core::tcp_server::TcpServer;
use tracing::trace;

#[derive(Debug)]
#[allow(dead_code)]
pub enum Command {
    Exit(Exit),
}

impl<'a> CommandHandler<'a> for Command {
    type Target = &'a TcpServer;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::Exit(cmd) => cmd.handle(context).await,
        }
    }
}

#[derive(Debug, Hash)]
pub struct Exit(pub u64);
impl<'a> CommandHandler<'a> for Exit {
    type Target = &'a TcpServer;

    async fn handle(self, context: Self::Target) {
        trace!("Removing connection with id: {}", self.0);
        let context = context.get_context();
        tokio::spawn(async move {
            context.remove_conn(self.0).await;
        });
    }
}
