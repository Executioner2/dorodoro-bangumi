pub mod command;
pub mod error;
pub mod http_tracker;
pub mod udp_tracker;

use crate::bytes::Bytes2Int;
use crate::config::Config;
use crate::datetime;
use crate::emitter::Emitter;
use crate::emitter::constant::TRACKER;
use crate::emitter::transfer::TransferPtr;
use crate::peer_manager::gasket::command::DiscoverPeerAddr;
use crate::runtime::{DelayedTask, Runnable};
use crate::torrent::TorrentArc;
use crate::tracker::error::Error::{InvalidHost, PeerBytesInvalid};
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::udp_tracker::UdpTracker;
use ahash::AHashSet;
use core::fmt::Display;
use core::str::FromStr;
use error::Result;
use lazy_static::lazy_static;
use nanoid::nanoid;
use rand::RngCore;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::channel;
use tracing::{error, info, trace};

lazy_static! {
    /// 进程 id，发送给 Tracker 的，用于区分多开情况
    static ref PROCESS_KEY: u32 = rand::rng().next_u32();
}

/// 随机生成一个 20 字节的 peer_id
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::bt::tracker::gen_peer_id;
///
/// let id = gen_peer_id();
/// println!("peer_id: {:?}", String::from_utf8_lossy(&id));
/// ```
pub fn gen_peer_id() -> [u8; 20] {
    let mut id = [0u8; 20];

    // 正式开发完成后再改成这个
    // id[0..9].copy_from_slice(b"-dr0100--");
    // id[9..].copy_from_slice(nanoid!(11).as_bytes());

    id[..].copy_from_slice(nanoid!(20).as_bytes()); // 临时使用完全随机生成的，且每次启动都不一样
    id
}

/// 随机生成一个 4 字节的 process_key。实际上这个 key 只在程序启动时生成一次。
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::bt::tracker::gen_process_key;
///
/// let key = gen_process_key();
/// println!("process_key: {}", key);
/// ```
pub fn gen_process_key() -> u32 {
    *PROCESS_KEY
}

// ===========================================================================
// Peer Host
// ===========================================================================

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct HostV4 {
    ip: [u8; 4],
    port: u16,
}

impl HostV4 {
    pub fn new(ip: [u8; 4], port: u16) -> Self {
        Self { ip, port }
    }

    pub fn ip(&self) -> [u8; 4] {
        self.ip
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl<T> TryFrom<(&str, T)> for HostV4
where
    T: TryInto<u16>,
{
    type Error = error::Error;

    fn try_from((ip, port): (&str, T)) -> Result<Self> {
        let ip = Ipv4Addr::from_str(ip).map_err(|_| InvalidHost)?.octets();
        let port = port.try_into().map_err(|_| InvalidHost)?;
        Ok(HostV4::new(ip, port))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct HostV6 {
    ip: [u8; 16],
    port: u16,
}

impl HostV6 {
    pub fn new(ip: [u8; 16], port: u16) -> Self {
        Self { ip, port }
    }

    pub fn ip(&self) -> [u8; 16] {
        self.ip
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl<T> TryFrom<(&str, T)> for HostV6
where
    T: TryInto<u16>,
{
    type Error = error::Error;

    fn try_from((ip, port): (&str, T)) -> Result<Self> {
        let ip = Ipv6Addr::from_str(ip).map_err(|_| InvalidHost)?.octets();
        let port = port.try_into().map_err(|_| InvalidHost)?;
        Ok(HostV6::new(ip, port))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Host {
    V4(HostV4),
    V6(HostV6),
}

impl Into<SocketAddr> for Host {
    fn into(self) -> SocketAddr {
        match self {
            Host::V4(host) => SocketAddr::new(IpAddr::from(host.ip()), host.port()),
            Host::V6(host) => SocketAddr::new(IpAddr::from(host.ip()), host.port()),
        }
    }
}

impl From<([u8; 4], u16)> for Host {
    fn from((ip, port): ([u8; 4], u16)) -> Self {
        Self::V4(HostV4 { ip, port })
    }
}

impl From<([u8; 16], u16)> for Host {
    fn from((ip, port): ([u8; 16], u16)) -> Self {
        Self::V6(HostV6 { ip, port })
    }
}

/// 解析 peer 列表 - IpV4
pub fn parse_peers_v4(peers: &[u8]) -> Result<Vec<SocketAddr>> {
    if peers.len() % 6 != 0 {
        return Err(PeerBytesInvalid);
    }
    Ok(peers
        .chunks(6)
        .map(|chunk| {
            let ip_bytes: [u8; 4] = chunk[..4].try_into().unwrap();
            SocketAddr::from((ip_bytes, u16::from_be_slice(&chunk[4..])))
        })
        .collect::<Vec<SocketAddr>>())
}

/// 解析 peer 列表 - IpV6
pub fn parse_peers_v6(peers: &[u8]) -> Result<Vec<SocketAddr>> {
    if peers.len() % 18 != 0 {
        return Err(PeerBytesInvalid);
    }
    Ok(peers
        .chunks(18)
        .map(|chunk| {
            let ip_bytes: [u8; 16] = chunk[..16].try_into().unwrap();
            SocketAddr::from((ip_bytes, u16::from_be_slice(&chunk[16..])))
        })
        .collect::<Vec<SocketAddr>>())
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
        write!(f, "{}", str)
    }
}

pub struct AnnounceInfo {
    download: Arc<AtomicU64>,
    uploaded: Arc<AtomicU64>,
    resource_size: u64,
    port: u16,
}

impl AnnounceInfo {
    pub fn new(
        download: Arc<AtomicU64>,
        uploaded: Arc<AtomicU64>,
        resource_size: u64,
        port: u16,
    ) -> Self {
        Self {
            download,
            uploaded,
            resource_size,
            port,
        }
    }
}

impl Clone for AnnounceInfo {
    fn clone(&self) -> Self {
        Self {
            download: self.download.clone(),
            uploaded: self.uploaded.clone(),
            resource_size: self.resource_size,
            port: self.port,
        }
    }
}

/// 解析 tracker 地址
fn parse_tracker_host(
    announce: &String,
    info_hash: Arc<[u8; 20]>,
    peer_id: Arc<[u8; 20]>,
) -> Option<TrackerInstance> {
    if announce.starts_with("http") {
        Some(TrackerInstance::HTTP(HttpTracker::new(
            announce.clone(),
            info_hash.clone(),
            peer_id.clone(),
        )))
    } else if announce.starts_with("udp://") {
        if let Some(end) = announce[6..].find("/announce") {
            let announce = announce[6..end + 6].to_string();
            Some(TrackerInstance::UDP(UdpTracker::new(
                announce,
                info_hash.clone(),
                peer_id.clone(),
            )))
        } else {
            let announce = announce[6..].to_string();
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
    peer_id: Arc<[u8; 20]>,
    torrent: TorrentArc,
) -> Vec<Arc<Mutex<(Event, TrackerInstance)>>> {
    let mut trackers = vec![];
    let info_hash = Arc::new(torrent.info_hash);
    let root = &torrent.announce;
    let mut visited = AHashSet::new();

    std::iter::once(root)
        .chain(torrent.announce_list.iter().flatten())
        .for_each(|announce| {
            if !visited.contains(announce) {
                if let Some(tracker) =
                    parse_tracker_host(announce, info_hash.clone(), peer_id.clone())
                {
                    trackers.push(Arc::new(Mutex::new((Event::Started, tracker))))
                }
                visited.insert(announce);
            }
        });

    trackers
}

pub enum TrackerInstance {
    UDP(UdpTracker),
    HTTP(HttpTracker),
}

/// tracker
pub struct Tracker {
    info: AnnounceInfo,
    emitter: Emitter,
    config: Config,
    gasket_transfer_id: String,
    trackers: Vec<Arc<Mutex<(Event, TrackerInstance)>>>,
    scan_time: u64,
}

impl Tracker {
    pub fn new(
        torrent: TorrentArc,
        peer_id: Arc<[u8; 20]>,
        info: AnnounceInfo,
        emitter: Emitter,
        config: Config,
        gasket_transfer_id: String,
    ) -> Self {
        let trackers = instance_tracker(peer_id, torrent);

        Self {
            info,
            emitter,
            config,
            gasket_transfer_id,
            trackers,
            scan_time: datetime::now_secs(),
        }
    }

    pub fn get_transfer_id(gasket_transfer_id: &String) -> String {
        format!("{}{}", gasket_transfer_id, TRACKER)
    }

    async fn tracker_handle_process(
        tracker: Arc<Mutex<(Event, TrackerInstance)>>,
        scan_time: u64,
        info: AnnounceInfo,
        send_to_gasket: Sender<TransferPtr>,
    ) -> u64 {
        match tracker.lock().await.deref_mut() {
            (event, TrackerInstance::UDP(tracker)) => {
                Tracker::scan_udp_tracker(event, tracker, scan_time, &info, send_to_gasket).await
            }
            (event, TrackerInstance::HTTP(tracker)) => {
                Tracker::scan_http_tracker(event, tracker, scan_time, &info, send_to_gasket).await
            }
        }
    }

    async fn scan_udp_tracker(
        event: &mut Event,
        tracker: &mut UdpTracker,
        scan_time: u64,
        info: &AnnounceInfo,
        send_to_gasket: Sender<TransferPtr>,
    ) -> u64 {
        let mut nrt = u64::MAX;
        if tracker.next_request_time() <= scan_time {
            nrt = match tracker.announcing(event.clone(), info).await {
                Ok(announce) => {
                    *event = Event::None;
                    let peers = announce.peers.clone();
                    info!(
                        "从 tracker [{}] 那里成功获取到了 peer 共计 [{}] 个",
                        tracker.announce(),
                        peers.len()
                    );
                    let cmd = DiscoverPeerAddr { peers }.into();
                    send_to_gasket.send(cmd).await.unwrap();
                    announce.interval as u64
                }
                Err(e) => {
                    error!(
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

    async fn scan_http_tracker(
        event: &mut Event,
        tracker: &mut HttpTracker,
        scan_time: u64,
        info: &AnnounceInfo,
        send_to_gasket: Sender<TransferPtr>,
    ) -> u64 {
        let mut nrt = u64::MAX;
        if tracker.next_request_time() <= scan_time {
            nrt = match tracker.announcing(event.clone(), &info).await {
                Ok(announce) => {
                    *event = Event::None;
                    let peers = announce.peers.clone();
                    info!(
                        "从 tracker [{}] 那里成功获取到了 peer 共计 [{}] 个",
                        tracker.announce(),
                        peers.len()
                    );
                    let cmd = DiscoverPeerAddr { peers }.into();
                    send_to_gasket.send(cmd).await.unwrap();
                    match announce.min_interval {
                        Some(interval) => interval,
                        None => announce.interval,
                    }
                }
                Err(e) => {
                    error!(
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

    async fn scan_tracker(
        trackers: Vec<Arc<Mutex<(Event, TrackerInstance)>>>,
        scan_time: u64,
        info: AnnounceInfo,
        send_to_gasket: Sender<TransferPtr>,
    ) -> u64 {
        let mut interval: u64 = u64::MAX;
        let mut join_handle = vec![];

        for tracker in trackers.iter() {
            let handle = tokio::spawn(Self::tracker_handle_process(
                tracker.clone(),
                scan_time,
                info.clone(),
                send_to_gasket.clone(),
            ));
            join_handle.push(handle);
        }

        for handle in join_handle {
            let i = handle.await.unwrap();
            interval = interval.min(i);
        }

        interval
    }

    /// 创建定时扫描任务
    async fn create_scan_task(
        &self,
        delay: u64,
    ) -> DelayedTask<Pin<Box<dyn Future<Output = u64> + Send>>> {
        trace!("创建 tracker 扫描任务\t{}秒后执行", delay);
        let send_to_gasket: Sender<TransferPtr> =
            self.emitter.get(&self.gasket_transfer_id).await.unwrap();
        let task = Tracker::scan_tracker(
            self.trackers.clone(),
            self.scan_time,
            self.info.clone(),
            send_to_gasket,
        );
        let delayed_task_handle =
            DelayedTask::new(Duration::from_secs(delay), Box::pin(task) as Pin<Box<_>>);
        delayed_task_handle
    }
}

impl Runnable for Tracker {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.config.channel_buffer());
        self.emitter
            .register(Tracker::get_transfer_id(&self.gasket_transfer_id), send)
            .await
            .unwrap();

        let mut delayed_task_handle = self.create_scan_task(0).await;

        loop {
            tokio::select! {
                res = &mut delayed_task_handle => {
                    let interval = match res {
                        Some(interval) => {
                            interval
                        }
                        None => {
                            15
                        }
                    };
                    delayed_task_handle = self.create_scan_task(interval).await;
                }
                _ = recv.recv() => {

                }
            }
        }
    }
}
