pub mod command;
mod error;

use crate::collection::FixedQueue;
use crate::command::CommandHandler;
use crate::core::config::Config;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::GASKET_PREFIX;
use crate::db::ConnWrapper;
use crate::peer::Peer;
use crate::peer_manager::PeerManagerContext;
use crate::peer_manager::gasket::command::{Command, StartWaittingAddr};
use crate::runtime::Runnable;
use crate::store::Store;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};
use crate::{peer, util};
use ahash::HashMap;
use bincode::{Decode, Encode, config};
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
use tokio_util::sync::WaitForCancellationFuture;
use tracing::{debug, error, info, trace};

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
    Ing,

    /// 暂停，未开始也用这个标记
    Pause(u32),

    /// 已完成
    Finished,
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
    peers: Arc<DashMap<u64, PeerInfo>>,
    peer_manager_context: PeerManagerContext,
    bytefield: Arc<Mutex<BytesMut>>,
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,
    emitter: Emitter,
    download: Arc<AtomicU64>,
    torrent: TorrentArc,
    save_path: Arc<PathBuf>,
    peer_id: Arc<[u8; 20]>,
    unstart_host: Arc<DashSet<Arc<SocketAddr>>>,
    gasket_transfer_id: String,
    peer_transfer_speed: Arc<DashMap<u64, u64>>,
}

impl GasketContext {
    pub fn cancel_token(&self) -> WaitForCancellationFuture {
        self.peer_manager_context.context.cancelled()
    }

    pub fn peer_id(&self) -> &[u8; 20] {
        self.peer_id.as_ref()
    }

    pub fn torrent(&self) -> TorrentArc {
        self.torrent.clone()
    }

    pub fn config(&self) -> Config {
        self.peer_manager_context.context.get_config().clone()
    }

    pub fn save_path(&self) -> &PathBuf {
        &self.save_path
    }

    pub fn bytefield(&self) -> &Arc<Mutex<BytesMut>> {
        &self.bytefield
    }

    pub async fn peer_exit(&mut self, peer_no: u64, reason: ExitReason) {
        debug!("peer_no [{}] 退出了，退出原因: {:?}", peer_no, reason);
        if self.peer_manager_context.context.is_cancelled() {
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
                    .send(&self.gasket_transfer_id, StartWaittingAddr.into())
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
        debug!("分块情况: {:?}", self.underway_bytefield);
        for mut item in self.underway_bytefield.iter_mut() {
            if let PieceStatus::Pause(block_offset) = item.value() {
                let res = Some((*item.key(), *block_offset));
                *item.value_mut() = PieceStatus::Ing;
                return res;
            }
        }
        None
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
        debug!("peer {} 归还下载分块", peer_no);
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
    pub async fn reported_piece_finished(&self, piece_index: u32) {
        self.underway_bytefield
            .get_mut(&piece_index)
            .map(|mut status| *status.value_mut() = PieceStatus::Finished);

        let (index, offset) = util::bytes::bitmap_offset(piece_index as usize);
        let mut bytefield = self.bytefield.lock().await;
        bytefield.get_mut(index).map(|byte| *byte |= offset);

        // 存数据库
        let conn = self.peer_manager_context.context.get_conn().await.unwrap();
        let mut stmt = conn
            .prepare_cached("update torrent set bytefield = ?1 where info_hash = ?2")
            .unwrap();
        stmt.execute((bytefield.as_ref(), &self.torrent.info_hash))
            .unwrap();
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
    peer_manager_context: PeerManagerContext,

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

    /// 存储处理
    store: Store,

    /// peer 传输速率
    peer_transfer_speed: Arc<DashMap<u64, u64>>,
}

impl Gasket {
    pub async fn new(
        id: u64,
        torrent: TorrentArc,
        context: PeerManagerContext,
        peer_id: Arc<[u8; 20]>,
        emitter: Emitter,
        store: Store,
    ) -> Self {
        let (download, uploaded, path, bytefield, ub) = Self::recover_from_db(
            context.context.get_conn().await.unwrap(),
            &torrent.info_hash,
        );

        Self {
            id,
            peer_manager_context: context,
            peer_id,
            torrent,
            peer_no_count: Arc::new(AtomicU64::new(0)),
            peers: Arc::new(DashMap::new()),
            save_path: Arc::new(path),
            download: Arc::new(AtomicU64::new(download)),
            uploaded: Arc::new(AtomicU64::new(uploaded)),
            bytefield: Arc::new(Mutex::new(bytefield)),
            underway_bytefield: Arc::new(ub),
            unstart_host: Arc::new(DashSet::new()),
            wait_queue: Arc::new(Mutex::new(VecDeque::new())),
            emitter,
            store,
            peer_transfer_speed: Arc::new(DashMap::new()),
        }
    }

    /// 从数据库中恢复状态
    fn recover_from_db(
        conn: ConnWrapper,
        info_hash: &[u8; 20],
    ) -> (u64, u64, PathBuf, BytesMut, DashMap<u32, PieceStatus>) {
        let mut stmt = conn.prepare_cached("select download, uploaded, save_path, bytefield, underway_bytefield from torrent where info_hash = ?1").unwrap();
        stmt.query_row([&info_hash], |row| {
            let ub: Vec<(u32, PieceStatus)> = bincode::decode_from_slice(
                row.get::<_, Vec<u8>>(4)?.as_slice(),
                config::standard(),
            )
            .unwrap()
            .0;
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u64>(1)?,
                PathBuf::from(row.get::<_, String>(2)?),
                BytesMut::from(row.get::<_, Vec<u8>>(3)?.as_slice()),
                ub.into_iter()
                    .fold(DashMap::new(), |acc, item: (u32, PieceStatus)| {
                        acc.insert(item.0, item.1);
                        acc
                    }),
            ))
        })
        .unwrap()
    }

    fn get_context(&self) -> GasketContext {
        GasketContext {
            peers: self.peers.clone(),
            peer_manager_context: self.peer_manager_context.clone(),
            bytefield: self.bytefield.clone(),
            underway_bytefield: self.underway_bytefield.clone(),
            wait_queue: self.wait_queue.clone(),
            emitter: self.emitter.clone(),
            download: self.download.clone(),
            torrent: self.torrent.clone(),
            save_path: self.save_path.clone(),
            peer_id: self.peer_id.clone(),
            unstart_host: self.unstart_host.clone(),
            gasket_transfer_id: self.get_transfer_id(),
            peer_transfer_speed: self.peer_transfer_speed.clone(),
        }
    }

    async fn start_peer(&self, addr: Arc<SocketAddr>) {
        debug!("启动peer: {}", addr);
        if self.unstart_host.contains(&addr) {
            return;
        }

        let torrent_peer_conn_limit = self
            .peer_manager_context
            .context
            .get_config()
            .torrent_peer_conn_limit();
        let unstart_host = self.unstart_host.clone();

        // 超过配额，加入等待队列中
        if torrent_peer_conn_limit <= self.peers.len() {
            unstart_host.insert(addr.clone());
            self.wait_queue
                .lock()
                .await
                .push_back(PeerInfo::new(addr, None));
            return;
        }

        let peer_no = self.peer_no_count.fetch_add(1, Ordering::Acquire);
        let context = self.get_context();

        if let Some(mut peer) = Peer::new(
            peer_no,
            addr.clone(),
            context,
            self.emitter.clone(),
            self.store.clone(),
        )
        .await
        {
            let wait_queue = self.wait_queue.clone();
            let peers = self.peers.clone();

            // 不开异步的话，可能导致 channel 堆积满
            tokio::spawn(async move {
                if torrent_peer_conn_limit <= peers.len() {
                    unstart_host.insert(addr.clone());
                    wait_queue.lock().await.push_back(PeerInfo::new(addr, None));
                    return;
                }
                match peer.start().await {
                    Ok(_) => {
                        unstart_host.insert(addr.clone());
                        let join_handle = tokio::spawn(peer.run());
                        peers.insert(peer_no, PeerInfo::new(addr, Some(join_handle)));
                    }
                    Err(e) => {
                        debug!("addr: [{}] 启动失败\t错误信息: {}", addr, e);
                        unstart_host.remove(&addr);
                    }
                }
            });
        } else {
            debug!("addr: [{}] 启动失败", addr);
            unstart_host.remove(&addr);
        }
    }

    async fn shutdown(self) {
        // 先把任务句柄读取出来，避免在循环中等待任务结束，以免和 peer_exit 中的 remove peer 操作形成死锁
        debug!("等待 peers 关闭");
        let handles = self
            .peers
            .iter_mut()
            .map(|mut item| item.join_handle.take())
            .collect::<Vec<Option<JoinHandle<()>>>>();
        for handle in handles {
            if let Some(handle) = handle {
                handle.await.unwrap();
            }
        }

        debug!("保存下载进度");
        let conn = self.peer_manager_context.context.get_conn().await.unwrap();
        let ub = self
            .underway_bytefield
            .iter()
            .map(|item| (item.key().clone(), (*item.value()).clone()))
            .collect::<Vec<(u32, PieceStatus)>>();
        let bytefield = self.bytefield.lock().await.clone();
        let download = self.download.load(Ordering::Relaxed);
        let uploaded = self.uploaded.load(Ordering::Relaxed);
        let infohash = self.torrent.info_hash;

        tokio::task::spawn_blocking(move || {
            let mut stmt = conn.prepare_cached("update torrent set download = ?1, uploaded = ?2, bytefield = ?3, underway_bytefield = ?4 where info_hash = ?5").unwrap();
            stmt.execute((
                download,
                uploaded,
                bytefield.as_ref(),
                bincode::encode_to_vec(ub, config::standard()).unwrap(),
                infohash
            )).unwrap();
        }).await.unwrap();
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
            self.peer_manager_context
                .context
                .get_config()
                .tcp_server_addr()
                .port(),
        );
        let tracker = Tracker::new(
            self.torrent.clone(),
            self.peer_id.clone(),
            info,
            self.emitter.clone(),
            self.peer_manager_context.clone(),
            transfer_id,
        );
        tokio::spawn(tracker.run())
    }
}

impl Runnable for Gasket {
    async fn run(mut self) {
        let (send, mut recv) = channel(
            self.peer_manager_context
                .context
                .get_config()
                .channel_buffer(),
        );
        let transfer_id = self.get_transfer_id();
        self.emitter.register(transfer_id.clone(), send.clone());

        trace!("启动 gasket");
        let tracker_handle = self.start_tracker();
        let speed_handle = tokio::spawn(start_speed_report(
            self.download.clone(),
            self.torrent.info.length,
            self.peer_transfer_speed.clone(),
            self.peer_manager_context.clone(),
        ));

        loop {
            tokio::select! {
                _ = self.peer_manager_context.context.cancelled() => {
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
                info!("下载速度: {:.2} MiB/s\t当前进度: {:.2}%", sum / window.len() as f64 / 1024.0 / 1024.0, download as f64 / length as f64 * 100.0);
            }
        }
    }

    debug!("速率播报已退出！")
}
