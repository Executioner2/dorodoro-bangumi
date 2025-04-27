use super::error::Result;
use crate::command_system;
use crate::core::command::CommandHandler;
use crate::core::tcp_server::TcpServer;
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use tracing::trace;

command_system! {
    ctx: TcpServer,
    Command {
        Exit
    }
}

#[derive(Debug, Hash)]
pub struct Exit(pub u64);
impl<'a> CommandHandler<'a, Result<()>> for Exit {
    type Target = &'a TcpServer;

    async fn handle(self, context: Self::Target) -> Result<()> {
        trace!("Removing connection with id: {}", self.0);
        let context = context.get_context();
        tokio::spawn(async move {
            context.remove_conn(self.0).await;
        });
        Ok(())
    }
}
