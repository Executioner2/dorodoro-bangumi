pub mod command;

use crate::bt;
use crate::command::CommandHandler;
use crate::core::config::Config;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::GASKET_PREFIX;
use crate::emitter::constant::PEER_PREFIX;
use crate::peer::Peer;
use crate::peer_manager::PeerManagerContext;
use crate::peer_manager::gasket::command::Command;
use crate::runtime::Runnable;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};
use ahash::RandomState;
use bytes::BytesMut;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::sync::mpsc::channel;
use tokio::task::JoinHandle;
use tokio_util::sync::WaitForCancellationFuture;
use tracing::{info, trace};

#[derive(PartialEq, Debug)]
pub enum ExitReason {
    /// 正常退出
    Normal,

    /// 没有任务的退出
    NotHasJob,

    /// 异常退出
    Exception,
}

struct PeerInfo {
    peer_no: u64,
    join_handle: Option<JoinHandle<()>>,
    addr: SocketAddr,
}

impl PeerInfo {
    fn get_transfer_id(&self) -> String {
        format!("{}{}", PEER_PREFIX, self.peer_no)
    }
}

#[derive(Clone)]
pub struct GasketContext {
    peers: Arc<Mutex<HashMap<u64, PeerInfo, RandomState>>>,
    context: PeerManagerContext,
    bytefield: Arc<Mutex<BytesMut>>,
    underway_bytefield: Arc<Mutex<BytesMut>>,
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,
    emitter: Emitter,
    download: Arc<AtomicU64>,
    torrent: TorrentArc,
    download_path: Arc<String>,
    peer_id: Arc<[u8; 20]>,
    unstart_host: Arc<Mutex<HashSet<SocketAddr, RandomState>>>,
}

impl GasketContext {
    pub fn cancel_token(&self) -> WaitForCancellationFuture {
        self.context.cancel_token.cancelled()
    }

    pub fn peer_id(&self) -> &[u8; 20] {
        self.peer_id.as_ref()
    }

    pub fn torrent(&self) -> TorrentArc {
        self.torrent.clone()
    }

    pub fn config(&self) -> Config {
        self.context.config.clone()
    }

    pub fn download_path(&self) -> &str {
        self.download_path.as_str()
    }

    pub async fn bytefield(&self) -> BytesMut {
        self.bytefield.lock().await.clone()
    }

    pub async fn underway_bytefield(&self) -> BytesMut {
        self.underway_bytefield.lock().await.clone()
    }

    pub async fn peer_exit(&mut self, peer_no: u64, reason: ExitReason) {
        let peer = self.peers.lock().await.remove(&peer_no);
        if peer.is_none() {
            return;
        }
        let mut peer = peer.unwrap();
        if reason == ExitReason::NotHasJob {
            peer.join_handle = None;
            self.wait_queue.lock().await.push_back(peer);
        } else {
            self.unstart_host.lock().await.remove(&peer.addr);
        }
    }

    /// 申请下载分块
    pub async fn apply_download_piece(&self, peer_no: u64, idx: usize, offset: u8) -> bool {
        trace!("peer {} 申请下载分块", peer_no);
        let mut ub = self.underway_bytefield.lock().await;

        if idx >= ub.len() {
            return false;
        }

        let flag = ub[idx] & offset == 0;
        ub.as_mut().get_mut(idx).map(|x| *x |= offset);
        flag
    }

    /// 归还分块下载
    pub async fn give_back_download_piece(&self, peer_no: u64, piece_index: u32) {
        trace!("peer {} 归还下载分块", peer_no);
        let mut ub = self.underway_bytefield.lock().await;
        let idx = (piece_index / 8) as usize;
        let offset = 1 << (piece_index % 8);

        if idx >= ub.len() {
            return;
        }

        ub.as_mut().get_mut(idx).map(|x| *x &= !offset);
    }

    /// 有 peer 告诉我们没有分块可以下载了。在这里根据下载情况，决定是否让这个 peer 去抢占别人的任务
    pub async fn report_no_downloadable_piece(&self, peer_no: u64) {
        trace!("peer {} 没有可下载的分块了", peer_no);
        let peers = self.peers.lock().await;

        // 通知 peer 结束运行
        if let Some(peer) = peers.get(&peer_no) {
            self.emitter
                .send(&peer.get_transfer_id(), bt::peer::command::Exit.into())
                .await
                .unwrap();
        }
    }

    /// 上报下载统计信息
    pub async fn report_statistics(
        &self,
        peer_no: u64,
        piece_index: u32,
        block_offset: u32,
        block_size: u64,
        avg_rate: u64,
        last_rate: u64,
        is_over: bool,
    ) {
        trace!(
            "peer {} 下载完了第 {} 块的第 {} 个，全部完成了吗? {}。平均速率: {}\t最后一次下载的速率: {}",
            peer_no, piece_index, block_offset, is_over, avg_rate, last_rate
        );
        self.download.fetch_add(block_size, Ordering::Relaxed);
    }
}

/// 同一个任务的peer交给一个垫片来管理，垫片对peer进行分块下载任务的分配
pub struct Gasket {
    /// gasket 的 id
    id: u64,

    /// peer manager context
    context: PeerManagerContext,

    /// peer id
    peer_id: Arc<[u8; 20]>,

    /// 种子信息
    torrent: TorrentArc,

    /// 下载路径
    download_path: Arc<String>,

    /// peer_id 自增计数
    peer_no_count: Arc<AtomicU64>,

    /// 正在运行中的 peer
    peers: Arc<Mutex<HashMap<u64, PeerInfo, RandomState>>>,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 这个有部分还在下载中，但是为了避免其它 peer 选择重叠，下载的 peer 会把它下载的那个分块标记为 1
    underway_bytefield: Arc<Mutex<BytesMut>>,

    /// 不可 start peer 的 host
    unstart_host: Arc<Mutex<HashSet<SocketAddr, RandomState>>>,

    /// 可连接，但是没有任务可分配的 peer，
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 命令发射器
    emitter: Emitter,

    /// 全局配置
    config: Config,
}

impl Gasket {
    pub fn new(
        id: u64,
        torrent: TorrentArc,
        context: PeerManagerContext,
        peer_id: Arc<[u8; 20]>,
        download_path: String,
        download: u64,
        uploaded: u64,
        emitter: Emitter,
        config: Config,
    ) -> Self {
        // todo - 计算有哪些分块是下载了的，哪些是还需要下载的
        let min = ((torrent.info.pieces.len() / 20) + 7) / 8;
        let bytefield = Arc::new(Mutex::new(BytesMut::from(vec![0u8; min].as_slice())));
        let underway_bytefield = Arc::new(Mutex::new(BytesMut::from(vec![0u8; min].as_slice())));

        Self {
            id,
            context,
            peer_id,
            torrent,
            peer_no_count: Arc::new(AtomicU64::new(0)),
            peers: Arc::new(Mutex::new(HashMap::default())),
            download_path: Arc::new(download_path),
            download: Arc::new(AtomicU64::new(download)),
            uploaded: Arc::new(AtomicU64::new(uploaded)),
            bytefield,
            underway_bytefield,
            unstart_host: Arc::new(Mutex::new(HashSet::default())),
            wait_queue: Arc::new(Mutex::new(VecDeque::new())),
            emitter,
            config,
        }
    }

    fn get_context(&self) -> GasketContext {
        GasketContext {
            peers: self.peers.clone(),
            context: self.context.clone(),
            bytefield: self.bytefield.clone(),
            underway_bytefield: self.underway_bytefield.clone(),
            wait_queue: self.wait_queue.clone(),
            emitter: self.emitter.clone(),
            download: self.download.clone(),
            torrent: self.torrent.clone(),
            download_path: self.download_path.clone(),
            peer_id: self.peer_id.clone(),
            unstart_host: self.unstart_host.clone(),
        }
    }

    async fn start_peer(&self, addr: SocketAddr) {
        let peer_no = self.peer_no_count.fetch_add(1, Ordering::Acquire);
        let context = self.get_context();
        let mut emmiter = Emitter::new();
        let transfer_id = self.get_transfer_id();
        self.emitter.get(&transfer_id).await.map(async |send| {
            emmiter.register(transfer_id, send).await.unwrap();
        });

        if let Some(mut peer) = Peer::new(peer_no, addr, context, emmiter).await {
            match peer.start().await {
                Ok(_) => {
                    let join_handle = tokio::spawn(peer.run());
                    self.peers.lock().await.insert(
                        peer_no,
                        PeerInfo {
                            peer_no,
                            join_handle: Some(join_handle),
                            addr,
                        },
                    );
                }
                Err(_) => {}
            }
        }

        // 成功与否都需要加入到 unstart 集合中，避免重复链接
        // todo - 需要改，避免多个资源同时下载时，有某个 peer 多个资源中都被发现了
        self.unstart_host.lock().await.insert(addr);
    }

    async fn shutdown(self) {
        for (_, peer) in self.peers.lock().await.iter_mut() {
            peer.join_handle.as_mut().map(async |handle| handle.await);
        }
    }

    fn get_transfer_id(&self) -> String {
        format!("{}{}", GASKET_PREFIX, self.id)
    }

    async fn start_tracker(&self) -> JoinHandle<()> {
        let transfer_id = self.get_transfer_id();
        trace!("启动 {} 的 tracker", transfer_id);
        let info = AnnounceInfo::new(
            self.download.clone(),
            self.uploaded.clone(),
            self.torrent.info.length,
            self.config.tcp_server_addr().port(),
        );
        let tracker = Tracker::new(
            self.torrent.clone(),
            self.peer_id.clone(),
            info,
            self.emitter.clone(),
            self.config.clone(),
            transfer_id,
        );
        tokio::spawn(tracker.run())
    }
}

impl Runnable for Gasket {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.config.channel_buffer());
        let transfer_id = self.get_transfer_id();
        self.emitter
            .register(transfer_id.clone(), send.clone())
            .await
            .unwrap();

        trace!("启动 gasket");
        let tracker_handle = self.start_tracker().await;

        loop {
            tokio::select! {
                _ = self.context.cancel_token.cancelled() => {
                    info!("peer 取消了请求");
                    break;
                },
                cmd = recv.recv() => {
                    if let Some(cmd) = cmd {
                        let cmd: Command = cmd.instance();
                        cmd.handle(&self).await;
                    }
                }
            }
        }

        tracker_handle.await.unwrap();
        self.shutdown().await;
    }
}
