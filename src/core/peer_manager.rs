//! Peer Manager

use crate::core::config::Config;
use crate::core::runtime::Runnable;
use crate::core::{command, runtime};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};

/// 发送给 scheduler 的
type SenderScheduler = Arc<Sender<command::scheduler::Command>>;

/// 发送给 peer manager 的
type SenderPeer = Arc<Sender<command::peer::Command>>;

/// 接收来自 peer 的
type ReceiverPeer = Receiver<command::peer::Command>;

type PeerId = u64;

struct PeerInfo {
    id: PeerId,
    send: SenderPeer,
    join_handle: JoinHandle<()>,
}

pub struct PeerManager {
    send: SenderScheduler,
    cancel_token: CancellationToken,
    peers: HashMap<PeerId, PeerInfo>,
    peer_id: PeerId,
    channel: (SenderPeer, ReceiverPeer),
    config: Config,
}

impl PeerManager {
    pub fn new(send: SenderScheduler, cancel_token: CancellationToken, config: Config) -> Self {
        let (sender_peer, receiver_peer) = tokio::sync::mpsc::channel(config.channel_buffer());
        let send_peer = Arc::new(sender_peer);
        Self {
            send,
            cancel_token,
            peers: HashMap::new(),
            peer_id: 0,
            channel: (send_peer.clone(), receiver_peer),
            config,
        }
    }

    pub fn get_sender(&self) -> SenderPeer {
        self.channel.0.clone()
    }

    async fn await_peers_exit(&mut self) {
        for (_, peer_info) in self.peers.iter_mut() {
            let join_handle = &mut peer_info.join_handle;
            join_handle.await.unwrap();
        }
    }

    fn handle_command(&mut self, cmd: command::peer::Command) {
        match cmd {}
    }
}

impl Runnable for PeerManager {
    async fn run(mut self) {
        info!("peer manager 已启动");
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    info!("等待 peer 退出");
                    self.await_peers_exit().await;
                    break;
                }
                recv = self.channel.1.recv() => {
                    trace!("peer manager 收到了消息: {:?}", recv);
                    if let Some(recv) = recv {
                        self.handle_command(recv);
                    }
                }
            }
        }
        info!("peer manager 已关闭");
    }
}
