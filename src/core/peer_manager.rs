//! Peer Manager

use crate::core::command::CommandHandler;
use crate::core::config::Config;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::runtime::Runnable;
use crate::peer_manager::command::Command;
use crate::torrent::TorrentArc;
use crate::tracker;
use ahash::RandomState;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::Mutex;
use tokio::sync::mpsc::channel;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};

pub mod command;
pub mod gasket;

pub struct GasketInfo {
    id: u64,
    torrent: TorrentArc,
    peer_id: Arc<[u8; 20]>,
    join_handle: JoinHandle<()>,
}

#[derive(Clone)]
pub struct PeerManagerContext {
    pub config: Config,
    pub cancel_token: CancellationToken,
    pub emitter: Emitter,
    pub gaskets: Arc<Mutex<HashMap<u64, GasketInfo, RandomState>>>,
    pub gasket_id: Arc<AtomicU64>,
    peer_id_pool: Arc<Mutex<HashSet<[u8; 20], RandomState>>>,
}

impl PeerManagerContext {
    pub async fn get_peer_id(&self) -> [u8; 20] {
        for _ in 0..10 {
            let peer_id = tracker::gen_peer_id();
            let mut pool = self.peer_id_pool.lock().await;
            if !pool.contains(&peer_id) {
                pool.insert(peer_id.clone());
                return peer_id;
            }
        }
        panic!("无法再获取新的 peer id");
    }

    pub async fn add_gasket(&self, gasket: GasketInfo) {
        self.gaskets.lock().await.insert(gasket.id, gasket);
    }

    pub async fn remove_gasket(&self, gasket_id: u64) {
        if let Some(gasket) = self.gaskets.lock().await.remove(&gasket_id) {
            self.peer_id_pool.lock().await.remove(&*gasket.peer_id);
        }
    }
}

pub struct PeerManager {
    cancel_token: CancellationToken,
    config: Config,
    gaskets: Arc<Mutex<HashMap<u64, GasketInfo, RandomState>>>,
    emitter: Emitter,
    gasket_id: Arc<AtomicU64>,
    peer_id_pool: Arc<Mutex<HashSet<[u8; 20], RandomState>>>,
}

impl PeerManager {
    pub fn new(config: Config, cancel_token: CancellationToken, emitter: Emitter) -> Self {
        Self {
            cancel_token,
            config,
            gaskets: Arc::new(Mutex::new(HashMap::default())),
            emitter,
            gasket_id: Arc::new(AtomicU64::new(0)),
            peer_id_pool: Arc::new(Mutex::new(HashSet::default())),
        }
    }

    fn get_context(&self) -> PeerManagerContext {
        PeerManagerContext {
            config: self.config.clone(),
            cancel_token: self.cancel_token.clone(),
            emitter: self.emitter.clone(),
            gaskets: self.gaskets.clone(),
            gasket_id: self.gasket_id.clone(),
            peer_id_pool: self.peer_id_pool.clone(),
        }
    }

    async fn shutdown(self) {
        self.emitter.remove(PEER_MANAGER).await.unwrap();
        for (_id, gasket_info) in self.gaskets.lock().await.iter_mut() {
            let handle = &mut gasket_info.join_handle;
            handle.await.unwrap()
        }
    }
}

impl Runnable for PeerManager {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.config.channel_buffer());
        self.emitter.register(PEER_MANAGER, send).await.unwrap();

        info!("peer manager 已启动");
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                recv = recv.recv() => {
                    trace!("peer manager 收到了消息: {:?}", recv);
                    if let Some(cmd) = recv {
                        let cmd: Command = cmd.instance();
                        cmd.handle(self.get_context()).await;
                    }
                }
            }
        }

        info!("等待 peer 退出");
        self.shutdown().await;
        info!("peer manager 已关闭");
    }
}
