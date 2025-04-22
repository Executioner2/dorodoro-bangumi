pub mod command;

use crate::core::config::Config;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::GASKET_PREFIX;
use crate::emitter::constant::PEER_PREFIX;
use crate::peer::Peer;
use crate::peer_manager::PeerManagerContext;
use crate::runtime::Runnable;
use crate::torrent::TorrentArc;
use crate::tracker::Tracker;
use crate::{bt, tracker};
use ahash::{AHashSet, RandomState};
use bytes::BytesMut;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
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
        let mut peers = self.peers.lock().await;
        if let Some(mut peer) = peers.remove(&peer_no) {
            if reason == ExitReason::NotHasJob {
                peer.join_handle = None;
                self.wait_queue.lock().await.push_back(peer);
            }
        }
        // let mut peers = self.peers.lock().await;
        // if let Some(sender) = self.wait_queue.lock().await.pop_front() {
        //     sender.send(peer::Download.into()).await.unwrap();
        // }
        // peers.remove(&peer_id);
        // if peers.is_empty() {
        //     trace!("没有 peer 了，强制刷新一波");
        //     drop(peers);
        //     self.force_flush_announce().await;
        // }
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

    // async fn force_flush_announce(&self) {
    //     let peers = tracker::discover_peer(
    //         self.torrent.clone(),
    //         self.download.load(Ordering::Relaxed),
    //         self.upload.load(Ordering::Relaxed),
    //         self.listener_port,
    //         Event::None,
    //     )
    //     .await;
    //     let uh = self.unable_host.lock().await;
    //     let mut host = self.hosts.lock().await;
    //     let peers_map = self.peers.lock().await;
    //     let set = peers_map
    //         .values()
    //         .map(|p| p.host.clone())
    //         .collect::<HashSet<Host>>();
    //     peers
    //         .into_iter()
    //         .filter(|p| !uh.contains(p) && !set.contains(p))
    //         .for_each(|p| {
    //             host.push(p);
    //         });
    //     if self
    //         .spm
    //         .send(FlushAnnounce(self.torrent.info_hash.clone()).into())
    //         .await
    //         .is_err()
    //     {
    //         error!("通知刷新 annouce 失败");
    //     }
    // }
    //
    // /// 刷新 tracker 的 announce 信息
    // async fn run(self) {
    //     loop {
    //         tokio::select! {
    //             _ = self.cancel_token.cancelled() => {
    //                 break;
    //             }
    //             _ = tokio::time::sleep(std::time::Duration::from_secs(60 * 15)) => {
    //                 trace!("到时间刷新annoucne");
    //                 self.force_flush_announce().await;
    //             }
    //         }
    //     }
    // }
}

/// 同一个任务的peer交给一个垫片来管理，垫片对peer进行分块下载任务的分配
pub struct Gasket<'a> {
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

    // hosts: Arc<Mutex<Vec<Host>>>,
    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 这个有部分还在下载中，但是为了避免其它 peer 选择重叠，下载的 peer 会把它下载的那个分块标记为 1
    underway_bytefield: Arc<Mutex<BytesMut>>,

    /// 不可用的 host
    unable_host: Arc<Mutex<HashSet<SocketAddr, RandomState>>>,

    /// 可连接，但是没有任务可分配的 peer，
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 命令发射器
    emitter: Emitter,

    /// 全局配置
    config: Config,

    /// trakcer 集合
    trackers: Vec<TrackerPtr<'a>>,
}

struct TrackerPtr<'a> {
    ptr: *const Tracker<'a>,
}

unsafe impl<'a> Send for TrackerPtr<'a> {}
unsafe impl<'a> Sync for TrackerPtr<'a> {}

impl<'a> Gasket<'a> {
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

        let trackers = instance_tracker(peer_id.clone(), torrent.clone());

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
            unable_host: Arc::new(Mutex::new(HashSet::default())),
            wait_queue: Arc::new(Mutex::new(VecDeque::new())),
            emitter,
            config,
            trackers,
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
            peer_id: self.peer_id.clone()
        }
    }

    async fn start_peer(&self, addr: SocketAddr) {
        let peer_no = self.peer_no_count.fetch_add(1, Ordering::Acquire);
        let context = self.get_context();
        let mut emmiter = Emitter::new();
        let transfer_id = self.get_transfer_id();
        if let Some(send) = self.emitter.get(&transfer_id).await {
            emmiter.register(transfer_id, send.clone()).await.unwrap();
        }

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
                Err(_e) => {
                    // error!("启动 peer 失败，peer_id: {}\thost: {:?}\terror: {}", peer_id, host, e)
                    self.unable_host.lock().await.insert(addr);
                }
            }
        } else {
            // warn!("peer_id: {}\t连不上这个host: {:?}", peer_id, host);
            self.unable_host.lock().await.insert(addr);
        }
    }

    async fn shutdown(self) {
        for (_, peer) in self.peers.lock().await.iter_mut() {
            peer.join_handle.as_mut().map(async |handle| handle.await);
        }
    }

    fn get_transfer_id(&self) -> String {
        format!("{}{}", GASKET_PREFIX, self.id)
    }
}

fn instance_tracker<'a>(peer_id: Arc<[u8; 20]>, torrent: TorrentArc) -> Vec<TrackerPtr<'a>> {
    let mut trackers = vec![];
    let info_hash = &torrent.info_hash;
    let root = &torrent.announce;
    let mut visited = AHashSet::new();

    std::iter::once(root)
        .chain(torrent.announce_list.iter().flatten())
        .for_each(|announce| {
            if !visited.contains(announce) {
                if let Some(tracker) = tracker::parse_tracker_host(announce, info_hash, &*peer_id) {
                    let ptr = Box::into_raw(Box::new(tracker));
                    trackers.push(TrackerPtr {
                        ptr: ptr as *const Tracker,
                    })
                }
                visited.insert(announce);
            }
        });

    trackers
}

impl<'a> Runnable for Gasket<'a> {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.config.channel_buffer());
        let transfer_id = self.get_transfer_id();
        self.emitter
            .register(transfer_id.clone(), send)
            .await
            .unwrap();

        trace!("启动 gasket");

        loop {
            tokio::select! {
                _ = self.context.cancel_token.cancelled() => {
                    info!("peer 取消了请求");
                    break;
                },
                _ = TrackerFuture::new(&self.trackers) => {

                },
                _ = recv.recv() => {
                    todo!("")
                }
            }
        }

        self.shutdown().await;
    }
}

/// tracker 定时任务
struct TrackerFuture<'a> {
    trackers: &'a Vec<TrackerPtr<'a>>,
}

impl<'a> TrackerFuture<'a> {
    pub fn new(trackers: &'a Vec<TrackerPtr<'a>>) -> Self {
        Self { trackers }
    }
}

impl<'a> Future for TrackerFuture<'_> {
    type Output = Option<Vec<SocketAddr>>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}
