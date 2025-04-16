//! Peer Manager

use crate::core::alias::{
    ReceiverPeer, ReceiverPeerManager, SenderPeer, SenderPeerManager, SenderScheduler,
};
use crate::core::command::{CommandHandler, peer_manager};
use crate::core::config::Config;
use crate::core::runtime::Runnable;
use crate::torrent::TorrentArc;
use crate::tracker::{gen_peer_id, Host};
use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use bytes::{BytesMut};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{Sender, channel};
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::{info, trace, warn};
use crate::peer::Peer;

type PeerId = u64;

struct PeerInfo {
    id: PeerId,
    send: SenderPeer,
    join_handle: JoinHandle<()>,
}

/// 同一个任务的peer交给一个垫片来管理，垫片对peer进行分块下载任务的分配
pub struct Gasket {
    torrent: TorrentArc,
    peer_id: PeerId,
    spm: SenderPeerManager,
    peers: Arc<Mutex<HashMap<PeerId, PeerInfo>>>,
    channel: (SenderPeer, ReceiverPeer),
    cancel_token: CancellationToken,
    hosts: Vec<Host>,
    buff_size: usize,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 这个有部分还在下载中，但是为了避免其它 peer 选择重叠，下载的 peer 会把它下载的那个分块标记为 1
    underway_bytefield: Arc<Mutex<BytesMut>>,
}

pub struct GasketContext {
    bytefield: Arc<Mutex<BytesMut>>,
    pub underway_bytefield: Arc<Mutex<BytesMut>>,
    pub torrent: TorrentArc,
    peers: Arc<Mutex<HashMap<PeerId, PeerInfo>>>,
    cancel_token: CancellationToken,
}

impl GasketContext {
    pub fn cancel_token(&self) -> WaitForCancellationFuture {
        self.cancel_token.cancelled()
    }

    pub async fn bytefield(&self) -> BytesMut {
        self.bytefield.lock().await.clone()
    }

    pub async fn underway_bytefield(&self) -> BytesMut {
        self.underway_bytefield.lock().await.clone()
    }

    pub async fn peer_exit(&self, peer_id: PeerId) {
        self.peers.lock().await.remove(&peer_id);
    }

    /// 申请下载分块
    pub async fn apply_download_piece(&self, peer_id: PeerId, piece_index: usize) {
        let ub = self.underway_bytefield.lock().await;

    }
}

impl Gasket {
    pub fn new(
        torrent: TorrentArc,
        spm: SenderPeerManager,
        cancel_token: CancellationToken,
        buff_size: usize,
        hosts: Vec<Host>,
    ) -> Self {
        // todo - 计算有哪些分块是下载了的，哪些是还需要下载的
        let min = (torrent.info.pieces.len() / 20) + 7 / 8;
        let bytefield = Arc::new(Mutex::new(BytesMut::from(vec![0u8; min].as_slice())));
        let underway_bytefield = Arc::new(Mutex::new(BytesMut::from(vec![0u8; min].as_slice())));
        Self {
            torrent,
            peer_id: 0,
            spm,
            peers: Arc::new(Mutex::new(HashMap::new())),
            cancel_token,
            channel: channel(buff_size),
            hosts,
            buff_size,
            bytefield,
            underway_bytefield
        }
    }

    async fn shutdown(&mut self) {
        for (_, peer) in self.peers.lock().await.iter_mut() {
            let join_handle = &mut peer.join_handle;
            join_handle.await.unwrap();
        }
    }

    fn get_context(&self) -> GasketContext {
        GasketContext {
            bytefield: self.bytefield.clone(),
            underway_bytefield: self.underway_bytefield.clone(),
            torrent: self.torrent.clone(),
            peers: self.peers.clone(),
            cancel_token: self.cancel_token.clone(),
        }
    }

    pub async fn start(&mut self) -> Result<[u8; 20], std::io::Error> {
        info!("启动垫片");
        let mut hosts = vec![];
        mem::swap(self.hosts.as_mut(), &mut hosts);
        for host in hosts.into_iter() {
            if let Some(peer) = Peer::new(
                self.peer_id,
                gen_peer_id(),
                host.clone(),
                self.get_context(),
                self.buff_size,
            ).await {
                if let Ok((join_handle, send)) = peer.start().await {
                    self.peers.lock().await.insert(self.peer_id, PeerInfo {
                        id: self.peer_id,
                        send,
                        join_handle,
                    });
                    self.peer_id += 1;
                    continue;
                }
            }
            warn!("连不上这个host: {:?}", host)
        }
        Ok(self.torrent.info_hash.clone())
    }
}

pub struct PeerManagerContext {
    pub gaskets: Arc<Mutex<HashMap<[u8; 20], Gasket>>>,
    pub spm: SenderPeerManager,
    pub config: Config,
    pub cancle_token: CancellationToken,
}

pub struct PeerManager {
    send: SenderScheduler,
    cancel_token: CancellationToken,
    channel: (SenderPeerManager, ReceiverPeerManager),
    config: Config,
    gaskets: Arc<Mutex<HashMap<[u8; 20], Gasket>>>,
}

impl PeerManager {
    pub fn new(send: SenderScheduler, cancel_token: CancellationToken, config: Config) -> Self {
        let channel = channel(config.channel_buffer());
        Self {
            send,
            cancel_token,
            channel,
            config,
            gaskets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_sender(&self) -> Sender<peer_manager::Command> {
        self.channel.0.clone()
    }

    fn get_context(&self) -> PeerManagerContext {
        PeerManagerContext {
            gaskets: self.gaskets.clone(),
            spm: self.channel.0.clone(),
            config: self.config.clone(),
            cancle_token: self.cancel_token.clone(),
        }
    }

    async fn shutdown(self) {
        for (_, gasket) in self.gaskets.lock().await.iter_mut() {
            gasket.shutdown().await;
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
