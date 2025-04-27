use crate::core::command::CommandHandler;
use tracing::{trace};
use crate::command_system;
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::peer::Peer;
use crate::peer_manager::gasket::ExitReason;
use crate::bt::peer::error::Result;

command_system! {
    ctx: Peer,
    Command {
        Download,
        Exit,
    }
}

#[derive(Debug)]
pub struct Download {
    pub(crate) piece: (u32, u32)   
}
impl<'a> CommandHandler<'a, Result<()>> for Download {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        trace!("被唤醒，执行下载任务");
        ctx.set_downloading_pieces(self.piece.0, self.piece.1);
        let _ = ctx.request_block(self.piece.0).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct Exit {
    pub reason: ExitReason
}
impl<'a> CommandHandler<'a, Result<()>> for Exit {
    type Target = &'a mut Peer;

    /// 什么都不需要做，在 peer 中会处理的
    async fn handle(self, _ctx: Self::Target) -> Result<()> {
        unimplemented!()
    }
}
