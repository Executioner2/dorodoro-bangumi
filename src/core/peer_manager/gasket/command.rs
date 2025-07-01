use core::fmt::{Display, Formatter};
use crate::command_system;
use crate::peer_manager::gasket::{Gasket, PeerInfo};
use std::net::SocketAddr;
use tracing::{debug, info, trace};
use crate::mapper::torrent::TorrentStatus;
use anyhow::Result;

command_system! {
    ctx: Gasket,
    Command {
        DiscoverPeerAddr,
        StartWaittingAddr,
        SaveProgress,
        StartTempPeer,
    }
}

#[derive(Debug)]
pub enum PeerSource {
    Tracker,
    DHT,
}

impl Display for PeerSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            PeerSource::Tracker => write!(f, "Tracker"),
            PeerSource::DHT => write!(f, "DHT"),
        }
    }
}

/// 发现了 peer addr
#[derive(Debug)]
pub struct DiscoverPeerAddr {
    pub peers: Vec<SocketAddr>,
    pub source: PeerSource,
}

impl<'a> CommandHandler<'a, Result<()>> for DiscoverPeerAddr {
    type Target = &'a mut Gasket;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        info!("通过 [{}] 的方式发现了 peer addr: {:?}", self.source, self.peers);
        for addr in self.peers {
            ctx.start_peer(addr).await
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
            debug!("尝试唤醒 [{}]", peer.addr);
            ctx.start_wait_peer(peer).await
        } else {
            info!("没有 peer 可以用了")
        }
        Ok(())
    }
}

/// 保存进度
#[derive(Debug)]
pub struct SaveProgress {
    pub(super) status: Option<TorrentStatus>
}

impl<'a> CommandHandler<'a, Result<()>> for SaveProgress {
    type Target = &'a mut Gasket;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.save_progress(self.status).await;
        Ok(())
    }
}

/// 启动一个临时 peer
#[derive(Debug)]
pub struct StartTempPeer {
    pub peer_info: PeerInfo,
}

impl<'a> CommandHandler<'a, Result<()>> for StartTempPeer {
    type Target = &'a mut Gasket;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        trace!("启动临时 peer [{}]", self.peer_info.addr);
        ctx.start_temp_peer(self.peer_info).await;
        Ok(())
    }
}
