pub mod command;
mod error;

use crate::collection::FixedQueue;
use crate::command::CommandHandler;
use crate::core::config::Config;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::GASKET_PREFIX;
use crate::mapper::torrent::{TorrentEntity, TorrentMapper};
use crate::peer::{Peer, Piece};
use crate::peer_manager::PeerManagerContext;
use crate::peer_manager::gasket::command::{Command, SaveProgress, StartWaittingAddr};
use crate::runtime::Runnable;
use crate::store::Store;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};
use crate::{peer, util};
use bincode::{Decode, Encode};
use bytes::BytesMut;
use dashmap::{DashMap, DashSet};
use fnv::FnvHashMap;
use rand::Rng;
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
use tokio_util::sync::WaitForCancellationFuture;
use tracing::{debug, error, info, trace};
use crate::db::ConnWrapper;

#[derive(PartialEq, Debug)]
pub enum ExitReason {
    /// 正常退出
    Normal,

    /// 没有任务的退出
    NotHasJob,

    /// 异常退出
    Exception,
}

#[derive(Eq, PartialEq, Decode, Encode, Clone, Debug)]
pub enum PieceStatus {
    /// 进行中
    Ing(u32),

    /// 暂停，未开始也用这个标记
    Pause(u32),
}

struct PeerInfo {
    /// 通信地址
    addr: Arc<SocketAddr>,

    /// 异步任务句柄
    join_handle: Option<JoinHandle<()>>,
}

impl PeerInfo {
    fn new(addr: Arc<SocketAddr>, join_handle: Option<JoinHandle<()>>) -> Self {
        Self { addr, join_handle }
    }
}

#[derive(Clone)]
pub struct GasketContext {
    /// gasket 的 id
    gtid: String,

    /// peer manager context
    pm_ctx: PeerManagerContext,

    /// peer id
    peer_id: Arc<[u8; 20]>,

    /// 种子信息
    torrent: TorrentArc,

    /// 保存路径
    save_path: Arc<PathBuf>,

    /// peer_id 自增计数
    peer_no_count: Arc<AtomicU64>,

    /// 正在运行中的 peer
    peers: Arc<DashMap<u64, PeerInfo>>,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 存储的是可链接地址，避免 tracker 扫描出相同的然后重复请求链接
    unstart_host: Arc<DashSet<Arc<SocketAddr>>>,

    /// 可连接，但是没有任务可分配的 peer
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 命令发射器
    emitter: Emitter,

    /// peer 传输速率
    peer_transfer_speed: Arc<DashMap<u64, u64>>,

    /// 等待下载的分片
    wait_download_piece: Arc<Mutex<VecDeque<u32>>>,
}

impl GasketContext {
    fn new(
        gtid: String,
        pm_ctx: PeerManagerContext,
        peer_id: Arc<[u8; 20]>,
        torrent: TorrentArc,
        entity: TorrentEntity,
        emitter: Emitter,
    ) -> Self {
        let mut wait_download_piece = VecDeque::new();
        let ub: DashMap<u32, PieceStatus> =
            entity.underway_bytefield.unwrap().into_iter().collect();
        for i in 0..(torrent.info.pieces.len() / 20) {
            let (idx, offset) = util::bytes::bitmap_offset(i);
            if let Some(val) = entity.bytefield.as_ref().unwrap().get(idx) {
                if val & offset == 0 && !ub.contains_key(&(idx as u32)) {
                    wait_download_piece.push_back(i as u32);
                }
            }
        }

        shuffle_vecdeque(&mut wait_download_piece);

        Self {
            gtid,
            pm_ctx,
            peer_id,
            torrent,
            peer_no_count: Arc::new(AtomicU64::new(0)),
            peers: Arc::new(DashMap::new()),
            save_path: Arc::new(entity.save_path.unwrap()),
            download: Arc::new(AtomicU64::new(entity.download.unwrap())),
            uploaded: Arc::new(AtomicU64::new(entity.uploaded.unwrap())),
            bytefield: Arc::new(Mutex::new(entity.bytefield.unwrap())),
            underway_bytefield: Arc::new(ub),
            unstart_host: Arc::new(DashSet::new()),
            wait_queue: Arc::new(Mutex::new(VecDeque::new())),
            emitter,
            peer_transfer_speed: Arc::new(DashMap::new()),
            wait_download_piece: Arc::new(Mutex::new(wait_download_piece)),
        }
    }

    pub fn cancel_token(&self) -> WaitForCancellationFuture {
        self.pm_ctx.context.cancelled()
    }

    pub fn peer_id(&self) -> &[u8; 20] {
        self.peer_id.as_ref()
    }

    pub fn torrent(&self) -> TorrentArc {
        self.torrent.clone()
    }

    pub fn config(&self) -> Config {
        self.pm_ctx.context.get_config().clone()
    }

    pub fn save_path(&self) -> &PathBuf {
        &self.save_path
    }

    pub fn bytefield(&self) -> &Arc<Mutex<BytesMut>> {
        &self.bytefield
    }

    pub async fn peer_exit(&mut self, peer_no: u64, reason: ExitReason) {
        debug!("peer_no [{}] 退出了，退出原因: {:?}", peer_no, reason);
        if self.pm_ctx.context.is_cancelled() {
            return;
        }

        if let Some((_, mut peer)) = self.peers.remove(&peer_no) {
            debug!("成功移除 peer_no [{}]", peer_no);
            if reason == ExitReason::NotHasJob {
                peer.join_handle = None;
                self.wait_queue.lock().await.push_back(peer);
            } else {
                self.unstart_host.remove(&peer.addr);
                trace!("将这个地址从不可用host中移除了");

                // 从等待队列中唤醒一个
                if let Err(e) = self
                    .emitter
                    .send(&self.gtid, StartWaittingAddr.into())
                    .await
                {
                    error!("唤醒等待队列中的失败！{}", e);
                }
            }
        }

        trace!("发送了唤醒消息");
    }

    /// 申请下载暂停了的分块
    pub fn apply_download_pasue_piece(&self) -> Option<(u32, u32)> {
        trace!("分块情况: {:?}", self.underway_bytefield);
        for mut item in self.underway_bytefield.iter_mut() {
            if let PieceStatus::Pause(block_offset) = item.value() {
                let res = Some((*item.key(), *block_offset));
                *item.value_mut() = PieceStatus::Ing(*block_offset);
                return res;
            }
        }
        None
    }

    /// 申请下载分块
    pub async fn apply_download_piece(&self, peer_no: u64) -> Option<(u32, u32)> {
        trace!("peer {} 申请下载分块", peer_no);
        let mut res = None;

        // 这里保证 wait_download_piece 中的一定是未开始的分块
        if let Some(piece_index) = self.wait_download_piece.lock().await.pop_front() {
            let (index, offset) = util::bytes::bitmap_offset(piece_index as usize);
            let mut bytefield = self.bytefield.lock().await;
            if bytefield.get_mut(index).map(|byte| *byte & offset) == Some(0) {
                self.underway_bytefield
                    .entry(piece_index)
                    .and_modify(|value| {
                        if let PieceStatus::Pause(block_offset) = value {
                            res = Some((piece_index, *block_offset));
                            *value = PieceStatus::Ing(*block_offset)
                        }
                    })
                    .or_insert_with(|| {
                        res = Some((piece_index, 0));
                        PieceStatus::Ing(0)
                    });
            }
        }

        res
    }

    /// 归还分块下载
    pub fn give_back_download_piece(&self, peer_no: u64, give_back: FnvHashMap<u32, Piece>) {
        debug!("peer {} 归还下载分块", peer_no);
        for (piece_index, piece) in give_back {
            if !piece.is_finish() {
                self.underway_bytefield
                    .insert(piece_index, PieceStatus::Pause(piece.block_offset()));
            }
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
    pub fn reported_download(&self, piece_index: u32, block_offset: u32, block_size: u64) {
        self.download.fetch_add(block_size, Ordering::Relaxed);
        if let Some(mut item) = self.underway_bytefield.get_mut(&piece_index) {
            if let PieceStatus::Ing(value) = item.value_mut() {
                *value = block_offset;
            }
        }
    }

    /// 上报上传信息
    pub fn reported_uploaded(&self, block_size: u64) {
        self.uploaded.fetch_add(block_size, Ordering::Relaxed);
    }

    /// 上报分块下载完成
    pub async fn reported_piece_finished(&self, piece_index: u32) {
        let (index, offset) = util::bytes::bitmap_offset(piece_index as usize);
        let mut bytefield = self.bytefield.lock().await;
        bytefield.get_mut(index).map(|byte| *byte |= offset);

        self.underway_bytefield.remove(&piece_index);

        // 存数据库，通过通道信号传递给 gasket 执行存储动作，避免阻塞 peer 线程
        self.emitter
            .send(&self.gtid, SaveProgress.into())
            .await
            .unwrap();

        // 异步广播
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

    /// gasket context
    ctx: GasketContext,

    /// 存储处理
    store: Store,
}

pub fn shuffle_vecdeque<T>(deque: &mut VecDeque<T>) {
    let mut rng = rand::rng();
    let len = deque.len();

    // Fisher-Yates 洗牌算法
    for i in (1..len).rev() {
        let j = rng.random_range(0..=i);
        deque.swap(i, j);
    }
}

impl Gasket {
    pub async fn new(
        id: u64,
        torrent: TorrentArc,
        pm_ctx: PeerManagerContext,
        peer_id: Arc<[u8; 20]>,
        emitter: Emitter,
        store: Store,
    ) -> Self {
        let conn = pm_ctx.context.get_conn().await.unwrap();
        let entity = conn.recover_from_db(&torrent.info_hash);
        let ctx = GasketContext::new(
            Self::get_transfer_id(id),
            pm_ctx,
            peer_id,
            torrent,
            entity,
            emitter,
        );

        Self { id, ctx, store }
    }
    
    fn get_config(&self) -> Config {
        self.ctx.config()
    }
    
    async fn get_conn(&self) -> ConnWrapper {
        self.ctx.pm_ctx.context.get_conn().await.unwrap()
    }
    
    async fn start_peer(&self, addr: Arc<SocketAddr>) {
        debug!("启动peer: {}", addr);
        if self.ctx.unstart_host.contains(&addr) {
            return;
        }

        let torrent_peer_conn_limit = self
            .get_config()
            .torrent_peer_conn_limit();
        let unstart_host = self.ctx.unstart_host.clone();

        // 超过配额，加入等待队列中
        if torrent_peer_conn_limit <= self.ctx.peers.len() {
            unstart_host.insert(addr.clone());
            self.ctx.wait_queue
                .lock()
                .await
                .push_back(PeerInfo::new(addr, None));
            return;
        }

        let peer_no = self.ctx.peer_no_count.fetch_add(1, Ordering::Acquire);
        let peer = Peer::new(
            peer_no,
            addr.clone(),
            self.ctx.clone(),
            self.ctx.emitter.clone(),
            self.store.clone(),
        );
        unstart_host.insert(addr.clone());
        let join_handle = tokio::spawn(peer.run());
        self.ctx.peers
            .insert(peer_no, PeerInfo::new(addr, Some(join_handle)));
    }

    async fn shutdown(self) {
        // 先把任务句柄读取出来，避免在循环中等待任务结束，以免和 peer_exit 中的 remove peer 操作形成死锁
        debug!("等待 peers 关闭");
        let handles = self
            .ctx
            .peers
            .iter_mut()
            .map(|mut item| item.join_handle.take())
            .collect::<Vec<Option<JoinHandle<()>>>>();
        for handle in handles {
            if let Some(handle) = handle {
                handle.await.unwrap();
            }
        }
        self.save_progress().await;
    }

    async fn save_progress(&self) {
        debug!("保存下载进度");
        let conn = self.get_conn().await;

        // 将 ub 转换为 Vec，将 Ing 改为 Pause
        let mut ub = Vec::with_capacity(self.ctx.underway_bytefield.len());
        for item in self.ctx.underway_bytefield.iter() {
            let key = item.key().clone();
            let mut value = item.value().clone();
            if let PieceStatus::Ing(v) = value {
                value = PieceStatus::Pause(v);
            }
            ub.push((key, value));
        }

        let entity = TorrentEntity {
            underway_bytefield: Some(ub),
            info_hash: Some(self.ctx.torrent.info_hash.to_vec()),
            bytefield: Some(self.ctx.bytefield.lock().await.clone()),
            download: Some(self.ctx.download.load(Ordering::Relaxed)),
            uploaded: Some(self.ctx.uploaded.load(Ordering::Relaxed)),
            ..Default::default()
        };
        conn.save_progress(entity);
    }

    #[inline]
    fn get_transfer_id(id: u64) -> String {
        format!("{}{}", GASKET_PREFIX, id)
    }

    fn start_tracker(&self) -> JoinHandle<()> {
        let transfer_id = Self::get_transfer_id(self.id);
        trace!("启动 {} 的 tracker", transfer_id);
        let info = AnnounceInfo::new(
            self.ctx.download.clone(),
            self.ctx.uploaded.clone(),
            self.ctx.torrent.info.length,
            self.get_config()
                .tcp_server_addr()
                .port(),
        );
        let tracker = Tracker::new(
            self.ctx.torrent.clone(),
            self.ctx.peer_id.clone(),
            info,
            self.ctx.emitter.clone(),
            self.ctx.pm_ctx.clone(),
            transfer_id,
        );
        tokio::spawn(tracker.run())
    }
    
    #[inline]
    async fn pop_wait_peer(&self) -> Option<PeerInfo> {
        self.ctx.wait_queue.lock().await.pop_front()
    }
}

impl Runnable for Gasket {
    async fn run(mut self) {
        let (send, mut recv) = channel(
            self.get_config().channel_buffer(),
        );
        let transfer_id = Self::get_transfer_id(self.id);
        self.ctx.emitter.register(transfer_id.clone(), send.clone());

        trace!("启动 gasket");
        let tracker_handle = self.start_tracker();
        let speed_handle = tokio::spawn(start_speed_report(
            self.ctx.download.clone(),
            self.ctx.torrent.info.length,
            self.ctx.peer_transfer_speed.clone(),
            self.ctx.pm_ctx.clone(),
        ));

        loop {
            tokio::select! {
                _ = self.ctx.pm_ctx.context.cancelled() => {
                    info!("gasket 退出");
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
        debug!("gasket {} 已退出！", transfer_id);
    }
}

/// 启动速率播报
async fn start_speed_report(
    download: Arc<AtomicU64>,
    length: u64,
    peer_transfer_speed: Arc<DashMap<u64, u64>>,
    peer_manager_context: PeerManagerContext,
) {
    let start = Instant::now() + Duration::from_secs(1);
    let mut interval = tokio::time::interval_at(start, Duration::from_secs(1));
    let mut window = FixedQueue::new(5);
    let mut sum = 0.0;
    loop {
        tokio::select! {
            _ = peer_manager_context.context.cancelled() => {
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
                let download = download.load(Ordering::Relaxed);
                trace!("下载速度: {:.2} MiB/s\t当前进度: {:.2}%", sum / window.len() as f64 / 1024.0 / 1024.0, download as f64 / length as f64 * 100.0);
            }
        }
    }

    debug!("速率播报已退出！")
}
