//! 磁力链接解析    
//!
//! 流程：
//! 1. 从 DHT 中查询知晓这个磁力链接具体内容的 peer
//! 2. 一次对每个 peer 尝试请求获取内容（协议规定磁力链接的种子内容应该只向同一个 peer 获取）

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use doro_util::global::{GlobalId, Id};
use doro_util::sync::MutexExt;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;
use tracing::{debug, error, info, trace};

use crate::base_peer::error::{ErrorType, PeerExitReason};
use crate::base_peer::rate_control::probe::Dashbord;
use crate::base_peer::{PeerInfoExt, PeerLaunch};
use crate::config::CHANNEL_BUFFER;
use crate::context::Context;
use crate::default_servant::{DefaultServant, DefaultServantBuilder};
use crate::dht::DHTTimedTask;
use crate::dht::routing::NodeId;
use crate::servant::{Servant, ServantCallback, ServantContext};
use crate::task::{HostSource, ReceiveHost, Task, TaskCallback};
use crate::task_manager::PeerId;

struct PeerInfo {
    /// peer id
    id: Id,

    /// peer 地址
    addr: SocketAddr,

    /// 主机来源
    source: HostSource,
}

impl PeerInfoExt for PeerInfo {
    fn get_id(&self) -> Id {
        self.id
    }

    fn get_addr(&self) -> SocketAddr {
        self.addr
    }

    fn get_dashbord(&self) -> Dashbord {
        Dashbord::default()
    }

    fn get_name(&self) -> String {
        format!("{} - {}", self.addr, self.id)
    }

    fn get_source(&self) -> HostSource {
        self.source
    }
}

/// 解析磁力链接
pub struct ParseMagnetInner {
    /// 任务 id
    id: Id,

    /// 任务控制
    task_control: TaskControl,

    /// 异步任务
    handles: Mutex<Option<JoinSet<()>>>,

    /// peer 启动器
    peer_launch: Mutex<Option<PeerLaunch<PeerInfo, DefaultServant>>>,

    /// dht 定时扫描
    dht_timed_task: Mutex<Option<DHTTimedTask<PeerInfo, Dispatch>>>,
}

impl Deref for ParseMagnet {
    type Target = Arc<ParseMagnetInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct ParseMagnet(Arc<ParseMagnetInner>);

impl ParseMagnet {
    pub fn new(id: Id, peer_id: PeerId, info_hash: [u8; 20]) -> Self {
        let servant = DefaultServantBuilder::new(info_hash, peer_id).arc_build();

        let peers = Arc::new(DashMap::new());
        let peer_launch = PeerLaunch::new(
            Arc::downgrade(&servant),
            peers.clone(),
        );
        
        let task_control = TaskControl(Arc::new(TaskControlInner {
            id,
            callback: OnceLock::new(),
        }));

        let wait_queue = Arc::new(Mutex::new(VecDeque::new()));
        let (tx, rx) = tokio::sync::mpsc::channel(CHANNEL_BUFFER);

        let dispatch = Arc::new(Dispatch::new(
            id, servant.clone(), task_control.clone(), 
            wait_queue.clone(), peer_launch.get_sender(),
            tx, peers
        ));

        let dht_timed_task = DHTTimedTask::new(
            id,
            NodeId::new(info_hash),
            wait_queue,
            Arc::downgrade(&dispatch),
            rx,
        );

        servant.set_callback(Arc::downgrade(&dispatch));

        Self(Arc::new(ParseMagnetInner {
            id,
            task_control,
            handles: Mutex::new(Some(JoinSet::new())),
            peer_launch: Mutex::new(Some(peer_launch)),
            dht_timed_task: Mutex::new(Some(dht_timed_task)),
        }))
    }

    #[rustfmt::skip]
    fn start_handle(&self) {
        let mut handles = self.handles.lock_pe();
        if handles.is_none() {
            *handles = Some(JoinSet::new());
        }
        let handles = handles.as_mut().unwrap();
        handles.spawn(Box::pin(self.dht_timed_task.lock_pe().take().unwrap().run())); // 启动 dht 扫描
        handles.spawn(Box::pin(self.peer_launch.lock_pe().take().unwrap().run())); // 启动 peer 启动器
    }
}

impl Task for ParseMagnet {
    fn get_id(&self) -> Id {
        self.id
    }

    fn set_callback(&self, callback: Box<dyn super::TaskCallback>) {
        self.task_control.set_callback(callback);
    }

    fn start(&self) -> super::Async<anyhow::Result<()>> {
        let this = self.clone();
        Box::pin(async move {
            info!("启动任务 [{}]", this.get_id());
            this.start_handle();
            Ok(())
        })
    }

    /// todo - 暂停任务
    fn pause(&self) -> super::Async<anyhow::Result<()>> {
        // let this = self.clone();
        Box::pin(async move { Ok(()) })
    }

    fn shutdown(&self) -> super::Async<()> {
        // 没啥好执行的，直接退出就行
        Box::pin(async move {})
    }

    /// todo - 订阅任务的状态变化
    fn subscribe_inside_info(&self, _subscriber: Box<dyn super::Subscriber>) -> super::Async<()> {
        // let this = self.clone();
        Box::pin(async move {})
    }
}

struct TaskControlInner {
    /// 任务 id
    id: Id,

    /// 任务回调句柄
    callback: OnceLock<Box<dyn TaskCallback>>,
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
}

/// 执行派遣
struct Dispatch {
    /// 任务 id
    id: Id,

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

    /// 不可启动的主机地址
    unstart_host: Arc<DashSet<SocketAddr>>,

    /// 任务是否完成
    finished: AtomicBool,

    /// peer 启动信号
    peer_launch_singal: Sender<PeerInfo>,

    /// peer 启动锁，保证不会超出数量限制
    peer_start_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Dispatch {
    pub fn new(
        id: Id, servant: Arc<DefaultServant>, task_control: TaskControl, 
        wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,
        peer_launch_singal: Sender<PeerInfo>,
        dht_peer_scan_signal: Sender<()>, peers: Arc<DashMap<Id, PeerInfo>>,
    ) -> Self {
        Self {
            id,
            servant,
            task_control,
            wait_queue,
            peer_launch_singal,
            dht_peer_scan_signal,
            peers,
            unstart_host: Arc::new(DashSet::new()),
            finished: AtomicBool::new(false),
            peer_start_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn is_finished(&self) -> bool {
        self.finished.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 任务完成
    async fn finish(&self) {
        self.finished.store(true, std::sync::atomic::Ordering::SeqCst);
        self.task_control.finish().await;
    }

    /// 从等待队列中唤醒 peer
    async fn start_wait_peer(&self) {
        loop {
            // 超过配额，加入等待队列中
            let pi = { self.wait_queue.lock_pe().pop_front() };
            if let Some(pi) = pi {
                if self.start_peer(pi).await.is_ok() {
                    break;
                }
            } else {
                self.dht_peer_scan_signal.send(()).await.unwrap();
                break;
            }
        }
    }

    /// 启动 peer
    async fn start_peer(&self, pi: PeerInfo) -> Result<()> {
        debug!("启动peer: {}", pi.get_name());
        if self.unstart_host.contains(&pi.get_addr()) || self.is_finished() {
            debug!("[{}] 已在启动队列中，或下载任务已完成，忽略启动请求", pi.get_name());
            return Ok(());
        }

        self.unstart_host.insert(pi.get_addr());
        let limit = Context::get_config().torrent_peer_conn_limit();
        let _lock = self.peer_start_lock.lock().await;
        if limit < self.peers.len() {
            self.wait_queue.lock_pe().push_back(pi);
            return Ok(());
        }
        self.peer_launch_singal.send(pi).await?;
        Ok(())
    }
}

#[async_trait]
impl ServantCallback for Dispatch {
    /// 握手成功
    async fn on_handshake_success(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        // 这里如果因为对面不支持扩展协议而请求失败，那么就会因为 Err 终止
        // 这个 peer，这是符合预期的
        sc.get_peer().request_extend_handsake().await
    }

    /// 扩展协议握手成功
    async fn on_extend_handshake_success(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        // 这里如果因为对面不支持扩展协议而请求失败，那么就会因为 Err 终止
        // 这个 peer，这是符合预期的
        self.servant.request_metadata_piece(sc.get_peer().get_id()).await
    }

    /// metadata 下载完成
    async fn on_metadata_complete(&self, _sc: Box<dyn ServantContext>) -> Result<()> {
        self.finish().await;
        Ok(())
    }

    /// metadata 分片写入完成
    async fn on_metadata_piece_write(&self, sc: Box<dyn ServantContext>) -> Result<()> {
        self.servant.request_metadata_piece(sc.get_peer().get_id()).await
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
                    error!("peer [{}] 引发致命错误，加入黑名单，原因: {}", peer.get_name(), e);
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
    /// 接收主机地址
    async fn receive_host(&self, addr: SocketAddr, source: HostSource) {
        let id = GlobalId::next_id();
        let pi = PeerInfo { id, addr, source };
        if let Err(e) = self.start_peer(pi).await {
            debug!("task [{}] 启动 peer 失败: {e}", self.id)
        }
    }

    /// 接收多个主机地址
    async fn receive_hosts(&self, hosts: Vec<SocketAddr>, source: HostSource) {
        for host in hosts {
            self.receive_host(host, source).await;
        }
    }
}
