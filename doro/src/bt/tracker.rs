pub mod http_tracker;
pub mod udp_tracker;

use core::fmt::Display;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Weak};
use std::time::Duration;

use ahash::AHashSet;
use anyhow::Result;
use doro_util::bytes_util::Bytes2Int;
use doro_util::{anyhow_eq, datetime};
use tokio::sync::Mutex;
use tracing::{debug, trace};

use crate::task::{HostSource, ReceiveHost};
use crate::task_manager::PeerId;
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::udp_tracker::UdpTracker;

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
) -> Option<TrackerInstance> {
    // let announce = form_urlencoded::parse(announce.as_bytes()).
    if announce.starts_with("http") {
        Some(TrackerInstance::HTTP(HttpTracker::new(
            announce.to_string(),
            info_hash.clone(),
            peer_id.clone(),
        )))
    } else if let Some(announce) = announce.strip_prefix("udp://") {
        if let Some(end) = announce.find("/announce") {
            let announce = announce[6..end + 6].to_string();
            Some(TrackerInstance::UDP(UdpTracker::new(
                announce,
                info_hash.clone(),
                peer_id.clone(),
            )))
        } else {
            let announce = announce.to_string();
            Some(TrackerInstance::UDP(UdpTracker::new(
                announce,
                info_hash.clone(),
                peer_id.clone(),
            )))
        }
    } else {
        None
    }
}

fn instance_tracker(
    peer_id: PeerId, trackers: Vec<Vec<String>>, info_hash: [u8; 20],
) -> Vec<Arc<Mutex<(Event, TrackerInstance)>>> {
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

async fn scan_udp_tracker<T: ReceiveHost + Send + Sync + 'static>(
    event: &mut Event, tracker: &mut UdpTracker, scan_time: u64, info: &AnnounceInfo,
    receive_host: Weak<T>,
) -> u64 {
    let mut nrt = u64::MAX;
    if tracker.next_request_time() <= scan_time {
        nrt = match tracker.announcing(event.clone(), info).await {
            Ok(announce) => {
                *event = Event::None;
                let peers = announce.peers.clone();
                trace!(
                    "从 tracker [{}] 那里成功获取到了 peer 共计 [{}] 个",
                    tracker.announce(),
                    peers.len()
                );
                if let Some(receive_host) = receive_host.upgrade() {
                    receive_host.receive_hosts(peers, HostSource::Tracker).await;
                }
                announce.interval as u64
            }
            Err(e) => {
                debug!(
                    "从 tracker [{}] 那里获取 peer 失败\t{}",
                    tracker.announce(),
                    e
                );
                tracker.inc_retry_count()
            }
        };
    }
    nrt
}

async fn scan_http_tracker<T: ReceiveHost + Send + Sync + 'static>(
    event: &mut Event, tracker: &mut HttpTracker, scan_time: u64, info: &AnnounceInfo,
    receive_host: Weak<T>,
) -> u64 {
    let mut nrt = u64::MAX;
    if tracker.next_request_time() <= scan_time {
        nrt = match tracker.announcing(event.clone(), info).await {
            Ok(announce) => {
                *event = Event::None;
                let peers = announce.peers.clone();
                trace!(
                    "从 tracker [{}] 那里成功获取到了 peer 共计 [{}] 个",
                    tracker.announce(),
                    peers.len()
                );
                if let Some(receive_host) = receive_host.upgrade() {
                    receive_host.receive_hosts(peers, HostSource::Tracker).await;
                }
                match announce.min_interval {
                    Some(interval) => interval,
                    None => announce.interval,
                }
            }
            Err(e) => {
                debug!(
                    "从 tracker [{}] 那里获取 peer 失败\t{}",
                    tracker.announce(),
                    e
                );
                tracker.unusable()
            }
        };
    }
    nrt
}

async fn scan_tracker<T: ReceiveHost + Send + Sync + 'static>(
    trackers: Vec<Arc<Mutex<(Event, TrackerInstance)>>>, scan_time: u64, info: AnnounceInfo,
    receive_host: Weak<T>,
) -> u64 {
    let mut interval: u64 = u64::MAX;
    let mut join_handle = vec![];

    for tracker in trackers.iter() {
        let handle = tokio::spawn(Box::pin(tracker_handle_process(
            tracker.clone(),
            scan_time,
            info.clone(),
            receive_host.clone(),
        )));
        join_handle.push(handle);
    }

    for handle in join_handle {
        let i = handle.await.unwrap();
        interval = interval.min(i);
    }

    interval
}

async fn tracker_handle_process<T: ReceiveHost + Send + Sync + 'static>(
    tracker: Arc<Mutex<(Event, TrackerInstance)>>, scan_time: u64, info: AnnounceInfo,
    receive_host: Weak<T>,
) -> u64 {
    match tracker.lock().await.deref_mut() {
        (event, TrackerInstance::UDP(tracker)) => {
            scan_udp_tracker(event, tracker, scan_time, &info, receive_host).await
        }
        (event, TrackerInstance::HTTP(tracker)) => {
            scan_http_tracker(event, tracker, scan_time, &info, receive_host).await
        }
    }
}

pub enum TrackerInstance {
    UDP(UdpTracker),
    HTTP(HttpTracker),
}

pub struct TrackerInner<T> {
    receive_host: Weak<T>,
    info: AnnounceInfo,
    trackers: Vec<Arc<Mutex<(Event, TrackerInstance)>>>,
    scan_time: u64,
}

impl<T> Drop for TrackerInner<T> {
    fn drop(&mut self) {
        use tracing::info;
        info!("TrackerInner 已 drop");
    }
}

impl<T> Deref for Tracker<T> {
    type Target = Arc<TrackerInner<T>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> Clone for Tracker<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct Tracker<T>(Arc<TrackerInner<T>>);

impl<T> Tracker<T>
where
    T: ReceiveHost + Send + Sync + 'static,
{
    pub fn new(
        receive_host: Weak<T>, peer_id: PeerId, trackers: Vec<Vec<String>>, info: AnnounceInfo,
        info_hash: [u8; 20],
    ) -> Self {
        let trackers = instance_tracker(peer_id, trackers, info_hash);

        Self(Arc::new(TrackerInner {
            receive_host,
            info,
            trackers,
            scan_time: datetime::now_secs(),
        }))
    }

    /// 本地环境测试
    #[allow(dead_code)]
    async fn local_env_test(&self) -> u64 {
        use std::str::FromStr;
        let peers = vec![
            SocketAddr::from_str("192.168.2.242:3115").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6881").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6882").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6883").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6884").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6885").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6886").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6887").unwrap(),
            // SocketAddr::from_str("192.168.2.113:6888").unwrap(),

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

    /// 开始定时扫描
    pub async fn run(self) {
        let trackers = self.trackers.clone();
        let scan_time = self.scan_time;
        let info = self.info.clone();
        let receive_host = self.receive_host.clone();

        // self.local_env_test().await;

        loop {
            let task = scan_tracker(
                trackers.clone(),
                scan_time,
                info.clone(),
                receive_host.clone(),
            );
            let interval = task.await;
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    }
}
