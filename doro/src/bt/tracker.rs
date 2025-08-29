pub mod http_tracker;
pub mod udp_tracker;

use core::fmt::Display;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::ops::DerefMut;
use std::sync::{Arc, Weak};

#[cfg(not(target_has_atomic = "64"))]
use portable_atomic::AtomicU64;
#[cfg(target_has_atomic = "64")]
use std::sync::atomic::AtomicU64;

use crate::config::TRACKER_ANNOUNCE_INTERVAL;
use crate::task::{HostSource, ReceiveHost};
use crate::task_manager::PeerId;
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::udp_tracker::UdpTracker;
use ahash::AHashSet;
use anyhow::Result;
use async_trait::async_trait;
use doro_util::bytes_util::Bytes2Int;
use doro_util::anyhow_eq;
use tokio::sync::Mutex;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinSet;
use tracing::debug;

pub trait AnnounceTrait: Debug + Send {
    /// 拿走 peers
    fn take_peers(&mut self) -> Vec<SocketAddr>;
}

#[async_trait]
pub trait TrackerInstance: Send {
    /// 主机地址
    fn host(&self) -> &str;

    /// 进行公告获取
    async fn announcing(
        &mut self, event: Event, info: &AnnounceInfo,
    ) -> Result<Box<dyn AnnounceTrait>>;
}

// ===========================================================================
// Peer Host
// ===========================================================================

/// 解析 peer 列表 - IpV4
#[rustfmt::skip]
pub fn parse_peers_v4(peers: &[u8]) -> Result<Vec<SocketAddr>> {
    anyhow_eq!(peers.len() % 6, 0, "peers length should be a multiple of 6");
    Ok(peers
        .chunks(6)
        .map(|chunk| {
            let ip_bytes: [u8; 4] = chunk[..4].try_into().unwrap();
            SocketAddr::from((ip_bytes, u16::from_be_slice(&chunk[4..])))
        }).collect::<Vec<SocketAddr>>()
    )
}

/// 解析 peer 列表 - IpV6
#[rustfmt::skip]
pub fn parse_peers_v6(peers: &[u8]) -> Result<Vec<SocketAddr>> {
    anyhow_eq!(peers.len() % 18, 0, "peers length should be a multiple of 18");
    Ok(peers
        .chunks(18)
        .map(|chunk| {
            let ip_bytes: [u8; 16] = chunk[..16].try_into().unwrap();
            SocketAddr::from((ip_bytes, u16::from_be_slice(&chunk[16..])))
        }).collect::<Vec<SocketAddr>>()
    )
}

/// announce event
#[derive(Clone, Eq, PartialEq)]
pub enum Event {
    /// 未发生特定事件，正常广播
    None = 0,

    /// 完成了资源下载
    Completed = 1,

    /// 参与资源下载，刚进入到 tracker 中时发送
    Started = 2,

    /// 停止资源下载，退出 tracker 时发送
    Stopped = 3,
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Event::None => "".to_string(),
            Event::Completed => "completed".to_string(),
            Event::Started => "started".to_string(),
            Event::Stopped => "stopped".to_string(),
        };
        write!(f, "{str}")
    }
}

#[derive(Clone, Default)]
pub struct AnnounceInfo {
    download: Arc<AtomicU64>,
    uploaded: Arc<AtomicU64>,
    resource_size: u64,
    port: u16,
}

impl AnnounceInfo {
    pub fn new(
        download: Arc<AtomicU64>, uploaded: Arc<AtomicU64>, resource_size: u64, port: u16,
    ) -> Self {
        Self {
            download,
            uploaded,
            resource_size,
            port,
        }
    }
}

/// 解析 tracker 地址
fn parse_tracker_host(
    announce: &str, info_hash: Arc<[u8; 20]>, peer_id: PeerId,
) -> Option<Box<dyn TrackerInstance>> {
    if announce.starts_with("http") {
        Some(Box::new(HttpTracker::new(
            announce.to_string(),
            info_hash.clone(),
            peer_id.clone(),
        )))
    } else if let Some(announce) = announce.strip_prefix("udp://") {
        let host = announce
            .find("/announce")
            .map(|end| announce[0..end].to_string())
            .unwrap_or_else(|| announce.to_string());

        Some(Box::new(UdpTracker::new(
            host,
            info_hash.clone(),
            peer_id.clone(),
        )))
    } else {
        None
    }
}

fn instance_tracker(peer_id: PeerId, trackers: Vec<Vec<String>>, info_hash: [u8; 20]) -> Trackers {
    let mut ret = vec![];
    let info_hash = Arc::new(info_hash);
    let mut visited = AHashSet::new();

    trackers.iter().flatten().for_each(|announce| {
        if !visited.contains(announce) {
            if let Some(tracker) = parse_tracker_host(announce, info_hash.clone(), peer_id.clone())
            {
                ret.push(Arc::new(Mutex::new((Event::Started, tracker))))
            }
            visited.insert(announce);
        }
    });

    ret
}

async fn tracker_handle_process<T: ReceiveHost + Send + Sync + 'static>(
    tracker: Arc<Mutex<(Event, Box<dyn TrackerInstance>)>>, info: AnnounceInfo,
    receive_host: Weak<T>,
) {
    let mut tracker_lock = tracker.lock().await;
    let (event, tracker_instance) = tracker_lock.deref_mut();
    match tracker_instance.announcing(event.clone(), &info).await {
        Ok(mut announce) => {
            *event = Event::None;
            let peers = announce.take_peers();
            debug!(
                "从 tracker [{}] 那里成功获取到了 peer 共计 [{}] 个",
                tracker_instance.host(),
                peers.len()
            );
            if let Some(receive_host) = receive_host.upgrade() {
                receive_host.receive_hosts(peers, HostSource::Tracker).await;
            }
        }
        Err(e) => {
            debug!(
                "从 tracker [{:?}] 那里获取 peer 失败\t{}",
                tracker_instance.host(),
                e
            );
        }
    }
}

pub enum TrackerType {
    /// udp 类型的 tracker
    UDP,

    /// http 类型的 tracker
    HTTP,
}

type Trackers = Vec<Arc<Mutex<(Event, Box<dyn TrackerInstance>)>>>;

pub struct Tracker<T> {
    /// 回执的信息接收者
    receive_host: Weak<T>,

    /// 公告数据
    info: AnnounceInfo,

    /// trackers
    trackers: Trackers,

    /// 强制刷新信号
    recv: Receiver<()>,

    /// 任务集合
    join_set: JoinSet<()>,
}

impl<T> Drop for Tracker<T> {
    fn drop(&mut self) {
        use tracing::info;
        info!("Tracker 已 drop");
    }
}

impl<T> Tracker<T>
where
    T: ReceiveHost + Send + Sync + 'static,
{
    pub fn new(
        receive_host: Weak<T>, peer_id: PeerId, trackers: Vec<Vec<String>>, info: AnnounceInfo,
        info_hash: [u8; 20], recv: Receiver<()>,
    ) -> Self {
        let trackers = instance_tracker(peer_id, trackers, info_hash);

        Self {
            receive_host,
            info,
            trackers,
            recv,
            join_set: JoinSet::new(),
        }
    }

    /// 本地环境测试
    #[allow(dead_code)]
    async fn local_env_test(&self) -> u64 {
        use std::str::FromStr;
        let peers = vec![
            // SocketAddr::from_str("192.168.2.242:3115").unwrap(),
            SocketAddr::from_str("192.168.2.113:6887").unwrap(),
            SocketAddr::from_str("192.168.2.113:6883").unwrap(),
            SocketAddr::from_str("192.168.2.113:6888").unwrap(),
            SocketAddr::from_str("192.168.2.113:6884").unwrap(),
            SocketAddr::from_str("192.168.2.113:6885").unwrap(),
            SocketAddr::from_str("192.168.2.113:6886").unwrap(),
            SocketAddr::from_str("192.168.2.113:6881").unwrap(),
            SocketAddr::from_str("192.168.2.113:6882").unwrap(),
            // SocketAddr::from_str("209.141.46.35:15982").unwrap(),
            // SocketAddr::from_str("123.156.68.196:20252").unwrap(),
            // SocketAddr::from_str("1.163.51.40:42583").unwrap(),

            // SocketAddr::from_str("106.73.62.197:40370").unwrap(),
        ];

        if let Some(receive_host) = self.receive_host.upgrade() {
            receive_host.receive_hosts(peers, HostSource::Tracker).await;
        }
        9999999
    }

    async fn scan_tracker(&mut self) {
        for tracker in self.trackers.iter() {
            self.join_set.spawn(Box::pin(tracker_handle_process(
                tracker.clone(),
                self.info.clone(),
                self.receive_host.clone(),
            )));
        }
    }

    /// 开始定时扫描
    pub async fn run(mut self) {
        let mut tick = tokio::time::interval(TRACKER_ANNOUNCE_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // self.local_env_test().await;
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    self.scan_tracker().await;
                }
                ret = self.recv.recv() => {
                    match ret {
                        Some(()) => {
                            tick.reset();
                            self.scan_tracker().await;
                        }
                        None => {
                            break
                        }
                    }
                }
            }
        }
    }
}
