use std::collections::VecDeque;
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use portable_atomic::AtomicBool;
#[cfg(not(target_has_atomic = "64"))]
use portable_atomic::AtomicU64;
#[cfg(target_has_atomic = "64")]
use std::sync::atomic::AtomicU64;

use anyhow::{anyhow, Error, Result};
use async_trait::async_trait;
use bytes::BytesMut;
use dashmap::{DashMap, DashSet};
use doro_util::global::{GlobalId, Id};
use doro_util::if_else;
use doro_util::sync::{MutexExt, ReadLockExt, WriteLockExt};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;
use tracing::{debug, error, info, trace};

use crate::base_peer::error::{deadly_error, ErrorType, PeerExitReason};
use crate::base_peer::rate_control::probe::Dashboard;
use crate::base_peer::rate_control::RateControl;
use crate::base_peer::{PeerInfoExt, PeerLaunch, PeerLaunchCallback};
use crate::bt::default_servant::coordinator::{Coordinator, PeerSpeed, PeerSwitch};
use crate::config::CHANNEL_BUFFER;
use crate::context::Context;
use crate::default_servant::{DefaultServant, DefaultServantBuilder};
use crate::dht::routing::NodeId;
use crate::dht::DHTTimedTask;
use crate::mapper::torrent::{PieceStatus, TorrentEntity, TorrentMapper, TorrentStatus};
use crate::servant::{Servant, ServantCallback, ServantContext};
use crate::task::{Async, HostSource, ReceiveHost, Subscriber, Task, TaskCallback};
use crate::task_manager::PeerId;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};

pub mod event;

#[derive(Debug)]
pub struct PeerInfo {
    /// peer 的编号
    id: Id,

    /// 通信地址
    addr: SocketAddr,

    /// 来源
    source: HostSource,

    /// 速率仪表盘
    dashboard: Dashboard,

    /// 是否长期运行
    lt_running: bool,

    /// 是否来自等待队列
    waited: bool,

    /// 错误的分块数量
    error_piece_cnt: u32,
}

impl PeerInfo {
    fn new(id: Id, addr: SocketAddr, source: HostSource, dashboard: Dashboard) -> Self {
        Self {
            id,
            addr,
            source,
            dashboard,
            lt_running: true,
            waited: false,
            error_piece_cnt: 0,
        }
    }

    pub fn is_lt(&self) -> bool {
        self.lt_running
    }

    fn reset(&mut self) {
        self.lt_running = true;
        self.dashboard.clear_ing();
    }

    fn set_lt_running(&mut self, running: bool) {
        self.lt_running = running;
    }

    fn set_waited(&mut self, b: bool) {
        self.waited = b;
    }
}

impl PeerInfoExt for PeerInfo {
    fn get_id(&self) -> Id {
        self.id
    }

    fn get_addr(&self) -> SocketAddr {
        self.addr
    }

    fn get_dashboard(&self) -> Dashboard {
        self.dashboard.clone()
    }

    fn get_name(&self) -> String {
        format!("{} - {}", self.addr, self.id)
    }

    fn get_source(&self) -> HostSource {
        self.source
    }

    fn get_peer_conn_limit(&self) -> usize {
        if_else!(self.lt_running, 
            Context::get_config().torrent_lt_peer_conn_limit(), 
            Context::get_config().torrent_peer_conn_limit()
        )
    }

    fn is_from_waited(&self) -> bool {
        self.waited
    }
}

/// 下载种子内容的任务
pub struct DownloadContentInner {
    /// 任务 id
    id: Id,

    /// 执行派遣
    dispatch: Arc<Dispatch>,

    /// 任务控制
    task_control: TaskControl,

    /// tracker
    tracker: Mutex<Option<Tracker<Dispatch>>>,

    /// 异步任务
    handles: Mutex<Option<JoinSet<()>>>,

    /// peer 启动器
    peer_launch: Mutex<Option<PeerLaunch<PeerInfo, DefaultServant, Dispatch>>>,

    /// dht 定时扫描
    dht_timed_task: Mutex<Option<DHTTimedTask<PeerInfo, Dispatch>>>,
}

impl Drop for DownloadContentInner {
    fn drop(&mut self) {
        info!("DownloadContentInner [{}] 已 drop", self.id)
    }
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
    #[rustfmt::skip]
    pub async fn new(
        id: Id, peer_id: PeerId, torrent: TorrentArc, save_path: PathBuf,
    ) -> Result<Self> {
        let entity = Self::load_torrent_from_db(&torrent, &save_path).await?.unwrap_or_default();
        let save_path = Context::get_config().default_download_dir();
        let save_path = Arc::new(entity.save_path.unwrap_or(save_path));
        let download = Arc::new(AtomicU64::new(entity.download.unwrap_or_default()));
        let uploaded = Arc::new(AtomicU64::new(entity.uploaded.unwrap_or_default()));
        let bytefield = Arc::new(Mutex::new(entity.bytefield.unwrap_or_default()));
        let status = Arc::new(RwLock::new(entity.status.unwrap_or(TorrentStatus::Download)));
        let underway_bytefield = Arc::new(DashMap::new());

        entity.underway_bytefield.into_iter().for_each(|ub| {
            for (k, v) in ub {
                underway_bytefield.insert(k, v);
            }
        });

        let wait_queue = Arc::new(Mutex::new(VecDeque::new()));
        let peers = Arc::new(DashMap::new());
        let unstart_host = Arc::new(DashSet::new());
        let peer_transfer_speed = Arc::new(DashMap::new());
        let servant = DefaultServantBuilder::new(torrent.info_hash, peer_id.clone())
            .set_save_path(save_path.clone()).set_bytefield(bytefield.clone())
            .set_underway_bytefield(underway_bytefield.clone())
            .set_torrent(torrent.clone())
            .set_piece_finished(*status.write_pe() != TorrentStatus::Download)
            .arc_build();

        let task_control = TaskControl(Arc::new(TaskControlInner {
            id,
            status: status.clone(),
            torrent: torrent.clone(),
            download: download.clone(),
            uploaded: uploaded.clone(),
            servant: servant.clone(),
            callback: OnceLock::new(),
            subscribers: RwLock::new(Vec::new()),
        }));

        let (dht_tx, dht_rx) = tokio::sync::mpsc::channel(CHANNEL_BUFFER);
        let (tracker_tx, tracker_rx) = tokio::sync::mpsc::channel(CHANNEL_BUFFER);
        let channel = tokio::sync::mpsc::channel(CHANNEL_BUFFER);
        let peer_launch_signal = channel.0.clone();

        let dispatch = Arc::new(Dispatch {
            id,
            wait_queue: wait_queue.clone(),
            peers: peers.clone(),
            servant: servant.clone(),
            task_control: task_control.clone(),
            unstart_host: unstart_host.clone(),
            status: status.clone(),
            peer_transfer_speed: peer_transfer_speed.clone(),
            torrent: torrent.clone(),
            download: download.clone(),
            peer_launch_signal,
            dht_peer_scan_signal: dht_tx,
            tracker_peer_scan_signal: tracker_tx,
            peer_find_flag: AtomicBool::new(false)
        });

        let peer_launch = PeerLaunch::new(
            Arc::downgrade(&servant),
            peers.clone(), wait_queue.clone(), unstart_host.clone(), 
            Arc::downgrade(&dispatch), channel
        );

        let dht_timed_task = DHTTimedTask::new(
            id,
            NodeId::new(torrent.info_hash),
            wait_queue.clone(),
            Arc::downgrade(&dispatch),
            dht_rx,
        );

        servant.set_callback(Arc::downgrade(&dispatch));

        let info = AnnounceInfo::new(
            download.clone(),
            uploaded.clone(),
            torrent.info.length,
            Context::get_config().tcp_server_addr().port(),
        );
        let tracker = Tracker::new(
            Arc::downgrade(&dispatch), peer_id.clone(), 
            torrent.get_trackers(), info, torrent.info_hash,
            tracker_rx
        );

        Ok(Self(Arc::new(DownloadContentInner {
            id,
            dispatch,
            task_control,
            tracker: Mutex::new(Some(tracker)),
            handles: Mutex::new(None),
            peer_launch: Mutex::new(Some(peer_launch)),
            dht_timed_task: Mutex::new(Some(dht_timed_task)),
        })))
    }

    /// 初始化资源，从数据库中恢复进度
    #[rustfmt::skip]
    async fn load_torrent_from_db(torrent: &TorrentArc, save_path: &Path) -> Result<Option<TorrentEntity>> {
        let mut conn = Context::get_conn().await?;
        match conn.recover_from_db(&torrent.info_hash)? {
            Some(entity) => Ok(Some(entity)),
            None => {
                conn.add_torrent(torrent, save_path)?;
                Ok(None)
            }
        }
    }

    // ===========================================================================
    // Content 相关
    // ===========================================================================

    /// 启动异步任务
    fn start_handle(&self) {
        let mut handles = self.handles.lock_pe();
        if handles.is_none() {
            *handles = Some(JoinSet::new());
        }
        let handles = handles.as_mut().unwrap();
        handles.spawn(Box::pin(self.tracker.lock_pe().take().unwrap().run())); // 启动 tracker
        handles.spawn(Box::pin(self.dht_timed_task.lock_pe().take().unwrap().run())); // 启动 dht 扫描
        handles.spawn(Box::pin(Coordinator::new(self.dispatch.clone()).run())); // 启动下载协调器
        handles.spawn(Box::pin(self.peer_launch.lock_pe().take().unwrap().run())); // 启动 peer 启动器
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
            info!("启动任务 [{}]", this.get_id());
            this.start_handle();
            Ok(())
        })
    }

    /// todo - 暂停任务
    fn pause(&self) -> Async<Result<()>> {
        Box::pin(async move {
            Ok(())
        })
    }

    /// 关闭任务
    fn shutdown(&self) -> Async<()> {
        let this = self.clone();
        Box::pin(async move {
            this.task_control.save_progress().await;
        })
    }

    /// 订阅任务的内部执行信息
    fn subscribe_inside_info(&self, subscriber: Subscriber) {
        self.task_control.add_subscribe(subscriber);
    }
}

struct TaskControlInner {
    /// 任务 id
    id: Id,

    /// 任务状态
    status: Arc<RwLock<TorrentStatus>>,

    /// 种子信息
    torrent: TorrentArc,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// peer 事件处理
    servant: Arc<DefaultServant>,

    /// 任务回调句柄
    callback: OnceLock<Box<dyn TaskCallback>>,

    /// 执行消息订阅者
    subscribers: RwLock<Vec<Subscriber>>,
}

impl Drop for TaskControlInner {
    fn drop(&mut self) {
        info!("TaskControlInner [{}] 已退出", self.id)
    }
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
        self.callback
            .set(callback)
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
        let conn = Context::get_conn().await.unwrap();

        // 将 ub 转换为 Vec，将 Ing 改为 Pause
        let underway_bytefield = self.servant.underway_bytefield();
        let mut ub = Vec::with_capacity(underway_bytefield.len());
        for item in underway_bytefield.iter() {
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
            bytefield: Some(self.servant.bytefield().lock_pe().clone()),
            download: Some(self.download.load(Ordering::Relaxed)),
            uploaded: Some(self.uploaded.load(Ordering::Relaxed)),
            status: Some(*self.status.read_pe()),
            ..Default::default()
        };
        conn.save_progress(entity).unwrap();
    }

    async fn finish(&self) {
        self.callback().finish(self.id).await;
    }

    fn add_subscribe(&self, subscriber: Subscriber) {
        self.subscribers.write_pe().push(subscriber);
    }
}

/// 执行派遣，派遣任务给 peer
struct Dispatch {
    /// 任务 id
    id: Id,

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
    status: Arc<RwLock<TorrentStatus>>,

    /// peer 传输速率
    peer_transfer_speed: Arc<DashMap<Id, u64>>,

    /// 种子信息
    torrent: TorrentArc,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// peer 启动信号
    peer_launch_signal: Sender<PeerInfo>,

    /// dht peer 主动扫描信号
    dht_peer_scan_signal: Sender<()>,

    /// tracker peer 主动扫描信号
    #[allow(dead_code)]
    tracker_peer_scan_signal: Sender<()>,

    /// 主动查询标记
    peer_find_flag: AtomicBool,
}

impl Drop for Dispatch {
    fn drop(&mut self) {
        info!("Dispatch - [{}] 已 drop", self.id);
    }
}

impl Dispatch {
    /// 是否完成了下载
    fn is_finished(&self) -> bool {
        *self.status.read_pe() == TorrentStatus::Finished
    }

    /// 直接启动一个 peer
    async fn start_peer(&self, addr: SocketAddr, source: HostSource) {
        debug!("启动peer: {}", addr);
        if self.is_finished() {
            debug!("[{addr}] 下载任务已完成，忽略启动请求");
            return;
        }

        let id = GlobalId::next_id();
        let pi = PeerInfo::new(id, addr, source, Dashboard::new());

        self.do_start_peer(pi, true).await
    }

    async fn take_from_wait_queue(&self) -> Option<PeerInfo> {
        if let Some(mut pi) = self.wait_queue.lock_pe().pop_front() {
            pi.set_waited(true);
            Some(pi)
        } else {
            let flag = self.peer_find_flag
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                    if !current {
                        Some(true)
                    } else {
                        None
                    }
                });
            if flag.is_ok() {
                self.dht_peer_scan_signal.send(()).await.unwrap();
                // self.tracker_peer_scan_signal.send(()).await.unwrap();
                self.peer_find_flag.store(true, Ordering::Relaxed);
            }
            None
        }
    }

    /// 从等待队列中唤醒 peer
    async fn start_wait_peer(&self) {
        if let Some(pi) = self.take_from_wait_queue().await {
            let lt = pi.is_lt();
            self.do_start_peer(pi, lt).await
        }
    }

    /// 启动 peer
    async fn do_start_peer(&self, mut pi: PeerInfo, lt: bool) {
        pi.reset();
        pi.set_lt_running(lt);
        let _ = self.peer_launch_signal.send(pi).await;
    }

    /// 检查是否下载完成
    #[rustfmt::skip]
    fn check_finished(&self) -> bool {
        if *self.status.read_pe() == TorrentStatus::Finished {
            return true;
        }

        if self.servant.check_piece_download_finished() {
            info!("torrent [{}] 下载完成", hex::encode(self.torrent.info_hash));
            *self.status.write_pe() = TorrentStatus::Finished;
            return true;
        }
        
        false
    }

    /// 下载完成后的后置处理
    async fn download_finished_after_handle(&self) {
        if self.is_finished() {
            self.task_control.finish().await;
        }
    }
}

#[async_trait]
impl ReceiveHost for Dispatch {
    /// 查询任务完成
    async fn find_task_finished(&self) {
        self.peer_find_flag.store(false, Ordering::Relaxed);
    }

    /// 接收主机地址
    async fn receive_host(&self, host: SocketAddr, source: HostSource) {
        self.start_peer(host, source).await
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
    /// 接收到了分块数据
    async fn received_block(
        &self, sc: Box<dyn ServantContext>, _: u32, _: u32, _: u32,
    ) -> Result<()> {
        self.servant.request_piece(sc.get_peer().get_id()).await
    }

    /// 有新的分片可用了
    async fn have_piece_available(
        &self, sc: Box<dyn ServantContext>, _piece_idx: u32, _block_offset: u32,
    ) -> Result<()> {
        self.servant.request_piece(sc.get_peer().get_id()).await
    }

    /// 可以进行请求
    async fn request_available(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        self.servant.request_piece(sc.get_peer().get_id()).await?;
        Ok(())
    }

    /// 网络读取量
    fn reported_read_size(&self, sc: Box<dyn ServantContext>, read_size: u64) {
        let id = sc.get_peer().get_id();
        *self.peer_transfer_speed.entry(id).or_insert(0) += read_size
    }

    /// 分块存储成功
    #[rustfmt::skip]
    async fn on_store_block_success(
        &self, _: Box<dyn ServantContext>, _piece_idx: u32, _block_offset: u32, block_size: u32,
    ) {
        self.download.fetch_add(block_size as u64, Ordering::Relaxed);
    }

    /// 分块存储失败
    async fn on_store_block_failed(
        &self, sc: Box<dyn ServantContext>, piece_idx: u32, block_offset: u32, block_size: u32,
        error: Error,
    ) {
        error!(
            "[{}] 分块存储失败: piece_idx: {piece_idx}, block_offset: {block_offset}, block_size: {block_size}, error: {error}",
            sc.get_peer().get_addr()
        );
    }

    /// 分片校验成功
    async fn on_verify_piece_success(&self, _sc: Box<dyn ServantContext>, _piece_idx: u32) {
        self.check_finished();
        self.task_control.save_progress().await;
        self.download_finished_after_handle().await;
    }

    /// 分片校验失败
    #[rustfmt::skip]
    async fn on_verify_piece_failed(&self, sc: Box<dyn ServantContext>, piece_idx: u32, error: Error) {
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
    async fn owner_bitfield(&self, sc: Box<dyn ServantContext>, _bitfield: Arc<RwLock<BytesMut>>) -> Result<()> {
        self.servant.request_piece(sc.get_peer().get_id()).await
    }

    /// 握手成功
    async fn on_handshake_success(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        let peer = sc.get_peer();
        peer.request_bitfield(self.servant.bytefield()).await?;
        peer.request_interested().await
    }

    /// peer 退出
    #[rustfmt::skip]
    async fn on_peer_exit(&self, sc: Box<dyn ServantContext>, reason: PeerExitReason) {
        let peer = sc.get_peer();
        debug!("peer [{}] 退出了，退出原因: {}", peer.name(), reason);
        if Context::global().is_cancelled() {
            return;
        }

        if let Some((_, peer)) = self.peers.remove(&peer.get_id()) {
            debug!("成功移除 peer [{}]", peer.get_name());
            match reason {
                PeerExitReason::DownloadFinished => {
                    self.unstart_host.remove(&peer.addr);
                }
                PeerExitReason::PeriodicPeerReplace => {
                    self.wait_queue.lock_pe().push_back(peer);
                    PeerSwitch::start_temp_peer(self).await;
                }
                PeerExitReason::NotHasJob => {
                    self.unstart_host.remove(&peer.addr);
                    self.start_wait_peer().await;
                }
                PeerExitReason::Exception(e) if e.error_type() == ErrorType::DeadlyError => {
                    error!("peer [{}] 引发致命错误，加入黑名单，原因: {}", peer.get_name(), e);
                    self.start_wait_peer().await;
                }
                _ => {
                    self.unstart_host.remove(&peer.addr);
                    trace!("将这个地址从不可用 host 中移除了");
                    self.start_wait_peer().await;
                }
            }
        }
    }

    /// piece 下载完成
    async fn on_piece_download_finished(&self, _sc: Box<dyn ServantContext>) {
        self.task_control.finish().await;
    }
}

#[async_trait]
impl PeerSwitch for Dispatch {
    /// 获取 task id
    fn get_task_id(&self) -> Id {
        self.id
    }

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
        if let Some(pi) = self.take_from_wait_queue().await {
            self.do_start_peer(pi, false).await
        }
    }

    /// 获得 lt peer 数量
    fn lt_peer_num(&self) -> usize {
        self.peers.iter().filter(|peer| peer.is_lt()).count()
    }

    /// 找到最慢的 lt peer
    #[rustfmt::skip]
    fn find_slowest_lt_peer(&self) -> Option<PeerSpeed> {
        self.peers.iter()
            .filter(|peer| peer.is_lt())
            .min_by_key(|peer| peer.dashboard.bw())
            .map(|peer| PeerSpeed::new(peer.id, peer.addr, peer.dashboard.bw()))
    }

    /// 找到最快的 temp peer
    #[rustfmt::skip]
    fn find_fastest_temp_peer(&self) -> Option<PeerSpeed> {
        self.peers.iter()
            .filter(|peer| !peer.is_lt())
            .max_by_key(|peer| peer.dashboard.bw())
            .map(|peer| PeerSpeed::new(peer.id, peer.addr, peer.dashboard.bw()))
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

    /// 列出所有 peer 的速度信息        
    /// 返回值: (peer_name, peer_bw, peer_cwnd, peer_lt)
    fn list_rate_info(&self) -> Vec<(String, u64, u32, bool)> {
        self.peers.iter().map(|peer| {
            (peer.get_name(), peer.dashboard.bw(), peer.dashboard.cwnd(), peer.is_lt())
        }).collect::<Vec<_>>()
    }

    /// 等待队列长度
    fn get_wait_queue_len(&self) -> usize {
        self.wait_queue.lock_pe().len()
    }

    /// 未启动的 host 数量
    fn get_unstart_host_num(&self) -> usize {
        self.unstart_host.len()
    }
}

#[async_trait]
impl PeerLaunchCallback for Dispatch {
    /// 当 peer 启动失败时调用
    async fn on_peer_start_failed(&self, _peer_info: Box<dyn PeerInfoExt>) {
        self.start_wait_peer().await;
    }

    /// 当 peer 启动成功时调用
    async fn on_peer_start_success(&self, _peer_id: Id) {
        // 启动成功，但什么都不需要做
    }

    /// 发生不可逆的错误
    async fn on_panic(&self, error: Error) {
        error!("发生不可逆的错误: {error}");
        self.task_control.finish().await;
    }
}