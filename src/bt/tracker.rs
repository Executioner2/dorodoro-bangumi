use crate::Integer;
use crate::bytes::Bytes2Int;
use core::fmt::Display;
use core::str::FromStr;
use lazy_static::lazy_static;
use nanoid::nanoid;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

pub mod http_tracker;
pub mod udp_tracker;

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

#[derive(Debug, Eq, PartialEq)]
pub enum PeerHostError {
    InvalidHost,
    PeerBytesInvalid,
}

impl Display for PeerHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeerHostError::InvalidHost => write!(f, "parse host error"),
            PeerHostError::PeerBytesInvalid => write!(f, "peer bytes invalid"),
        }
    }
}

impl std::error::Error for PeerHostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

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
    type Error = PeerHostError;

    fn try_from((ip, port): (&str, T)) -> Result<Self, Self::Error> {
        let ip = Ipv4Addr::from_str(ip)
            .map_err(|_| PeerHostError::InvalidHost)?
            .octets();
        let port = port.try_into().map_err(|_| PeerHostError::InvalidHost)?;
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
    type Error = PeerHostError;

    fn try_from((ip, port): (&str, T)) -> Result<Self, Self::Error> {
        let ip = Ipv6Addr::from_str(ip)
            .map_err(|_| PeerHostError::InvalidHost)?
            .octets();
        let port = port.try_into().map_err(|_| PeerHostError::InvalidHost)?;
        Ok(HostV6::new(ip, port))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Host {
    V4(HostV4),
    V6(HostV6),
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
pub fn parse_peers_v4(peers: &[u8]) -> Result<Vec<Host>, PeerHostError> {
    if peers.len() % 6 != 0 {
        return Err(PeerHostError::PeerBytesInvalid);
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
pub fn parse_peers_v6(peers: &[u8]) -> Result<Vec<Host>, PeerHostError> {
    if peers.len() % 18 != 0 {
        return Err(PeerHostError::PeerBytesInvalid);
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
