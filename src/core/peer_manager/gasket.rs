pub mod command;
mod error;

use crate::collection::FixedQueue;
use crate::command::CommandHandler;
use crate::core::config::Config;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::GASKET_PREFIX;
use crate::peer;
use crate::peer::Peer;
use crate::peer_manager::PeerManagerContext;
use crate::peer_manager::gasket::command::{Command, StartWaittingAddr};
use crate::runtime::Runnable;
use crate::store::Store;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};
use ahash::HashMap;
use bytes::BytesMut;
use dashmap::{DashMap, DashSet};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::mpsc::channel;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::{error, info, trace};

#[derive(PartialEq, Debug)]
pub enum ExitReason {
    /// 正常退出
    Normal,

    /// 没有任务的退出
    NotHasJob,

    /// 异常退出
    Exception,
}

#[derive(Eq, PartialEq)]
enum PieceStatus {
    /// 进行中
    Ing,

    /// 暂停，未开始也用这个标记
    Pause(u32),

    /// 已完成
    Finished,
}

struct PeerInfo {
    /// 异步任务句柄
    join_handle: Option<JoinHandle<()>>,

    /// 通信地址
    addr: SocketAddr,
}

impl PeerInfo {
    fn new(addr: SocketAddr, join_handle: JoinHandle<()>) -> Self {
        Self {
            join_handle: Some(join_handle),
            addr,
        }
    }
}

#[derive(Clone)]
pub struct GasketContext {
    peers: Arc<DashMap<u64, PeerInfo>>,
    context: PeerManagerContext,
    bytefield: Arc<BytesMut>,
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,
    emitter: Emitter,
    download: Arc<AtomicU64>,
    torrent: TorrentArc,
    download_path: Arc<PathBuf>,
    peer_id: Arc<[u8; 20]>,
    unstart_host: Arc<DashSet<SocketAddr>>,
    gasket_transfer_id: String,
    peer_transfer_speed: Arc<DashMap<u64, u64>>,
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

    pub fn download_path(&self) -> &PathBuf {
        &self.download_path
    }

    pub fn bytefield(&self) -> Arc<BytesMut> {
        self.bytefield.clone()
    }

    pub async fn peer_exit(&mut self, peer_no: u64, reason: ExitReason) {
        trace!("peer_no [{}] 退出了，退出原因: {:?}", peer_no, reason);
        let peer = self.peers.remove(&peer_no);
        trace!("成功移除 peer_no [{}]", peer_no);
        if peer.is_none() {
            return;
        }

        let mut peer = peer.unwrap().1;
        if reason == ExitReason::NotHasJob {
            peer.join_handle = None;
            self.wait_queue.lock().await.push_back(peer);
            return;
        }

        self.unstart_host.remove(&peer.addr);
        trace!("将这个地址从不可用host中移除了");

        // 从等待队列中唤醒一个
        if let Err(e) = self
            .emitter
            .send(&self.gasket_transfer_id, StartWaittingAddr.into())
            .await
        {
            error!("唤醒等待队列中的失败！{}", e);
        }

        trace!("发送了唤醒消息");
    }

    /// 申请下载分块
    pub fn apply_download_piece(&self, peer_no: u64, piece_index: u32) -> Option<u32> {
        trace!("peer {} 申请下载分块: {}", peer_no, piece_index);
        let mut res = None;
        self.underway_bytefield
            .entry(piece_index)
            .and_modify(|value| {
                if let PieceStatus::Pause(block_offset) = value {
                    res = Some(*block_offset);
                    *value = PieceStatus::Ing
                }
            })
            .or_insert_with(|| {
                res = Some(0);
                PieceStatus::Ing
            });
        res
    }

    /// 归还分块下载
    pub fn give_back_download_piece(&self, peer_no: u64, give_back: HashMap<u32, u32>) {
        trace!("peer {} 归还下载分块", peer_no);
        for (piece_index, block_offset) in give_back {
            self.underway_bytefield
                .insert(piece_index, PieceStatus::Pause(block_offset));
        }
    }

    /// 有 peer 告诉我们没有分块可以下载了。在这里根据下载情况，决定是否让这个 peer 去抢占别人的任务
    pub async fn report_no_downloadable_piece(&self, peer_no: u64) {
        trace!("peer {} 没有可下载的分块了", peer_no);

        // 通知 peer 结束运行
        if let Some(_) = self.peers.get(&peer_no) {
            let tid = Peer::get_transfer_id(peer_no);
            let cmd = peer::command::Exit {
                reason: ExitReason::NotHasJob,
            }
            .into();
            self.emitter.send(&tid, cmd).await.unwrap();
        }
    }

    /// 上报下载量信息
    pub fn reported_download(&self, block_size: u64) {
        self.download.fetch_add(block_size, Ordering::Relaxed);
    }
    
    /// 上报分块下载完成
    pub fn reported_piece_finished(&self, piece_index: u32) {
        self.underway_bytefield
            .get_mut(&piece_index)
            .map(|mut status| *status.value_mut() = PieceStatus::Finished);
    }

    /// 上报读取到的数据大小
    pub fn reported_read_size(&self, peer_no: u64, read_size: u64) {
        *self.peer_transfer_speed.entry(peer_no).or_insert(0) += read_size
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
    download_path: Arc<PathBuf>,

    /// peer_id 自增计数
    peer_no_count: Arc<AtomicU64>,

    /// 正在运行中的 peer
    peers: Arc<DashMap<u64, PeerInfo>>,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<BytesMut>,

    /// 这个有部分还在下载中，但是为了避免其它 peer 选择重叠，下载的 peer 会把它下载的那个分块标记为 1
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 不可 start peer 的 host
    unstart_host: Arc<DashSet<SocketAddr>>,

    /// 可连接，但是没有任务可分配的 peer，
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 命令发射器
    emitter: Emitter,

    /// 全局配置
    config: Config,

    /// 存储处理
    store: Store,

    /// peer 传输速率
    peer_transfer_speed: Arc<DashMap<u64, u64>>,
}

impl Gasket {
    pub fn new(
        id: u64,
        torrent: TorrentArc,
        context: PeerManagerContext,
        peer_id: Arc<[u8; 20]>,
        download_path: PathBuf,
        download: u64,
        uploaded: u64,
        emitter: Emitter,
        config: Config,
        store: Store,
    ) -> Self {
        // todo - 计算有哪些分块是下载了的，哪些是还需要下载的
        let min = ((torrent.info.pieces.len() / 20) + 7) / 8;
        let bytefield = Arc::new(BytesMut::from(vec![0u8; min].as_slice()));

        Self {
            id,
            context,
            peer_id,
            torrent,
            peer_no_count: Arc::new(AtomicU64::new(0)),
            peers: Arc::new(DashMap::new()),
            download_path: Arc::new(download_path),
            download: Arc::new(AtomicU64::new(download)),
            uploaded: Arc::new(AtomicU64::new(uploaded)),
            bytefield,
            underway_bytefield: Arc::new(DashMap::new()),
            unstart_host: Arc::new(DashSet::new()),
            wait_queue: Arc::new(Mutex::new(VecDeque::new())),
            emitter,
            config,
            store,
            peer_transfer_speed: Arc::new(DashMap::new()),
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
            gasket_transfer_id: self.get_transfer_id(),
            peer_transfer_speed: self.peer_transfer_speed.clone(),
        }
    }

    async fn start_peer(&self, addr: SocketAddr) {
        let peer_no = self.peer_no_count.fetch_add(1, Ordering::Acquire);
        let context = self.get_context();

        if let Some(mut peer) = Peer::new(
            peer_no,
            addr,
            context,
            self.emitter.clone(),
            self.store.clone(),
        )
        .await
        {
            let peers = self.peers.clone();
            match peer.start().await {
                Ok(_) => {
                    let join_handle = tokio::spawn(peer.run());
                    peers.insert(peer_no, PeerInfo::new(addr, join_handle));
                }
                Err(_) => {}
            }
            // tokio::spawn(async move { // 不开异步的话，可能导致 channel 堆积满了然后崩溃
            //     match peer.start().await {
            //         Ok(_) => {
            //             let join_handle = tokio::spawn(peer.run());
            //             peers.lock().await.insert(
            //                 peer_no,
            //                 PeerInfo {
            //                     join_handle: Some(join_handle),
            //                     addr,
            //                 },
            //             );
            //         }
            //         Err(_) => {}
            //     }
            // });
        }

        // 成功与否都需要加入到 unstart 集合中，避免重复链接
        // todo - 需要改，避免多个资源同时下载时，有某个 peer 多个资源中都被发现了
        self.unstart_host.insert(addr);
    }

    async fn shutdown(self) {
        for mut peer in self.peers.iter_mut() {
            peer.join_handle.as_mut().map(async |handle| handle.await);
        }
    }

    fn get_transfer_id(&self) -> String {
        format!("{}{}", GASKET_PREFIX, self.id)
    }

    fn start_tracker(&self) -> JoinHandle<()> {
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
        self.emitter.register(transfer_id.clone(), send.clone());

        trace!("启动 gasket");
        let tracker_handle = self.start_tracker();
        let speed_handle = tokio::spawn(start_speed_report(
            self.peer_transfer_speed.clone(),
            self.context.cancel_token.clone(),
        ));

        loop {
            tokio::select! {
                _ = self.context.cancel_token.cancelled() => {
                    info!("peer 取消了请求");
                    break;
                }
                cmd = recv.recv() => {
                    if let Some(cmd) = cmd {
                        let cmd: Command = cmd.instance();
                        if let Err(e) = cmd.handle(&mut self).await {
                            error!("处理指令出现错误\t{}", e);
                            break;
                        }
                    }
                }
            }
        }

        speed_handle.await.unwrap();
        tracker_handle.await.unwrap();
        self.shutdown().await;
    }
}

/// 启动速率播报
async fn start_speed_report(
    peer_transfer_speed: Arc<DashMap<u64, u64>>,
    cancel_token: CancellationToken,
) {
    let start = Instant::now() + Duration::from_secs(1);
    let mut interval = tokio::time::interval_at(start, Duration::from_secs(1));
    let mut window = FixedQueue::new(5);
    let mut sum = 0.0;
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                break;
            }
            _ = interval.tick() => {
                let mut speed: f64 = 0.0;
                peer_transfer_speed.retain(|_, read_size| {
                    speed += *read_size as f64;
                    false
                });
                window.push(speed).map(|head| sum -= head);
                sum += speed;
                info!("下载速度: {:.2} MB/s", sum / window.len() as f64 / 1024.0 / 1024.0);
            }
        }
    }
}
