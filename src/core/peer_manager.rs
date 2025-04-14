//! Peer Manager

use crate::core::command::{CommandHandler, peer_manager};
use crate::core::config::Config;
use crate::core::runtime::Runnable;
use std::collections::HashMap;
use tokio::sync::mpsc::{Sender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};
use crate::core::alias::{ReceiverPeerManager, SenderPeerManager, SenderScheduler};

type PeerId = u64;

struct PeerInfo {
    id: PeerId,
    send: SenderPeerManager,
    join_handle: JoinHandle<()>,
}

pub struct PeerManager {
    send: SenderScheduler,
    cancel_token: CancellationToken,
    peers: HashMap<PeerId, PeerInfo>,
    peer_id: PeerId,
    channel: (SenderPeerManager, ReceiverPeerManager),
    config: Config,
}

impl PeerManager {
    pub fn new(send: SenderScheduler, cancel_token: CancellationToken, config: Config) -> Self {
        let channel = tokio::sync::mpsc::channel(config.channel_buffer());
        Self {
            send,
            cancel_token,
            peers: HashMap::new(),
            peer_id: 0,
            channel,
            config,
        }
    }

    pub fn get_sender(&self) -> Sender<peer_manager::Command> {
        self.channel.0.clone()
    }

    async fn shutdown(mut self) {
        for (_, peer_info) in self.peers.iter_mut() {
            let join_handle = &mut peer_info.join_handle;
            join_handle.await.unwrap();
        }
    }
}

impl Runnable for PeerManager {
    async fn run(mut self) {
        info!("peer manager 已启动");
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                recv = self.channel.1.recv() => {
                    trace!("peer manager 收到了消息: {:?}", recv);
                    if let Some(cmd) = recv {
                        cmd.handle(&mut self).await;
                    }
                }
            }
        }

        info!("等待 peer 退出");
        self.shutdown().await;
        info!("peer manager 已关闭");
    }
}
