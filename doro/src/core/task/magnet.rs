//! 磁力链接解析    
//!
//! 流程：
//! 1. 从 DHT 中查询知晓这个磁力链接具体内容的 peer
//! 2. 一次对每个 peer 尝试请求获取内容（协议规定磁力链接的种子内容应该只向同一个 peer 获取）

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use anyhow::{anyhow, Error, Result};
use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use doro_util::global::{GlobalId, Id};
use doro_util::hash;
use doro_util::sync::{MutexExt, ReadLockExt, WriteLockExt};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;
use tracing::{debug, error, info, trace};
use url::Url;

use crate::base_peer::error::{ErrorType, PeerExitReason};
use crate::base_peer::rate_control::probe::Dashboard;
use crate::base_peer::{PeerInfoExt, PeerLaunch, PeerLaunchCallback};
use crate::config::CHANNEL_BUFFER;
use crate::context::Context;
use crate::default_servant::{DefaultServant, DefaultServantBuilder};
use crate::dht::DHTTimedTask;
use crate::dht::routing::NodeId;
use crate::servant::{Servant, ServantCallback, ServantContext};
use crate::task::magnet::event::Event;
use crate::task::{Async, HostSource, ReceiveHost, Subscriber, Task, TaskCallback};
use crate::task_manager::PeerId;
use crate::tracker::{AnnounceInfo, Tracker};

pub mod event;

#[allow(dead_code)]
pub struct Magnet {
    /// 资源的 info hash
    pub info_hash: [u8; 20],

    /// 资源的 trackers
    pub trackers: Vec<String>,

    /// 资源名字
    pub name: Option<String>,

    /// 文件大小
    pub file_size: Option<u64>,
}

impl Magnet {
    pub fn from_url(url: &str) -> Result<Self> {
        let mut info_hash = None;
        let mut trackers = vec![];
        let mut name = None;
        let mut file_size = None;

        let url = Url::parse(url)?;
        for (k, v) in url.query_pairs() {
            match k.as_ref() {
                "xt" => {
                    let hash = v
                        .split(":")
                        .last()
                        .ok_or_else(|| anyhow!("invalid info hash"))?;
                    if !hash::check_info_hash(hash) {
                        return Err(anyhow!("不正确的 info_hash"));
                    }
                    let hash: [u8; 20] = hex::decode(hash)?
                        .try_into()
                        .map_err(|_| anyhow!("info_hash 字符串转 [u8; 20] 失败"))?;
                    info_hash = Some(hash);
                }
                "tr" => {
                    trackers.push(v.to_string());
                }
                "dn" => {
                    name = Some(v.to_string());
                }
                "xl" => {
                    let size = v
                        .parse::<u64>()
                        .map_err(|e| anyhow!("invalid file size: {}", e))?;
                    file_size = Some(size);
                }
                _ => {}
            }
        }

        let info_hash = info_hash.ok_or_else(|| anyhow!("missing info hash"))?;

        Ok(Self {
            info_hash,
            trackers,
            name,
            file_size,
        })
    }
}

#[derive(Debug)]
struct PeerInfo {
    /// peer id
    id: Id,

    /// peer 地址
    addr: SocketAddr,

    /// 主机来源
    source: HostSource,

    /// 是否来自等待队列
    waited: bool,
}

impl PeerInfo {
    fn new(id: Id, addr: SocketAddr, source: HostSource) -> Self {
        Self {
            id,
            addr,
            source,
            waited: false,
        }
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
        Dashboard::default()
    }

    fn get_name(&self) -> String {
        format!("{} - {}", self.addr, self.id)
    }

    fn get_source(&self) -> HostSource {
        self.source
    }

    fn get_peer_conn_limit(&self) -> usize {
        Context::get_config().torrent_peer_conn_limit()
    }
    
    fn is_from_waited(&self) -> bool {
        self.waited
    }
}

/// 解析磁力链接
pub struct ParseMagnetInner {
    /// 任务 id
    id: Id,

    /// 执行派遣
    #[allow(dead_code)] // 保证 dispatch 不会被 drop，因为其他地方都是用的弱引用
    dispatch: Arc<Dispatch>,

    /// 任务控制
    task_control: TaskControl,

    /// 异步任务
    handles: Mutex<Option<JoinSet<()>>>,

    /// peer 启动器
    peer_launch: Mutex<Option<PeerLaunch<PeerInfo, DefaultServant, Dispatch>>>,

    /// dht 定时扫描
    dht_timed_task: Mutex<Option<DHTTimedTask<PeerInfo, Dispatch>>>,

    /// dht 定时扫描
    tracker: Mutex<Option<Tracker<Dispatch>>>,
}

impl Deref for ParseMagnet {
    type Target = Arc<ParseMagnetInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for ParseMagnetInner {
    fn drop(&mut self) {
        info!("ParseMagnetInner 已 drop")
    }
}

#[derive(Clone)]
pub struct ParseMagnet(Arc<ParseMagnetInner>);

impl ParseMagnet {
    pub fn new(id: Id, peer_id: PeerId, magnet: Magnet) -> Self {
        let info_hash = magnet.info_hash;
        let magnet = Arc::new(magnet);
        let servant = DefaultServantBuilder::new(info_hash, peer_id.clone()).arc_build();
        let wait_queue = Arc::new(Mutex::new(VecDeque::new()));
        let unstart_host = Arc::new(DashSet::new());

        let peers = Arc::new(DashMap::new());
        let channel = tokio::sync::mpsc::channel(CHANNEL_BUFFER);
        let peer_launch_signal = channel.0.clone();

        let task_control = TaskControl(Arc::new(TaskControlInner {
            id,
            callback: OnceLock::new(),
            subscribers: RwLock::new(Vec::new()),
        }));

        let (dht_tx, dht_rx) = tokio::sync::mpsc::channel(CHANNEL_BUFFER);
        let (tracker_tx, tracker_rx) = tokio::sync::mpsc::channel(CHANNEL_BUFFER);

        let dispatch = Arc::new(Dispatch {
            id,
            magnet: magnet.clone(),
            servant: servant.clone(),
            task_control: task_control.clone(),
            wait_queue: wait_queue.clone(),
            peer_launch_signal: peer_launch_signal,
            dht_peer_scan_signal: dht_tx,
            tracker_peer_scan_signal: tracker_tx,
            peers: peers.clone(),
            unstart_host: unstart_host.clone(),
            finished: AtomicBool::new(false),
            peer_find_flag: AtomicBool::new(false),
        });

        let peer_launch = PeerLaunch::new(
            Arc::downgrade(&servant),
            peers.clone(),
            wait_queue.clone(),
            unstart_host.clone(),
            Arc::downgrade(&dispatch),
            channel,
        );

        let tracker = Tracker::new(
            Arc::downgrade(&dispatch),
            peer_id.clone(),
            vec![magnet.trackers.clone()],
            AnnounceInfo::default(),
            info_hash,
            tracker_rx
        );

        let dht_timed_task = DHTTimedTask::new(
            id,
            NodeId::new(info_hash),
            wait_queue,
            Arc::downgrade(&dispatch),
            dht_rx,
        );

        servant.set_callback(Arc::downgrade(&dispatch));

        Self(Arc::new(ParseMagnetInner {
            id,
            dispatch,
            task_control,
            handles: Mutex::new(Some(JoinSet::new())),
            peer_launch: Mutex::new(Some(peer_launch)),
            dht_timed_task: Mutex::new(Some(dht_timed_task)),
            tracker: Mutex::new(Some(tracker)),
        }))
    }

    #[rustfmt::skip]
    async fn start_handle(&self) {
        let mut handles = self.handles.lock_pe();
        if handles.is_none() {
            *handles = Some(JoinSet::new());
        }
        let handles = handles.as_mut().unwrap();
        handles.spawn(Box::pin(self.dht_timed_task.lock_pe().take().unwrap().run())); // 启动 dht 扫描
        handles.spawn(Box::pin(self.peer_launch.lock_pe().take().unwrap().run())); // 启动 peer 启动器
        handles.spawn(Box::pin(self.tracker.lock_pe().take().unwrap().run())); // 启动 tracker 扫描
    }
}

impl Task for ParseMagnet {
    fn get_id(&self) -> Id {
        self.id
    }

    fn set_callback(&self, callback: Box<dyn TaskCallback>) {
        self.task_control.set_callback(callback);
    }

    fn start(&self) -> Async<Result<()>> {
        let this = self.clone();
        Box::pin(async move {
            info!("启动任务 [{}]", this.get_id());
            this.start_handle().await;
            Ok(())
        })
    }

    /// todo - 暂停任务
    fn pause(&self) -> Async<Result<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn shutdown(&self) -> Async<()> {
        let this = self.clone();
        Box::pin(async move {
            // 没啥好执行的，直接退出就行
            info!("关闭任务 [{}]", this.id);
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

    /// 任务回调句柄
    callback: OnceLock<Box<dyn TaskCallback>>,

    /// 执行消息订阅者
    subscribers: RwLock<Vec<Subscriber>>,
}

impl Deref for TaskControl {
    type Target = Arc<TaskControlInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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

    // 返回任务回调句柄
    fn callback(&self) -> &dyn TaskCallback {
        self.callback.get().unwrap().as_ref()
    }

    /// 任务完成    
    async fn finish(&self) {
        self.callback().finish(self.id).await;
    }

    fn add_subscribe(&self, subscriber: Subscriber) {
        self.subscribers.write_pe().push(subscriber);
    }

    async fn notify_event(&self, message: Event) {
        let mut idx = 0;
        while idx < self.subscribers.read_pe().len() {
            let sub = {
                match self.subscribers.read_pe().get(idx) {
                    Some(sub) => sub.clone(),
                    None => break,
                }
            };

            let _ = sub.send(Box::new(message.clone())).await; // 忽略发送失败的，一般就是退出订阅了
            if sub.is_closed() {
                self.subscribers.write_pe().remove(idx);
            } else {
                idx += 1;
            }
        }
    }
}

/// 执行派遣
struct Dispatch {
    /// 任务 id
    id: Id,

    /// 下载资源的 info_hash
    magnet: Arc<Magnet>,

    /// 事件处理
    servant: Arc<DefaultServant>,

    /// 任务控制
    task_control: TaskControl,

    /// 等待队列
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 运行中的 peer
    peers: Arc<DashMap<Id, PeerInfo>>,

    /// dht peer 主动扫描信号
    dht_peer_scan_signal: Sender<()>,
    
    /// tracker peer 主动扫描信号
    tracker_peer_scan_signal: Sender<()>,

    /// 不可启动的主机地址
    unstart_host: Arc<DashSet<SocketAddr>>,

    /// 任务是否完成
    finished: AtomicBool,

    /// peer 启动信号
    peer_launch_signal: Sender<PeerInfo>,

    /// 主动查询标记
    peer_find_flag: AtomicBool,
}

impl Drop for Dispatch {
    fn drop(&mut self) {
        info!("Dispatch - [{}] 已 drop", self.id);
    }
}

impl Dispatch {
    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// 任务完成
    async fn finish(&self) {
        self.finished
            .store(true, Ordering::SeqCst);
        self.task_control.finish().await;
    }

    async fn take_from_wait_queue(&self) -> Option<PeerInfo> {
        if let Some(mut pi) = self.wait_queue.lock_pe().pop_front() {
            pi.set_waited(true);
            Some(pi)
        } else {
            if !self.peer_find_flag.load(Ordering::Relaxed) {
                self.dht_peer_scan_signal.send(()).await.unwrap();
                self.tracker_peer_scan_signal.send(()).await.unwrap();
                self.peer_find_flag.store(true, Ordering::Relaxed);
            }
            None
        }
    }

    /// 从等待队列中唤醒 peer
    async fn start_wait_peer(&self) {
        if let Some(pi) = self.take_from_wait_queue().await {
            self.start_peer(pi).await
        }
    }

    /// 启动 peer
    async fn start_peer(&self, pi: PeerInfo) {
        debug!("启动peer: {}", pi.get_name());
        if self.is_finished() {
            debug!("[{}] 下载任务已完成，忽略启动请求", pi.get_name());
            return;
        }

        self.peer_launch_signal.send(pi).await.unwrap();
    }
}

#[async_trait]
impl ServantCallback for Dispatch {
    /// 握手成功
    async fn on_handshake_success(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        // 这里如果因为对面不支持扩展协议而请求失败，那么就会因为 Err 终止
        // 这个 peer，这是符合预期的
        sc.get_peer().request_extend_handshake().await
    }

    /// 扩展协议握手成功
    async fn on_extend_handshake_success(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        // 这里如果因为对面不支持扩展协议而请求失败，那么就会因为 Err 终止
        // 这个 peer，这是符合预期的
        self.servant
            .request_metadata_piece(sc.get_peer().get_id())
            .await
    }

    /// metadata 下载完成
    async fn on_metadata_complete(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        let torrent = sc.get_peer().parse_torrent_from_metadata(self.magnet.clone())?;
        self.task_control
            .notify_event(Event::ParseSuccess(torrent))
            .await;
        self.finish().await;
        Ok(())
    }

    /// metadata 分片写入完成
    async fn on_metadata_piece_write(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        self.servant
            .request_metadata_piece(sc.get_peer().get_id())
            .await
    }

    /// peer 退出
    async fn on_peer_exit(&self, sc: Box<dyn ServantContext>, reason: PeerExitReason) {
        let peer = sc.get_peer();
        debug!("peer [{}] 退出，原因：{}", peer.name(), reason);
        if Context::global().is_cancelled() {
            return;
        }

        if let Some((_, peer)) = self.peers.remove(&peer.get_id()) {
            match reason {
                PeerExitReason::Exception(e) if e.error_type() == ErrorType::DeadlyError => {
                    error!(
                        "peer [{}] 引发致命错误，加入黑名单，原因: {}",
                        peer.get_name(),
                        e
                    );
                }
                _ => {
                    self.unstart_host.remove(&peer.addr);
                    trace!("将这个地址从不可用 host 中移除了");
                    self.start_wait_peer().await;
                }
            }
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
    async fn receive_host(&self, addr: SocketAddr, source: HostSource) {
        let id = GlobalId::next_id();
        let pi = PeerInfo::new(id, addr, source);
        self.start_peer(pi).await
    }

    /// 接收多个主机地址
    async fn receive_hosts(&self, hosts: Vec<SocketAddr>, source: HostSource) {
        for host in hosts {
            self.receive_host(host, source).await;
        }
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
