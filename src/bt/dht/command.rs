use std::net::SocketAddr;
use crate::emitter::transfer::{TransferPtr, CommandEnum};
use super::error::Result;
use crate::command::CommandHandler;
use crate::command_system;
use crate::dht::DHT;

command_system! {
    ctx: DHT,
    Command {
        Spread,
    }
}

/// 发现了一个支持 DHT 的节点，向其发送 DHT 更新
#[derive(Debug)]
pub struct Spread{
    pub addr: SocketAddr,
}
impl<'a> CommandHandler<'a, Result<()>> for Spread {
    type Target = &'a mut DHT;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.spread(self.addr).await;
        Ok(())
    }
}
