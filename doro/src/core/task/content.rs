use std::collections::VecDeque;
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use anyhow::{Error, Ok, Result, anyhow};
use async_trait::async_trait;
use bytes::BytesMut;
use dashmap::{DashMap, DashSet};
use doro_util::bytes_util;
use doro_util::global::{GlobalId, Id};
use doro_util::option_ext::OptionExt;
use doro_util::sync::MutexExt;
use fnv::FnvHashSet;
use tokio::task::JoinSet;
use tracing::{debug, error, info, trace};

use crate::base_peer::BasePeer;
use crate::base_peer::error::{ErrorType, PeerExitReason, deadly_error};
use crate::base_peer::rate_control::RateControl;
use crate::base_peer::rate_control::probe::Dashbord;
use crate::context::Context;
use crate::default_servant::DefaultServant;
use crate::dht;
use crate::dht::routing::NodeId;
use crate::mapper::torrent::{PieceStatus, TorrentEntity, TorrentMapper, TorrentStatus};
use crate::servant::{Servant, ServantCallback, ServantContext};
use crate::task::content::coordinator::{Coordinator, PeerSpeed, PeerSwitch};
use crate::task::{Async, HostSource, ReceiveHost, Subscriber, Task, TaskCallback};
use crate::task_manager::PeerId;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};

mod coordinator;

/// 每间隔一分钟扫描一次 peers
const DHT_FIND_PEERS_INTERVAL: Duration = Duration::from_secs(60);

/// 等待的 peer 上限，如果超过这个数量，就不会进行 dht 扫描
const DHT_WAIT_PEER_LIMIT: usize = 20;

/// 期望每次 dht 扫描能找到 25 个 peer
const DHT_EXPECT_PEERS: usize = 25;

#[derive(Debug)]
pub struct PeerInfo {
    /// peer 的编号
    id: Id,

    /// 通信地址
    addr: SocketAddr,

    /// 来源
    #[allow(dead_code)]
    source: HostSource,

    /// 速率仪表盘
    dashbord: Dashbord,

    /// 是否长期运行
    lt_running: bool,

    /// 是否正在等待有可用分片
    wait_piece: bool,

    /// 正在下载的分片
    download_piece: FnvHashSet<u32>,

    /// 对端拥有的分片
    bitfield: BytesMut,

    /// 错误的分块数量
    error_piece_cnt: u32,
}

impl PeerInfo {
    fn new(id: Id, addr: SocketAddr, source: HostSource, dashbord: Dashbord) -> Self {
        Self {
            id,
            addr,
            source,
            dashbord,
            lt_running: true,
            download_piece: FnvHashSet::default(),
            bitfield: BytesMut::new(),
            wait_piece: false,
            error_piece_cnt: 0,
        }
    }

    pub fn is_lt(&self) -> bool {
        self.lt_running
    }

    fn reset(&mut self) {
        self.lt_running = true;
        self.wait_piece = false;
        self.download_piece.clear();
        self.bitfield.clear();
    }

    fn set_lt_running(&mut self, running: bool) {
        self.lt_running = running;
    }
}

/// 下载种子内容的任务
pub struct DownloadContentInner {
    /// 任务 id
    id: Id,

    /// 种子信息
    torrent: TorrentArc,

    /// 执行派遣
    dispatch: Dispatch,

    /// 任务控制
    task_control: TaskControl,

    /// tracker
    tracker: Tracker<Dispatch>,

    /// 保存路径
    save_path: Arc<RwLock<PathBuf>>,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 可连接，但是没有任务可分配的 peer
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 是否下载完成
    status: Arc<Mutex<TorrentStatus>>,

    /// 异步任务
    handles: Mutex<Option<JoinSet<()>>>,
}

/// 下载种子内容的任务的外部包装
#[derive(Clone)]
pub struct DownloadContent(Arc<DownloadContentInner>);

impl Deref for DownloadContent {
    type Target = Arc<DownloadContentInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DownloadContent {
    pub fn new(id: Id, peer_id: PeerId, torrent: TorrentArc) -> Self {
        let wait_queue = Arc::new(Mutex::new(VecDeque::new()));
        let peers = Arc::new(DashMap::new());
        let bytefield = Arc::new(Mutex::new(Default::default()));
        let unstart_host = Arc::new(DashSet::new());
        let status = Arc::new(Mutex::new(TorrentStatus::Download));
        let peer_transfer_speed = Arc::new(DashMap::new());
        let underway_bytefield = Arc::new(DashMap::new());
        let download = Arc::new(AtomicU64::new(0));
        let uploaded = Arc::new(AtomicU64::new(0));
        let servant = Arc::new(DefaultServant::new(peer_id.clone(), torrent.info_hash));

        let task_control = TaskControl(Arc::new(TaskControlInner {
            id,
            status: status.clone(),
            torrent: torrent.clone(),
            download: download.clone(),
            uploaded: uploaded.clone(),
            bytefield: bytefield.clone(),
            underway_bytefield: underway_bytefield.clone(),
            servant: servant.clone(),
            callback: OnceLock::new(),
        }));

        let dispatch = Dispatch {
            inner: Arc::new(DispatchInner {
                wait_queue: wait_queue.clone(),
                peers: peers.clone(),
                servant: servant.clone(),
                task_control: task_control.clone(),
                unstart_host: unstart_host.clone(),
                status: status.clone(),
                peer_transfer_speed: peer_transfer_speed.clone(),
                torrent: torrent.clone(),
                download: download.clone(),
                bytefield: bytefield.clone(),
                underway_bytefield: underway_bytefield.clone(),
            }),
        };

        servant.init(dispatch.clone());

        let info = AnnounceInfo::new(
            download.clone(),
            uploaded.clone(),
            torrent.info.length,
            Context::get_config().tcp_server_addr().port(),
        );
        let tracker = Tracker::new(dispatch.clone(), peer_id.clone(), torrent.clone(), info);

        Self(Arc::new(DownloadContentInner {
            id,
            torrent,
            dispatch,
            task_control,
            tracker,
            save_path: Arc::new(Default::default()),
            download,
            uploaded,
            bytefield,
            underway_bytefield,
            wait_queue,
            status,
            handles: Mutex::new(None),
        }))
    }

    /// 初始化资源，从数据库中恢复进度
    #[rustfmt::skip]
    async fn init(&self) -> Result<()> {
        let conn = Context::global().get_conn().await?;
        let entity = conn.recover_from_db(&self.torrent.info_hash)?;

        *self.save_path.lock_pe() = entity.save_path.unwrap_or(
            Context::get_config()
                .default_download_dir()
                .clone(),
        );

        let order = Ordering::Relaxed;
        self.download.store(entity.download.unwrap_or_default(), order);
        self.uploaded.store(entity.uploaded.unwrap_or_default(), order);
        entity.bytefield.map_ext(|b| *self.bytefield.lock_pe() = b);
        entity.status.map_ext(|s| *self.status.lock_pe() = s);
        entity.underway_bytefield.into_iter().for_each(|ub| {
            for (k, v) in ub {
                self.underway_bytefield.insert(k, v);
            }
        });

        Ok(())
    }

    // ===========================================================================
    // Content 相关
    // ===========================================================================

    /// 启动异步任务
    async fn start_handle(&self) -> Result<()> {
        let mut handles = self.handles.lock_pe();
        if handles.is_none() {
            *handles = Some(JoinSet::new());
        }
        let handles = handles.as_mut().unwrap();
        handles.spawn(Box::pin(self.tracker.clone().run())); // 启动 tracker
        handles.spawn(Box::pin(self.clone().start_interval_find_peers_from_dht())); // 启动 dht 扫描
        handles.spawn(Box::pin(Coordinator::new(self.dispatch.clone()).run())); // 启动下载协调器
        Ok(())
    }

    /// 定时从 dht 中查询 peer
    async fn start_interval_find_peers_from_dht(self) {
        let info_hash = NodeId::new(self.torrent.info_hash);

        loop {
            if *self.status.lock_pe() == TorrentStatus::Finished {
                break;
            }
            if self.wait_queue.lock_pe().len() < DHT_WAIT_PEER_LIMIT {
                #[rustfmt::skip]
                dht::find_peers(
                    info_hash.clone(),
                    self.dispatch.clone(),
                    self.id,
                    DHT_EXPECT_PEERS,
                ).await;
            }
            tokio::time::sleep(DHT_FIND_PEERS_INTERVAL).await;
        }
    }
}

impl Task for DownloadContent {
    /// 获取任务 id
    fn get_id(&self) -> Id {
        self.id
    }

    /// 设置任务的回调函数  
    fn set_callback(&self, callback: Box<dyn TaskCallback>) {
        self.task_control.set_callback(callback);
    }

    /// 开始任务
    fn start(&self) -> Async<Result<()>> {
        let this = self.clone();
        Box::pin(async move {
            this.init().await?;
            this.start_handle().await
        })
    }

    /// todo - 暂停任务
    fn pause(&self) -> Async<Result<()>> {
        // let this = self.clone();
        Box::pin(async move { Ok(()) })
    }

    /// 关闭任务
    fn shutdown(&self) -> Async<()> {
        let this = self.clone();
        Box::pin(async move { 
            this.task_control.save_progress().await;
        })
    }

    /// todo - 订阅任务的状态变化
    fn subscribe_inside_info(
        &self, _subscriber: Box<dyn Subscriber>,
    ) -> Async<()> {
        // let this = self.clone();
        Box::pin(async move {})
    }
}

struct TaskControlInner {
    /// 任务 id
    id: Id,

    /// 任务状态
    status: Arc<Mutex<TorrentStatus>>,

    /// 种子信息
    torrent: TorrentArc,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// peer 事件处理
    #[allow(dead_code)]
    servant: Arc<DefaultServant>,

    /// 任务回调句柄
    callback: OnceLock<Box<dyn TaskCallback>>,
}

impl Deref for TaskControl {
    type Target = Arc<TaskControlInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 任务状态控制
#[derive(Clone)]
struct TaskControl(Arc<TaskControlInner>);

impl TaskControl {
    /// 设置任务的回调句柄
    fn set_callback(&self, callback: Box<dyn TaskCallback>) {
        self.callback.set(callback)
            .map_err(|_| anyhow!("task callback has been set"))
            .unwrap();
    }

    /// 返回任务回调句柄
    fn callback(&self) -> &dyn TaskCallback {
        self.callback.get().unwrap().as_ref()
    }

    /// 保存下载进度
    async fn save_progress(&self) {
        trace!("保存下载进度");
        let conn = Context::global().get_conn().await.unwrap();

        // 将 ub 转换为 Vec，将 Ing 改为 Pause
        let mut ub = Vec::with_capacity(self.underway_bytefield.len());
        for item in self.underway_bytefield.iter() {
            let key = *item.key();
            let mut value = item.value().clone();
            if let PieceStatus::Ing(v) = value {
                value = PieceStatus::Pause(v);
            }
            ub.push((key, value));
        }

        let entity = TorrentEntity {
            underway_bytefield: Some(ub),
            info_hash: Some(self.torrent.info_hash.to_vec()),
            bytefield: Some(self.bytefield.lock_pe().clone()),
            download: Some(self.download.load(Ordering::Relaxed)),
            uploaded: Some(self.uploaded.load(Ordering::Relaxed)),
            status: Some(*self.status.lock_pe()),
            ..Default::default()
        };
        conn.save_progress(entity).unwrap();
    }

    async fn finish(&self) {
        self.callback().finish(self.id).await;
    }
}

struct DispatchInner {
    /// 可连接，但是没有任务可分配的 peer
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 正在运行中的 peer
    peers: Arc<DashMap<Id, PeerInfo>>,

    /// peer 事件处理
    servant: Arc<DefaultServant>,

    /// 任务控制
    task_control: TaskControl,

    /// 存储的是不允许启动的地址，避免 tracker 扫描出相同的然后重复请求链接。
    /// 多次响应无法通过校验的分片的 peer，会从 peers 和 wait_queue 中移除，
    /// 但是不会从 unstart_host 中移除，以起到黑名单作用
    unstart_host: Arc<DashSet<SocketAddr>>,

    /// 任务状态
    status: Arc<Mutex<TorrentStatus>>,

    /// peer 传输速率
    peer_transfer_speed: Arc<DashMap<Id, u64>>,

    /// 种子信息
    torrent: TorrentArc,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,
}

/// 执行派遣，派遣任务给 peer
#[derive(Clone)]
struct Dispatch {
    inner: Arc<DispatchInner>,
}

impl Dispatch {
    /// 是否完成了下载
    fn is_finished(&self) -> bool {
        *self.status.lock_pe() == TorrentStatus::Finished
    }

    /// 直接启动一个 peer
    async fn start_peer(&self, addr: SocketAddr, source: HostSource) -> Result<()> {
        debug!("启动peer: {}", addr);
        if self.unstart_host.contains(&addr) || self.is_finished() {
            debug!("[{addr}] 已在启动队列中，或下载任务已完成，忽略启动请求");
            return Ok(());
        }

        let limit = Context::get_config().torrent_lt_peer_conn_limit();
        self.unstart_host.insert(addr);
        let id = GlobalId::next_id();
        let pi = PeerInfo::new(id, addr, source, Dashbord::new());

        if limit <= self.peers.len() {
            self.wait_queue.lock_pe().push_back(pi);
            return Ok(());
        }

        self.do_start_peer(pi, true).await
    }

    /// 从等待队列中唤醒 peer
    async fn start_wait_peer(&self, pi: PeerInfo) -> Result<()> {
        // 超过配额，加入等待队列中
        let limit = Context::get_config().torrent_peer_conn_limit();
        if limit < self.peers.len() {
            self.wait_queue.lock_pe().push_back(pi);
            return Ok(());
        }

        self.do_start_peer(pi, true).await
    }

    /// 从临时队列中唤醒 peer
    async fn start_temp_peer(&self, pi: PeerInfo) -> Result<()> {
        // 超过配额，加入等待队列中
        // gasket 通知关闭 peer 关闭，此过程是异步的，因此可能有 1 个 peer 的延迟
        let limit = Context::get_config().torrent_peer_conn_limit();
        if limit < self.peers.len() {
            self.wait_queue.lock_pe().push_back(pi);
            return Ok(());
        }

        self.do_start_peer(pi, false).await
    }

    /// 启动 peer
    async fn do_start_peer(&self, mut pi: PeerInfo, lt: bool) -> Result<()> {
        pi.reset();
        pi.set_lt_running(lt);
        let default_servant = self.servant.clone();
        let bp = BasePeer::new(pi.id, pi.addr, default_servant, pi.dashbord.clone());
        bp.async_run().await?;
        self.peers.insert(pi.id, pi);
        Ok(())
    }

    // 尝试唤醒一个等待 piece 的，下载速率相对来说还不错的 peer
    /// todo - 这个可能要改版
    async fn try_notify_wait_piece(&self) {
        let id = {
            match self.peers.iter().find(|item| item.wait_piece) {
                Some(pi) => pi.id,
                None => return,
            }
        };
        self.assign_peer_handle(id).await;
    }

    /// 尝试分配任务
    /// todo - 这个可能要改版
    pub async fn assign_peer_handle(&self, id: Id) {
        if let Some(peer) = self.servant.get_peer(id) {
            let _ = peer.request_piece().await;
        }
    }

    /// 检查是否下载完成
    #[rustfmt::skip]
    fn check_finished(&self) -> bool {
        if *self.status.lock_pe() == TorrentStatus::Finished {
            return true;
        }

        let mut pn = self.torrent.piece_num() - 1; // 分片下标是从 0 开始的
        let mut last_v = !0u8 << (7 - (pn & 7));
        let bytefield = self.bytefield.lock_pe();
        for v in bytefield.iter().rev() {
            // 从后往前匹配
            if *v != last_v { return false; }
            pn -= 1;
            last_v = u8::MAX;
        }

        info!("torrent [{}] 下载完成", hex::encode(self.torrent.info_hash));
        *self.status.lock_pe() = TorrentStatus::Finished;
        true
    }

    /// 下载完成后的后置处理
    async fn download_finished_after_handle(&self) {
        if self.is_finished() {
            self.task_control.finish().await;
        }
    }
}

impl Deref for Dispatch {
    type Target = Arc<DispatchInner>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[async_trait]
impl ReceiveHost for Dispatch {
    /// 接收主机地址
    async fn receive_host(&self, host: SocketAddr, source: HostSource) {
        if let Err(e) = self.start_peer(host, source).await {
            error!("启动 peer 失败: {e}")
        }
    }

    /// 接收多个主机地址
    async fn receive_hosts(&self, hosts: Vec<SocketAddr>, source: HostSource) {
        for host in hosts {
            self.receive_host(host, source).await;
        }
    }
}

#[async_trait]
impl ServantCallback for Dispatch {
    /// 分块存储成功
    #[rustfmt::skip]
    async fn on_store_block_success(
        &self, _: &dyn ServantContext, piece_idx: u32, block_offset: u32, block_size: u32,
    ) {
        self.download.fetch_add(block_size as u64, Ordering::Relaxed);
        if let Some(mut item) = self.underway_bytefield.get_mut(&piece_idx) {
            if let PieceStatus::Ing(value) = item.value_mut() {
                *value = block_offset;
            }
        }
    }

    /// 分块存储失败
    async fn on_store_block_failed(
        &self, sc: &dyn ServantContext, piece_idx: u32, block_offset: u32, block_size: u32,
        error: Error,
    ) {
        error!(
            "[{}] 分块存储失败: piece_idx: {piece_idx}, block_offset: {block_offset}, block_size: {block_size}, error: {error}",
            sc.get_peer().get_addr()
        );
        todo!("重置这个 peer 的下载进度")
    }

    /// 分片校验成功
    async fn on_verify_piece_success(&self, sc: &dyn ServantContext, piece_idx: u32) {
        let (index, offset) = bytes_util::bitmap_offset(piece_idx as usize);
        self.bytefield
            .lock_pe()
            .get_mut(index)
            .map_ext(|byte| *byte |= offset);

        self.underway_bytefield.remove(&piece_idx);
        self.peers
            .get_mut(&sc.get_peer().get_id())
            .map_ext(|mut item| {
                item.download_piece.remove(&piece_idx);
            });

        self.check_finished();
        self.task_control.save_progress().await;

        self.download_finished_after_handle().await;
    }

    /// 分片校验失败
    #[rustfmt::skip]
    async fn on_verify_piece_failed(&self, sc: &dyn ServantContext, piece_idx: u32, error: Error) {
        error!("[{}] 分片校验失败: piece_idx: {piece_idx}, error: {error}", sc.get_peer().get_addr());
        // 获取真实的分片大小
        let read_length = self.torrent.piece_length(piece_idx);

        // 分片下载失败，删除一个分片的大小
        self.download.fetch_sub(read_length as u64, Ordering::Relaxed);

        // 错误分片数加 1
        let id = sc.get_peer().get_id();
        if let Some(mut peer) = self.peers.get_mut(&id) {
            peer.error_piece_cnt += 1;
            if peer.error_piece_cnt >= Context::get_config().error_piece_limit() {
                self.notify_peer_stop(id, deadly_error(anyhow!("错误分片数达到上限"))).await
            }
        }
    }

    /// 对端传来他拥有的分片
    fn owner_bitfield(&self, sc: &dyn ServantContext, bitfield: BytesMut) {
        let peer = sc.get_peer();
        self.peers
            .get_mut(&peer.get_id())
            .map_ext(|mut item| item.bitfield = bitfield);
    }

    /// 握手成功
    async fn on_handshake_success(&self, sc: &dyn ServantContext) -> Result<()> {
        let peer = sc.get_peer();
        peer.request_bitfield(self.bytefield.clone()).await?;
        peer.request_interested().await
    }

    /// peer 退出
    async fn on_peer_exit(&self, sc: &dyn ServantContext, reason: PeerExitReason) {
        let peer = sc.get_peer();
        debug!("peer_no [{}] 退出了，退出原因: {}", peer.get_id(), reason);
        if Context::global().is_cancelled() {
            return;
        }

        if let Some((_, peer)) = self.peers.remove(&peer.get_id()) {
            debug!("成功移除 peer_no [{}]", peer.id);
            match reason {
                PeerExitReason::DownloadFinished => {
                    self.unstart_host.remove(&peer.addr);
                }
                PeerExitReason::NotHasJob | PeerExitReason::PeriodicPeerReplace => {
                    self.wait_queue.lock_pe().push_back(peer);
                    self.try_notify_wait_piece().await;
                }
                PeerExitReason::Exception(e) if e.error_type() == ErrorType::DeadlyError => {
                    error!(
                        "peer_no [{}] 引发致命错误，加入黑名单，原因: {}",
                        peer.id, e
                    );
                }
                _ => {
                    self.unstart_host.remove(&peer.addr);
                    trace!("将这个地址从不可用host中移除了");
                    loop {
                        let pi = { self.wait_queue.lock_pe().pop_front() };
                        if let Some(pi) = pi {
                            if self.start_wait_peer(pi).await.is_ok() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl PeerSwitch for Dispatch {
    /// 升级为 lt peer
    fn upgrage_lt_peer(&self, id: Id) -> Option<()> {
        self.peers.get_mut(&id)?.set_lt_running(true);
        Some(())
    }

    /// 替换 peer
    async fn replace_peer(&self, old_id: Id, new_id: Id) -> Option<()> {
        self.upgrage_lt_peer(new_id)?;
        self.notify_peer_stop(old_id, PeerExitReason::PeriodicPeerReplace)
            .await;
        Some(())
    }

    /// 通知 peer 停止
    async fn notify_peer_stop(&self, id: Id, reason: PeerExitReason) {
        self.servant.peer_exit(id, reason).await
    }

    /// 开启一个新的临时 peer
    async fn start_temp_peer(&self) {
        if Context::get_config().torrent_peer_conn_limit() <= self.peers.len() {
            info!(
                "无法启动临时 peer，因为当前 peer 数量已达上限，当前 peers 数量: {}",
                self.peers.len()
            );
            return;
        }
        loop {
            let pi = { self.wait_queue.lock_pe().pop_front() };
            if let Some(pi) = pi {
                if self.start_temp_peer(pi).await.is_ok() {
                    break;
                }
            }    
        }
    }

    /// 找到最慢的 lt peer
    #[rustfmt::skip]
    fn find_slowest_lt_peer(&self) -> Option<PeerSpeed> {
        self.peers.iter()
            .filter(|peer| peer.is_lt())
            .min_by_key(|peer| peer.dashbord.bw())
            .map(|peer| PeerSpeed::new(peer.id, peer.addr, peer.dashbord.bw()))
    }

    /// 找到最快的 temp peer
    #[rustfmt::skip]
    fn find_fastest_temp_peer(&self) -> Option<PeerSpeed> {
        self.peers.iter()
            .filter(|peer| !peer.is_lt())
            .max_by_key(|peer| peer.dashbord.bw())
            .map(|peer| PeerSpeed::new(peer.id, peer.addr, peer.dashbord.bw()))
    }

    /// 拿走累计的 peer 传输速度
    fn take_peer_transfer_speed(&self) -> u64 {
        let mut speed = 0;
        self.peer_transfer_speed.retain(|_, read_size| {
            speed += *read_size;
            false
        });
        speed
    }

    /// 获得下载文件大小
    fn file_length(&self) -> u64 {
        self.torrent.info.length
    }

    /// 已下载的文件大小
    fn download_length(&self) -> u64 {
        self.download.load(Ordering::Relaxed)
    }

    /// 是否达到 peer 限制
    fn is_peer_limit(&self) -> bool {
        self.peers.len() > Context::get_config().torrent_peer_conn_limit()
            || self.wait_queue.lock_pe().is_empty()
    }

    /// 是否完成下载
    fn is_finished(&self) -> bool {
        self.is_finished()
    }
}
