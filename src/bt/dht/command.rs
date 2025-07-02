use std::net::SocketAddr;
use crate::command_system;
use crate::dht::DHT;
use anyhow::Result;

command_system! {
    ctx: DHT,
    Command {
        Spread,
        PeersScan,
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
        ctx.spread(self.addr).await
    }
}

/// 扫描 DHT 网络中的节点，更新路由表
#[derive(Debug)]
pub struct PeersScan;
impl<'a> CommandHandler<'a, Result<()>> for PeersScan {
    type Target = &'a mut DHT;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.peers_scan().await;
        Ok(())
    }
}
