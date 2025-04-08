use crate::bytes::Bytes2Int;
use lazy_static::lazy_static;
use nanoid::nanoid;
use rand::RngCore;

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

pub enum PeerHostError {
    InvalidHost,
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

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Host {
    V4(HostV4),
    V6(HostV6),
}

impl From<([u8; 4], u16)> for Host {
    fn from((ip, port): ([u8; 4], u16)) -> Self {
        Self::V4 (HostV4 {
            ip,
            port,
        })
    }
}

impl From<([u8; 16], u16)> for Host {
    fn from((ip, port): ([u8; 16], u16)) -> Self {
        Self::V6 (HostV6 {
            ip,
            port,
        })
    }
}