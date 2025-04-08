//! UDP Tracker 实现
//!
//! TODO - 当前是单线程处理，跑通后改为多线程处理

pub mod buffer;
mod error;
pub mod socket;
#[cfg(test)]
mod tests;

use crate::bt::constant::udp_tracker::*;
use crate::bytes::Bytes2Int;
use crate::torrent::Torrent;
use crate::tracker::udp_tracker::error::SocketError;
use crate::tracker::udp_tracker::socket::SocketArc;
use crate::tracker::{Host, HostV4, HostV6};
use crate::{datetime, if_else, tracker, util};
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use error::Result;
use rand::Rng;
use std::io::Write;
use std::net::IpAddr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::warn;

type Buffer = Vec<u8>;

/// 动作类型
enum Action {
    /// 连接动作
    Connect = 0,

    /// 广播动作
    Announce = 1,

    /// 抓取动作
    Scrape = 2,

    /// 出现错误
    Error = 3,
}

/// announce event
enum Event {
    None = 0,
    Completed = 1,
    Started = 2,
    Stopped = 3,
}

/// 连接信息
#[derive(Default, Debug)]
struct Connect {
    /// 连接 ID
    connection_id: u64,

    /// 获得连接 ID 的时间戳
    timestamp: u64,
}

#[derive(Debug)]
pub struct Announce {
    /// 下次 announce 间隔（秒）
    interval: u32,

    /// 当前未完成下载的 peer 数
    leechers: u32,

    /// 已完成下载的 peer 数
    seedrs: u32,

    /// peer 主机列表
    peers: Vec<Host>,
}

pub struct Scrape {}

pub struct UdpTracker<'a> {
    socket: SocketArc,
    connect: Connect,
    retry_count: u8,
    announce: &'a str,
    info_hash: &'a [u8; 20],
    peer_id: &'a [u8; 20],
    download: u64,
    left: u64,
    uploaded: u64,
    port: u16,
}

impl<'a> UdpTracker<'a> {
    /// 创建一个 UDP Tracker 实例（默认读超时时间为 15 秒）
    ///
    /// # Example
    ///
    /// ```
    /// use dorodoro_bangumi::tracker::{gen_peer_id, udp_tracker as udp_tracker};
    /// use dorodoro_bangumi::tracker::udp_tracker::socket::SocketBuilder;
    /// use udp_tracker::socket::SocketArc;
    /// use udp_tracker::UdpTracker;
    ///
    /// let socket = SocketBuilder::new().build().unwrap();
    /// let info_hash = [0u8; 20];
    /// let peer_id = gen_peer_id();
    /// let mut tracker = UdpTracker::new(socket.clone(), "tracker.torrent.eu.org:451", &info_hash, &peer_id, 0, 9999, 9987).unwrap();
    /// ```
    pub fn new(
        socket: SocketArc,
        announce: &'a str,
        info_hash: &'a [u8; 20],
        peer_id: &'a [u8; 20],
        download: u64,
        left: u64,
        port: u16,
    ) -> Result<Self> {
        Ok(Self {
            socket,
            connect: Connect::default(),
            retry_count: 0,
            announce,
            info_hash,
            peer_id,
            download,
            left,
            uploaded: 0,
            port,
        })
    }

    /// 向 Tracker 发送广播请求
    ///
    /// 正常情况下返回可用资源的地址
    pub fn announcing(&mut self) -> Result<Announce> {
        self.update_connect()?;
        let (req_tran_id, mut req) =
            Self::gen_protocol_head(self.connect.connection_id, Action::Announce);

        req.write(self.info_hash)?;
        req.write(self.peer_id)?;
        req.write_u64::<BigEndian>(self.download)?;
        req.write_u64::<BigEndian>(self.left)?;
        req.write_u64::<BigEndian>(self.uploaded)?;
        req.write_u32::<BigEndian>(Event::Started as u32)?;
        req.write_u32::<BigEndian>(0)?; // ip
        req.write_u32::<BigEndian>(tracker::gen_process_key())?;
        req.write_i32::<BigEndian>(-1)?; // 期望的 peer 数量
        req.write_u16::<BigEndian>(self.port)?;

        let resp = self.send(&req, -1)?;

        // 解析响应数据
        if resp.len() < MIN_ANNOUNCE_RESP_SIZE {
            return Err(SocketError::ResponseLengthError(resp.len()));
        }
        self.check_resp_data(&resp, req_tran_id)?;
        let interval = u32::from_be_slice(&resp[8..12]);
        let leechers = u32::from_be_slice(&resp[12..16]); // 未完成下载的 peer 数
        let seedrs = u32::from_be_slice(&resp[16..20]);

        // 解析 peers 列表
        let peers = if self.socket.is_ipv4() {
            Self::parse_peers_v4(&resp[20..])
        } else {
            Self::parse_peers_v6(&resp[20..])
        };

        Ok(Announce {
            interval,
            leechers,
            seedrs,
            peers,
        })
    }

    /// 向 Tracker 发送抓取请求
    ///
    /// 返回 Tracker 上的资源信息
    pub fn scraping(&mut self) -> Result<Scrape> {
        self.update_connect()?;
        todo!()
    }

    /// 更新连接信息
    ///
    /// # Returns
    /// 正确的情况下返回 `OK(())`
    fn update_connect(&mut self) -> Result<()> {
        let now = datetime::now_secs();
        if self.connect.timestamp <= now && now - self.connect.timestamp <= CONNECTION_ID_TIMEOUT {
            // 无需重新获取连接 ID
            return Ok(());
        }

        // 重新获取连接 ID
        let connect = self.connecting()?;
        self.connect = connect;
        Ok(())
    }

    /// 连接到 Tracker
    ///
    /// # Returns
    /// 正确的情况下，返回 Connect
    fn connecting(&mut self) -> Result<Connect> {
        let (req_tran_id, req) = Self::gen_protocol_head(TRACKER_PROTOCOL_ID, Action::Connect);
        let resp = self.send(&req, MIN_CONNECT_RESP_SIZE as isize)?;

        // 解析响应数据
        if resp.len() < MIN_CONNECT_RESP_SIZE {
            return Err(SocketError::ResponseLengthError(resp.len()));
        }
        self.check_resp_data(&resp, req_tran_id)?;
        let connection_id = u64::from_be_slice(&resp[8..16]);

        Ok(Connect {
            connection_id,
            timestamp: datetime::now_secs(),
        })
    }

    /// 检查响应数据（在调用前，应该先检查数据是否够 8 个字节大小）
    fn check_resp_data(&self, data: &Bytes, req_tran_id: u32) -> Result<()> {
        let action = u32::from_be_slice(&data[0..4]);
        let resp_tran_id = u32::from_be_slice(&data[4..8]);

        if resp_tran_id != req_tran_id {
            return Err(SocketError::TransactionIdMismatching(
                req_tran_id,
                resp_tran_id,
            ));
        } else if action == Action::Error as u32 {
            return Err(SocketError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Tracker 出现错误",
            )));
        }

        Ok(())
    }

    /// 生成协议头，协议头格式：
    /// - 协议 ID（u64）
    /// - 动作类型（u32）
    /// - 请求传输 ID（u32）
    ///
    /// 返回（请求传输 ID，Buffer）
    fn gen_protocol_head(connect_id: u64, action: Action) -> (u32, Buffer) {
        let mut buffer = Buffer::with_capacity(16);
        let req_tran_id = util::rand::gen_tran_id();
        buffer.write_u64::<BigEndian>(connect_id).unwrap();
        buffer.write_u32::<BigEndian>(action as u32).unwrap();
        buffer.write_u32::<BigEndian>(req_tran_id).unwrap();
        (req_tran_id, buffer)
    }

    /// 发送数据并接收响应的数据
    ///
    /// # Arguments
    ///
    /// * `data` - 要发送的数据
    /// * `expect_size` - 期望返回的字节数，-1表示采用默认值（[`MAX_PAYLOAD_SIZE`]）
    ///
    /// # Returns
    /// 正确的情况下，返回响应的数据
    fn send(&mut self, data: &[u8], expect_size: isize) -> Result<Bytes> {
        if self.retry_count > MAX_RETRY_NUM {
            return Err(SocketError::Timeout);
        }

        match self.socket.send_recv(data, self.announce, expect_size) {
            Ok(resp) => {
                self.retry_count = 0;
                Ok(resp)
            }
            Err(e) => {
                // todo - 可能需要针对不同任务做处理，以及需要做基准测试
                warn!("电波无法传达！\n{}", e);
                self.retry_count += 1;
                let lazy = Duration::from_millis(
                    SOCKET_READ_TIMEOUT.as_millis() as u64 * self.retry_count as u64,
                );
                thread::sleep(lazy);
                self.send(data, expect_size)
            }
        }
    }

    /// 解析 peer 列表 - IpV4
    fn parse_peers_v4(peers: &[u8]) -> Vec<Host> {
        assert_eq!(peers.len() % 6, 0, "peer 列表长度错误");
        peers
            .chunks(6)
            .map(|chunk| {
                let ip_bytes: [u8; 4] = chunk[..4].try_into().unwrap();
                Host::from((
                    ip_bytes,
                    u16::from_be_slice(&chunk[4..]),
                ))
            })
            .collect::<Vec<Host>>()
    }

    /// 解析 peer 列表 - IpV6
    fn parse_peers_v6(peers: &[u8]) -> Vec<Host> {
        assert_eq!(peers.len() % 18, 0, "peer 列表长度错误");
        peers
            .chunks(18)
            .map(|chunk| {
                let ip_bytes: [u8; 16] = chunk[..16].try_into().unwrap();
                Host::from((
                    ip_bytes,
                    u16::from_be_slice(&chunk[16..]),
                ))
            })
            .collect::<Vec<Host>>()
    }
}
