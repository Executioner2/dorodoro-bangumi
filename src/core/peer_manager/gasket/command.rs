use crate::command::CommandHandler;
use crate::command_system;
use crate::emitter::transfer::CommandEnum;
use crate::emitter::transfer::TransferPtr;
use crate::peer_manager::gasket::Gasket;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, trace};
use super::error::Result;

command_system! {
    ctx: Gasket,
    Command {
        DiscoverPeerAddr,
        StartWaittingAddr,
        SaveProgress,
    }
}

/// 发现了 peer addr
#[derive(Debug)]
pub struct DiscoverPeerAddr {
    pub peers: Vec<SocketAddr>,
}

impl<'a> CommandHandler<'a, Result<()>> for DiscoverPeerAddr {
    type Target = &'a mut Gasket;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        for addr in self.peers {
            ctx.start_peer(Arc::new(addr)).await
        }
        Ok(())
    }
}

/// 启动一个等待中的地址
#[derive(Debug)]
pub struct StartWaittingAddr;

impl<'a> CommandHandler<'a, Result<()>> for StartWaittingAddr {
    type Target = &'a mut Gasket;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        trace!("从等待队列中唤醒一个");
        // 有个副作用，原本应该先唤醒的 addr，如果遇到当前没有可用的 peer 配额时，会被移
        // 动到最后。
        if let Some(peer) = ctx.pop_wait_peer().await {
            trace!("尝试唤醒 [{}]", peer.addr);
            ctx.start_peer(peer.addr).await
        } else {
            info!("没有 peer 可以用了")
        }
        Ok(())
    }
}

/// 保存进度
#[derive(Debug)]
pub struct SaveProgress;

impl<'a> CommandHandler<'a, Result<()>> for SaveProgress {
    type Target = &'a mut Gasket;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.save_progress().await;
        Ok(())
    }
}
