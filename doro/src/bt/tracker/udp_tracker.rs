//! UDP Tracker 实现

#[deprecated(note = "一个tracker一个socket的方案会比此模块占用更少的cpu资源")]
pub mod socket;
#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::Bytes;
use doro_util::buffer::ByteBuffer;
use doro_util::bytes_util::{Bytes2Int, WriteBytesBigEndian};
use doro_util::{anyhow_eq, anyhow_ge, anyhow_le, anyhow_ne, datetime, net};
use tokio::net::UdpSocket;
use tracing::warn;

use crate::bt::constant::udp_tracker::*;
use crate::task_manager::PeerId;
use crate::tracker;
use crate::tracker::{AnnounceInfo, AnnounceTrait, Event, TrackerInstance};

type Buffer = Vec<u8>;

/// 动作类型
enum Action {
    /// 连接动作
    Connect = 0,

    /// 广播动作
    Announce = 1,

    /// 抓取动作
    _Scrape = 2,

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

#[derive(Debug)]
pub struct Announce {
    /// 下次 announce 间隔（秒）
    pub interval: u32,

    /// 当前未完成下载的 peer 数
    pub leechers: u32,

    /// 已完成下载的 peer 数
    pub seedrs: u32,

    /// peer 主机列表
    pub peers: Vec<SocketAddr>,
}

impl AnnounceTrait for Announce {
    fn take_peers(&mut self) -> Vec<SocketAddr> {
        std::mem::take(&mut self.peers)
    }
}

pub struct Scrape {}

pub struct UdpTracker {
    connect: Connect,
    retry_count: u8,
    host: String,
    info_hash: Arc<[u8; 20]>,
    peer_id: PeerId,
    next_request_time: u64,
}

impl UdpTracker {
    /// 创建一个 UDP Tracker 实例（默认读超时时间为 15 秒）
    pub fn new(host: String, info_hash: Arc<[u8; 20]>, peer_id: PeerId) -> Self {
        Self {
            connect: Connect::default(),
            retry_count: 0,
            host,
            info_hash,
            peer_id,
            next_request_time: 0,
        }
    }

    pub fn next_request_time(&self) -> u64 {
        self.next_request_time
    }

    /// 向 Tracker 发送抓取请求
    ///
    /// 返回 Tracker 上的资源信息
    pub async fn scraping(&mut self) -> Result<Scrape> {
        self.update_connect().await?;
        todo!()
    }

    /// 重试次数 +1，返回距离下一次请求的间隔时间
    pub fn inc_retry_count(&mut self) -> u64 {
        if self.retry_count > MAX_RETRY_NUM {
            return u64::MAX;
        }
        self.retry_count += 1;
        let interval = SOCKET_READ_TIMEOUT.as_millis() as u64 * self.retry_count as u64;
        self.next_request_time = datetime::now_secs() + interval;
        interval
    }

    pub fn announce(&self) -> &str {
        &self.host
    }

    /// 发送数据到指定地址，并接收期望大小的数据。如果 expect_size 为负数，则接收默认大小（）的数据。
    ///
    /// # Arguments
    ///
    /// * `data` - 要发送的数据
    /// * `target` - 发送的目标地址
    /// * `expect_size` - 期望接收的数据大小，如果为负数，则接收默认大小（[`MAX_PAYLOAD_SIZE`]）的数据
    ///
    /// # Returns
    /// 正常的情况下，返回接收到的数据。
    async fn send_recv(&self, data: &[u8], target: &str, expect_size: isize) -> Result<Bytes> {
        let addr = net::domain_resolve(target).await.ok_or(anyhow!("无法解析的域名 [{target:?}]"))?;
        let socket = UdpSocket::bind(DEFAULT_ADDR).await?;
        socket.connect(addr).await?;

        tokio::time::timeout(SOCKET_WRITE_TIMEOUT, socket.send(data)).await??;
        let expect_size = if expect_size < 0 {
            MAX_PAYLOAD_SIZE
        } else {
            expect_size as usize
        };

        let mut buffer = ByteBuffer::new(expect_size);
        let (size, _socket_addr) =
            tokio::time::timeout(SOCKET_WRITE_TIMEOUT, socket.recv_from(buffer.as_mut())).await??;

        // 转换为已初始化的缓冲区
        buffer.resize(size);

        Ok(Bytes::from_owner(buffer))
    }

    /// 更新连接信息
    ///
    /// # Returns
    /// 正确的情况下返回 `OK(())`
    async fn update_connect(&mut self) -> Result<()> {
        let now = datetime::now_secs();
        if self.connect.timestamp <= now && now - self.connect.timestamp <= CONNECTION_ID_TIMEOUT {
            // 无需重新获取连接 ID
            return Ok(());
        }

        // 重新获取连接 ID
        let connect = self.connecting().await?;
        self.connect = connect;
        Ok(())
    }

    /// 连接到 Tracker
    ///
    /// # Returns
    /// 正确的情况下，返回 Connect
    async fn connecting(&mut self) -> Result<Connect> {
        let (req_tran_id, req) = Self::gen_protocol_head(TRACKER_PROTOCOL_ID, Action::Connect);
        let resp = self.send(&req, MIN_CONNECT_RESP_SIZE as isize).await?;

        // 解析响应数据
        anyhow_ge!(resp.len(), MIN_CONNECT_RESP_SIZE, "响应数据长度不足");
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

        anyhow_eq!(resp_tran_id, req_tran_id, "传输 ID 不匹配");
        anyhow_ne!(action, Action::Error as u32, "Tracker 出现错误");

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
        let req_tran_id = doro_util::rand::gen_tran_id();
        buffer.write_u64(connect_id).unwrap();
        buffer.write_u32(action as u32).unwrap();
        buffer.write_u32(req_tran_id).unwrap();
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
    async fn send(&mut self, data: &[u8], expect_size: isize) -> Result<Bytes> {
        anyhow_le!(self.retry_count, MAX_RETRY_NUM, "请求超时，且重试次数过多");

        match self.send_recv(data, &self.host, expect_size).await {
            Ok(resp) => {
                self.retry_count = 0;
                Ok(resp)
            }
            Err(e) => {
                // todo - 可能需要针对不同任务做处理，以及需要做基准测试
                warn!("电波无法传达！\t{}", e);
                Err(e)
                // self.retry_count += 1;
                // let lazy = Duration::from_millis(
                //     SOCKET_READ_TIMEOUT.as_millis() as u64 * self.retry_count as u64,
                // );
                // thread::sleep(lazy);
                // self.send(data, expect_size)
            }
        }
    }
}

#[async_trait]
impl TrackerInstance for UdpTracker {
    /// 返回主机地址
    fn host(&self) -> &str {
        &self.host
    }
    
    /// 向 Tracker 发送广播请求
    ///
    /// 正常情况下返回可用资源的地址
    async fn announcing(&mut self, event: Event, info: &AnnounceInfo) -> Result<Box<dyn AnnounceTrait>> {
        self.update_connect().await?;
        let (req_tran_id, mut req) =
            Self::gen_protocol_head(self.connect.connection_id, Action::Announce);

        let download = info.download.load(Ordering::Acquire);
        let left = info.resource_size.saturating_sub(download);
        req.write_bytes(self.info_hash.as_slice())?;
        req.write_bytes(self.peer_id.value().as_slice())?;
        req.write_u64(download)?;
        req.write_u64(left)?;
        req.write_u64(info.uploaded.load(Ordering::Acquire))?;
        req.write_u32(event as u32)?;
        req.write_u32(0)?; // ip
        req.write_u32(doro_util::rand::gen_process_key())?;
        req.write_i32(-1)?; // 期望的 peer 数量
        req.write_u16(info.port)?;

        let resp = self.send(&req, -1).await?;

        // 解析响应数据
        anyhow_ge!(resp.len(), MIN_ANNOUNCE_RESP_SIZE, "响应数据长度不足");
        self.check_resp_data(&resp, req_tran_id)?;
        let interval = u32::from_be_slice(&resp[8..12]);
        let leechers = u32::from_be_slice(&resp[12..16]); // 未完成下载的 peer 数
        let seedrs = u32::from_be_slice(&resp[16..20]);

        // 解析 peers 列表
        let peers = tracker::parse_peers_v4(&resp[20..])?;
        // let peers = if self.socket.local_addr()?.is_ipv4() {
        //     tracker::parse_peers_v4(&resp[20..])?
        // } else {
        //     tracker::parse_peers_v6(&resp[20..])?
        // };

        self.next_request_time = datetime::now_secs() + interval as u64;

        Ok(Box::new(Announce {
            interval,
            leechers,
            seedrs,
            peers,
        }))
    }
}