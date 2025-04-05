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
use crate::tracker::udp_tracker::error::SocketError;
use crate::tracker::udp_tracker::socket::SocketArc;
use crate::{datetime, util};
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use error::Result;
use rand::Rng;

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

/// 连接信息
#[derive(Default, Debug)]
struct Connect {
    /// 连接 ID
    connection_id: u64,

    /// 获得连接 ID 的时间戳
    timestamp: u64,
}

pub struct Announce {}

pub struct Scrape {}

pub struct UdpTracker<'a> {
    socket: SocketArc,
    connect: Connect,
    retry_count: u8,
    announce: &'a str,
}

impl<'a> UdpTracker<'a> {
    /// 创建一个 UDP Tracker 实例（默认读超时时间为 15 秒）
    ///
    /// # Example
    ///
    /// ```
    /// use dorodoro_bangumi::tracker::udp_tracker as udp_tracker;
    /// use udp_tracker::socket::SocketArc;
    /// use udp_tracker::UdpTracker;
    ///
    /// let socket = SocketArc::new().unwrap();
    /// let mut tracker = UdpTracker::new(socket.clone(), "tracker.torrent.eu.org:451").unwrap();
    /// ```
    pub fn new(socket: SocketArc, announce: &'a str) -> Result<Self> {
        Ok(Self {
            socket,
            connect: Connect::default(),
            retry_count: 0,
            announce,
        })
    }

    /// 向 Tracker 发送广播请求
    ///
    /// 正常情况下返回可用资源的地址
    pub fn announcing(&mut self) -> Result<Announce> {
        self.update_connect()?;
        todo!()
    }

    /// 向 Tracker 发送抓取请求
    ///
    /// 返回 Tracker 上的资源信息
    pub fn scraping(&mut self) -> Result<Scrape> {
        self.update_connect()?;
        todo!()
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
            Err(_) => {
                // 可能需要针对不同任务做处理
                self.retry_count += 1;
                // 开启一个延迟任务，等待一段时间后再重试
                todo!()
            }
        }
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
        let resp = self.send(&req, CONNECT_RESP_SIZE)?;

        // 解析响应数据
        let action = u32::from_be_slice(&resp[0..4]);
        let resp_tran_id = u32::from_be_slice(&resp[4..8]);
        let connection_id = u64::from_be_slice(&resp[8..16]);

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

        Ok(Connect {
            connection_id,
            timestamp: datetime::now_secs(),
        })
    }
}
