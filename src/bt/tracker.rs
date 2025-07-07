pub mod command;
pub mod http_tracker;
pub mod udp_tracker;

use crate::bytes_util::Bytes2Int;
use crate::{anyhow_eq, datetime};
use crate::emitter::Emitter;
use crate::emitter::constant::TRACKER;
use crate::emitter::transfer::TransferPtr;
use crate::peer_manager::PeerManagerContext;
use crate::peer_manager::gasket::command::{DiscoverPeerAddr, PeerSource};
use crate::runtime::{CommandHandleResult, CustomTaskResult, RunContext, Runnable};
use crate::torrent::TorrentArc;
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::udp_tracker::UdpTracker;
use ahash::AHashSet;
use core::fmt::Display;
use std::net::SocketAddr;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{Sender};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::{error, info};
use crate::command::CommandHandler;
use crate::tracker::command::Command;
use anyhow::Result;
use futures::stream::FuturesUnordered;

// ===========================================================================
// Peer Host
// ===========================================================================

/// 解析 peer 列表 - IpV4
pub fn parse_peers_v4(peers: &[u8]) -> Result<Vec<SocketAddr>> {
    anyhow_eq!(peers.len() % 6, 0, "peers length should be a multiple of 6");
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
    anyhow_eq!(peers.len() % 18, 0, "peers length should be a multiple of 18");
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

#[derive(Clone)]
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
    #[allow(dead_code)]
    pmc: PeerManagerContext,
    gasket_transfer_id: String,
    trackers: Vec<Arc<Mutex<(Event, TrackerInstance)>>>,
    scan_time: u64,
    cancel_token: CancellationToken,
}

impl Tracker {
    pub fn new(
        torrent: TorrentArc,
        peer_id: Arc<[u8; 20]>,
        info: AnnounceInfo,
        emitter: Emitter,
        pmc: PeerManagerContext,
        gasket_transfer_id: String,
        cancel_token: CancellationToken
    ) -> Self {
        let trackers = instance_tracker(peer_id, torrent);

        Self {
            info,
            emitter,
            pmc,
            gasket_transfer_id,
            trackers,
            scan_time: datetime::now_secs(),
            cancel_token,
        }
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
                    let cmd = DiscoverPeerAddr { peers, source: PeerSource::Tracker }.into();
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
                    let cmd = DiscoverPeerAddr { peers, source: PeerSource::Tracker }.into();
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
    
    /// 本地环境测试
    #[allow(dead_code)]
    async fn local_env_test(&self) -> u64 {
        use std::str::FromStr;
        let cmd = DiscoverPeerAddr {
            peers: vec![
                // SocketAddr::from_str("192.168.2.242:3115").unwrap(),
                
                SocketAddr::from_str("192.168.2.113:6881").unwrap(),
                SocketAddr::from_str("192.168.2.113:6882").unwrap(),
                SocketAddr::from_str("192.168.2.113:6883").unwrap(),
                SocketAddr::from_str("192.168.2.113:6884").unwrap(),
                SocketAddr::from_str("192.168.2.113:6885").unwrap(),
                SocketAddr::from_str("192.168.2.113:6886").unwrap(),
                SocketAddr::from_str("192.168.2.113:6887").unwrap(),
                SocketAddr::from_str("192.168.2.113:6888").unwrap(),
                
                // SocketAddr::from_str("209.141.46.35:15982").unwrap(),
                // SocketAddr::from_str("123.156.68.196:20252").unwrap(),
                // SocketAddr::from_str("1.163.51.40:42583").unwrap(),

                // SocketAddr::from_str("106.73.62.197:40370").unwrap(),
            ],
            source: PeerSource::Tracker,
        }
            .into();
        self.emitter
            .send(&self.gasket_transfer_id, cmd)
            .await
            .unwrap();
        9999999
    }

    /// 创建定时扫描任务
    fn create_scan_task(&self) -> Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>> {
        let trackers = self.trackers.clone();
        let scan_time = self.scan_time;
        let info = self.info.clone();

        let send_to_gasket: Sender<TransferPtr> =
            self.emitter.get(&self.gasket_transfer_id).unwrap();
        
        Box::pin(async move {
            loop {
                let task = Tracker::scan_tracker(
                    trackers.clone(),
                    scan_time,
                    info.clone(),
                    send_to_gasket.clone(),
                );
                let interval = task.await;
                tokio::time::sleep(Duration::from_secs(interval)).await;
            }
        })
    }
}

impl Runnable for Tracker {
    fn emitter(&self) -> &Emitter {
        &self.emitter
    }

    fn get_transfer_id<T: ToString>(suffix: T) -> String {
        format!("{}{}", suffix.to_string(), TRACKER)
    }

    fn get_suffix(&self) -> String {
        self.gasket_transfer_id.to_string()
    }

    fn register_lt_future(&mut self) -> FuturesUnordered<Pin<Box<dyn Future<Output=CustomTaskResult> + Send + 'static>>> {
        let futures = FuturesUnordered::new();        
        // fixme - 正式记得取消下面这行注释
        futures.push(self.create_scan_task());
        futures
    }

    async fn run_before_handle(&mut self, _rc: RunContext) -> Result<()> {
        // fixme - 正式记得注释掉下面这行
        // self.local_env_test().await;
        Ok(())
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel_token.cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }
}
