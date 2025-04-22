pub mod error;
pub mod http_tracker;
pub mod udp_tracker;

use crate::bytes::Bytes2Int;
use crate::tracker::error::Error::{InvalidHost, PeerBytesInvalid};
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::udp_tracker::UdpTracker;
use core::fmt::Display;
use core::str::FromStr;
use error::Result;
use lazy_static::lazy_static;
use nanoid::nanoid;
use rand::RngCore;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

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
pub fn parse_peers_v4(peers: &[u8]) -> Result<Vec<Host>> {
    if peers.len() % 6 != 0 {
        return Err(PeerBytesInvalid);
    }
    Ok(peers
        .chunks(6)
        .map(|chunk| {
            let ip_bytes: [u8; 4] = chunk[..4].try_into().unwrap();
            Host::from((ip_bytes, u16::from_be_slice(&chunk[4..])))
        })
        .collect::<Vec<Host>>())
}

/// 解析 peer 列表 - IpV6
pub fn parse_peers_v6(peers: &[u8]) -> Result<Vec<Host>> {
    if peers.len() % 18 != 0 {
        return Err(PeerBytesInvalid);
    }
    Ok(peers
        .chunks(18)
        .map(|chunk| {
            let ip_bytes: [u8; 16] = chunk[..16].try_into().unwrap();
            Host::from((ip_bytes, u16::from_be_slice(&chunk[16..])))
        })
        .collect::<Vec<Host>>())
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

enum TrackerType {
    UDP,
    HTTP,
}

pub enum Tracker<'a> {
    UDP(UdpTracker<'a>),
    HTTP(HttpTracker<'a>),
}

pub struct AnnounceInfo {
    download: u64,
    left: u64,
    uploaded: u64,
    port: u16,
}

/// 解析 tracker 地址
pub fn parse_tracker_host<'a>(
    announce: &'a str,
    info_hash: &'a [u8; 20],
    peer_id: &'a [u8; 20],
) -> Option<Tracker<'a>> {
    if announce.starts_with("http") {
        Some(Tracker::HTTP(HttpTracker::new(
            announce, info_hash, peer_id,
        )))
    } else if announce.starts_with("udp://") {
        if let Some(end) = announce[6..].find("/announce") {
            let announce = &announce[6..end + 6];
            Some(Tracker::UDP(UdpTracker::new(announce, info_hash, peer_id)))
        } else {
            let announce = &announce[6..];
            Some(Tracker::UDP(UdpTracker::new(announce, info_hash, peer_id)))
        }
    } else {
        None
    }
}
