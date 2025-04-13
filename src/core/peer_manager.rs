//! Peer Manager

use crate::core::command::{CommandHandler, peer_manager, scheduler};
use crate::core::config::Config;
use crate::core::runtime::Runnable;
use std::collections::HashMap;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};

/// 发送给 scheduler 的
type SenderScheduler = Sender<scheduler::Command>;

/// 其它组件向 peer manager 发送的
type SenderPeerManager = Sender<peer_manager::Command>;

/// 接收发送给 peer manager 的
type ReceiverPeerManager = Receiver<peer_manager::Command>;

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

    async fn await_peers_exit(&mut self) {
        for (_, peer_info) in self.peers.iter_mut() {
            let join_handle = &mut peer_info.join_handle;
            join_handle.await.unwrap();
        }
    }

    async fn handle_command(&mut self, cmd: peer_manager::Command) {
        cmd.handle(self).await;
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
                        self.handle_command(recv).await;
                    }
                }
            }
        }
        info!("peer manager 已关闭");
    }
}
