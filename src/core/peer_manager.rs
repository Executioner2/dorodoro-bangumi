//! Peer Manager

use crate::core::alias::{
    ReceiverPeer, ReceiverPeerManager, SenderPeer, SenderPeerManager, SenderScheduler,
};
use crate::core::command::peer_manager::FlushAnnounce;
use crate::core::command::{CommandHandler, peer_manager};
use crate::core::config::Config;
use crate::core::runtime::Runnable;
use crate::peer::Peer;
use crate::torrent::TorrentArc;
use crate::tracker;
use crate::tracker::{Event, Host, gen_peer_id, HostV4};
use bytes::BytesMut;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{Sender, channel};
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::{error, info, trace};

type PeerId = u64;

struct PeerInfo {
    send: SenderPeer,
    join_handle: JoinHandle<()>,
    host: Host,
}

#[derive(Clone)]
pub struct GasketContext {
    pub torrent: TorrentArc,
    pub download_path: Arc<String>,
    peer_id: Arc<AtomicU64>,
    peers: Arc<Mutex<HashMap<PeerId, PeerInfo>>>,
    cancel_token: CancellationToken,
    download: Arc<AtomicU64>,
    upload: Arc<AtomicU64>,
    hosts: Arc<Mutex<Vec<Host>>>,
    buff_size: usize,
    spm: SenderPeerManager,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 这个有部分还在下载中，但是为了避免其它 peer 选择重叠，下载的 peer 会把它下载的那个分块标记为 1
    pub underway_bytefield: Arc<Mutex<BytesMut>>,

    pub listener_port: u16,

    /// 不可用的 host
    unable_host: Arc<Mutex<HashSet<Host>>>,
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

    pub async fn peer_exit(self, peer_id: PeerId) {
        let mut peers = self.peers.lock().await;
        peers.remove(&peer_id);
        if peers.is_empty() {
            trace!("没有 peer 了，强制刷新一波");
            drop(peers);
            self.force_flush_announce().await;
        }
    }

    /// 申请下载分块
    pub async fn apply_download_piece(&self, peer_id: PeerId, piece_index: usize) {
        // let ub = self.underway_bytefield.lock().await;
        trace!("peer {} 申请下载第 {} 块", peer_id, piece_index);
    }

    /// 有 peer 告诉我们没有分块可以下载了。在这里根据下载情况，决定是否让这个 peer 去抢占别人的任务
    pub async fn report_no_downloadable_piece(&self, peer_id: PeerId) {
        trace!("peer {} 没有可下载的分块了", peer_id);
    }

    /// 上报下载统计信息
    pub async fn report_statistics(
        &self,
        peer_id: PeerId,
        piece_index: u32,
        block_offset: u32,
        block_size: u64,
        avg_rate: u64,
        last_rate: u64,
        is_over: bool,
    ) {
        trace!(
            "peer {} 下载完了第 {} 块的第 {} 个，全部完成了吗? {}。平均速率: {}\t最后一次下载的速率: {}",
            peer_id, piece_index, block_offset, is_over, avg_rate, last_rate
        );
        self.download.fetch_add(block_size, Ordering::Relaxed);
    }

    async fn force_flush_announce(&self) {
        let peers = tracker::discover_peer(
            self.torrent.clone(),
            self.download.load(Ordering::Relaxed),
            self.upload.load(Ordering::Relaxed),
            self.listener_port,
            Event::None,
        )
        .await;
        let uh = self.unable_host.lock().await;
        let mut host = self.hosts.lock().await;
        let peers_map = self.peers.lock().await;
        let set = peers_map.values().map(|p| p.host.clone()).collect::<HashSet<Host>>();
        peers.into_iter().filter(|p| !uh.contains(p) && !set.contains(p)).for_each(|p| {
            host.push(p);
        });
        if self
            .spm
            .send(FlushAnnounce(self.torrent.info_hash.clone()).into())
            .await
            .is_err()
        {
            error!("通知刷新 annouce 失败");
        }
    }

    /// 刷新 tracker 的 announce 信息
    async fn run(self) {
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(60 * 15)) => {
                    trace!("到时间刷新annoucne");
                    self.force_flush_announce().await;
                }
            }
        }
    }
}

/// 同一个任务的peer交给一个垫片来管理，垫片对peer进行分块下载任务的分配
pub struct Gasket {
    channel: (SenderPeer, ReceiverPeer),
    context: GasketContext,
}

impl Gasket {
    pub fn new(
        torrent: TorrentArc,
        spm: SenderPeerManager,
        cancel_token: CancellationToken,
        buff_size: usize,
        hosts: Vec<Host>,
        download_path: Arc<String>,
        listener_port: u16,
    ) -> Self {
        // todo - 计算有哪些分块是下载了的，哪些是还需要下载的
        let min = ((torrent.info.pieces.len() / 20) + 7) / 8;
        let bytefield = Arc::new(Mutex::new(BytesMut::from(vec![0u8; min].as_slice())));
        let underway_bytefield = Arc::new(Mutex::new(BytesMut::from(vec![0u8; min].as_slice())));
        let context = GasketContext {
            torrent,
            download_path,
            spm,
            peer_id: Arc::new(AtomicU64::new(0)),
            peers: Arc::new(Mutex::new(HashMap::new())),
            cancel_token,
            download: Arc::new(AtomicU64::new(0)),
            upload: Arc::new(AtomicU64::new(0)),
            hosts: Arc::new(Mutex::new(hosts)),
            buff_size,
            bytefield,
            underway_bytefield,
            listener_port,
            unable_host: Arc::new(Mutex::new(HashSet::new())),
        };
        Self {
            channel: channel(buff_size),
            context,
        }
    }

    async fn shutdown(&mut self) {
        for (_, peer) in self.context.peers.lock().await.iter_mut() {
            let join_handle = &mut peer.join_handle;
            join_handle.await.unwrap();
        }
    }

    fn get_context(&self) -> GasketContext {
        self.context.clone()
    }

    pub async fn start_peer(&self) {
        let mut hosts = self.context.hosts.lock().await;
        // hosts.clear();
        // hosts.push(Host::from(([192, 168, 2, 177], 3115)));
        trace!("一共有[{}]个host", hosts.len());

        async fn start_peer(
            peer_id: PeerId,
            host: Host,
            context: GasketContext,
            buff_size: usize,
            peers: Arc<Mutex<HashMap<PeerId, PeerInfo>>>,
            unable_host: Arc<Mutex<HashSet<Host>>>,
        ) {
            if let Some(mut peer) =
                Peer::new(peer_id, gen_peer_id(), host.clone(), context, buff_size).await
            {
                match peer.start().await {
                    Ok(send) => {
                        let join_handle = tokio::spawn(peer.run());
                        peers.lock().await.insert(
                            peer_id,
                            PeerInfo {
                                send,
                                join_handle,
                                host,
                            },
                        );
                    }
                    Err(_e) => {
                        // error!("启动 peer 失败，peer_id: {}\thost: {:?}\terror: {}", peer_id, host, e)
                        unable_host.lock().await.insert(host);
                    }
                }
            } else {
                // warn!("peer_id: {}\t连不上这个host: {:?}", peer_id, host);
                unable_host.lock().await.insert(host);
            }
        }

        while let Some(host) = hosts.pop() {
            let peer_id = self.context.peer_id.fetch_add(1, Ordering::Acquire);
            let context = self.context.clone();
            let buff_size = self.context.buff_size;
            let peers = self.context.peers.clone();
            let unable_host = self.context.unable_host.clone();
            tokio::spawn(start_peer(
                peer_id,
                host,
                context,
                buff_size,
                peers,
                unable_host,
            ));
        }
    }

    pub async fn start(&mut self) -> Result<[u8; 20], std::io::Error> {
        info!("启动垫片");
        self.start_peer().await;
        tokio::spawn(self.get_context().run()).await?;
        Ok(self.context.torrent.info_hash.clone())
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
