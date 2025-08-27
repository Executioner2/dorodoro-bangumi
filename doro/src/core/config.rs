use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ===========================================================================
// 写死的配置值，一般也不会改的
// ===========================================================================

/// channel 大小
pub const CHANNEL_BUFFER: usize = 100;

/// 数据库连接池大小
pub const DATABASE_CONN_LIMIT: usize = 10;

/// bencode 编码的最大深度
pub const MAX_DEPTH: usize = 10;

/// 等待的 peer 上限，如果超过这个数量，就不会进行 dht 扫描
pub const DHT_WAIT_PEER_LIMIT: usize = 20;

/// 期望每次 dht 扫描能找到 25 个 peer
pub const DHT_EXPECT_PEERS: usize = 25;

/// 每间隔一分钟扫描一次 peers
pub const DHT_FIND_PEERS_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct Config {
    inner: Arc<ConfigInner>,
}

#[derive(Serialize, Deserialize)]
pub struct ClientAuth {
    pub username: String,
    pub password: String,
}

impl ClientAuth {
    pub fn init() -> Self {
        Self {
            username: "admin".to_string(),
            password: "admin".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ConfigInner {
    /// tcp server 监听地址
    tcp_server_addr: SocketAddr,

    /// udp server 监听地址
    udp_server_addr: SocketAddr,

    /// udp 包大小限制
    udp_packet_limit: usize,

    /// 块大小
    block_size: u32,

    /// 单个 peer 同时请求下载的分片的数量
    con_req_piece_limit: usize,

    /// 在成功获取到 n 个 piece 响应后，增加一个分片请求
    sucessed_recv_piece: usize,

    /// 刷入磁盘的缓存上限
    buf_limit: usize,

    /// 计算 hash 值时，一次读取的 chunk 大小
    hash_chunk_size: usize,

    /// hash 计算的队列长度，队列内的都是并发计算
    hash_concurrency: usize,

    /// 总的 peer 配额
    peer_conn_limit: usize,

    /// 每个 torrent 的 lt peer 配额
    torrent_lt_peer_conn_limit: usize,

    /// 每个 torrent 的临时 peer 配额
    torrent_temp_peer_conn_limit: usize,

    /// peer 链接超时设定
    peer_connection_timeout: Duration,

    /// 默认下载目录
    default_download_dir: PathBuf,

    /// 认证信息
    client_auth: ClientAuth,

    /// rss 订阅并发刷新量
    rss_refresh_concurrency: usize,

    /// 错误分片上限
    error_piece_limit: u32,

    /// 单个 peer 异步任务数量限制
    async_task_limit: usize,

    /// 总的异步任务池大小
    async_task_pool_size: usize,

    /// 单个任务的 peer 异步起动数量限制
    async_peer_start_limit: usize,

    /// 异步 peer 启动池大小
    async_peer_start_pool_size: usize,

    /// 种子元数据大小限制
    metadata_size_limit: u32,
}

impl Default for ConfigInner {
    fn default() -> Self {
        Self {
            tcp_server_addr: "0.0.0.0:3300".parse().unwrap(),
            udp_server_addr: "0.0.0.0:3300".parse().unwrap(),
            udp_packet_limit: 65535,
            block_size: 1 << 14,
            con_req_piece_limit: 100,
            sucessed_recv_piece: 64, // 按照一次响应 16384 个字节，64 次响应成功，即为响应了 1MB 的数据
            buf_limit: 16 << 20,     // 16MB 的写入缓存
            hash_chunk_size: 512,
            hash_concurrency: 1, // 默认就一个
            peer_conn_limit: 500,
            torrent_lt_peer_conn_limit: 10,
            torrent_temp_peer_conn_limit: 2,
            peer_connection_timeout: Duration::from_secs(5),
            default_download_dir: PathBuf::from("./download/"),
            client_auth: ClientAuth::init(),
            rss_refresh_concurrency: 10,
            error_piece_limit: 3,
            async_task_limit: 25,
            async_task_pool_size: 2500,
            async_peer_start_limit: 3,
            async_peer_start_pool_size: 300,
            metadata_size_limit: 10 << 20, // 10MB
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_inner(inner: ConfigInner) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn inner(&self) -> &ConfigInner {
        &self.inner
    }

    pub fn set_tcp_server_addr(mut self, tcp_server_addr: SocketAddr) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.tcp_server_addr = tcp_server_addr;
        }
        self
    }

    pub fn set_udp_server_addr(mut self, udp_server_addr: SocketAddr) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.udp_server_addr = udp_server_addr;
        }
        self
    }

    pub fn set_udp_packet_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.udp_packet_limit = limit;
        }
        self
    }

    pub fn set_block_size(mut self, block_size: u32) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.block_size = block_size;
        }
        self
    }

    pub fn set_con_req_piece_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.con_req_piece_limit = limit;
        }
        self
    }

    pub fn set_sucessed_recv_piece(mut self, num: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.sucessed_recv_piece = num;
        }
        self
    }

    pub fn set_buf_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.buf_limit = limit;
        }
        self
    }

    pub fn set_hash_chunk_size(mut self, size: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.hash_chunk_size = size;
        }
        self
    }

    pub fn set_hash_concurrency(mut self, len: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.hash_concurrency = len;
        }
        self
    }

    pub fn set_peer_conn_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.peer_conn_limit = limit;
        }
        self
    }

    pub fn set_torrent_lt_peer_conn_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.torrent_lt_peer_conn_limit = limit;
        }
        self
    }

    pub fn set_torrent_temp_peer_conn_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.torrent_temp_peer_conn_limit = limit;
        }
        self
    }

    pub fn set_peer_connection_timeout(mut self, timeout: Duration) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.peer_connection_timeout = timeout;
        }
        self
    }

    pub fn set_default_download_dir(mut self, dir: PathBuf) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.default_download_dir = dir;
        }
        self
    }

    pub fn set_client_auth(mut self, auth: ClientAuth) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.client_auth = auth;
        }
        self
    }

    pub fn set_rss_refresh_concurrency(mut self, concurrency: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.rss_refresh_concurrency = concurrency;
        }
        self
    }

    pub fn set_error_piece_limit(mut self, limit: u32) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.error_piece_limit = limit;
        }
        self
    }

    pub fn set_async_task_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.async_task_limit = limit;
        }
        self
    }

    pub fn set_async_task_pool_size(mut self, size: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.async_task_pool_size = size;
        }
        self
    }

    pub fn set_async_peer_start_limit(mut self, limit: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.async_peer_start_limit = limit;
        }
        self
    }

    pub fn set_async_peer_start_pool_size(mut self, size: usize) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.async_peer_start_pool_size = size;
        }
        self
    }

    pub fn set_metadata_size_limit(mut self, limit: u32) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.metadata_size_limit = limit;
        }
        self
    }

    pub fn tcp_server_addr(&self) -> SocketAddr {
        self.inner.tcp_server_addr
    }

    pub fn udp_server_addr(&self) -> SocketAddr {
        self.inner.udp_server_addr
    }

    pub fn udp_packet_limit(&self) -> usize {
        self.inner.udp_packet_limit
    }

    pub fn block_size(&self) -> u32 {
        self.inner.block_size
    }

    pub fn con_req_piece_limit(&self) -> usize {
        self.inner.con_req_piece_limit
    }

    pub fn sucessed_recv_piece(&self) -> usize {
        self.inner.sucessed_recv_piece
    }

    pub fn buf_limit(&self) -> usize {
        self.inner.buf_limit
    }

    pub fn hash_chunk_size(&self) -> usize {
        self.inner.hash_chunk_size
    }

    pub fn hash_concurrency(&self) -> usize {
        self.inner.hash_concurrency
    }

    pub fn peer_conn_limit(&self) -> usize {
        self.inner.peer_conn_limit
    }

    pub fn torrent_lt_peer_conn_limit(&self) -> usize {
        self.inner.torrent_lt_peer_conn_limit
    }

    pub fn torrent_temp_peer_conn_limit(&self) -> usize {
        self.inner.torrent_temp_peer_conn_limit
    }

    pub fn torrent_peer_conn_limit(&self) -> usize {
        self.inner.torrent_lt_peer_conn_limit + self.inner.torrent_temp_peer_conn_limit
    }

    pub fn peer_connection_timeout(&self) -> Duration {
        self.inner.peer_connection_timeout
    }

    pub fn default_download_dir(&self) -> &PathBuf {
        &self.inner.default_download_dir
    }

    pub fn client_auth(&self) -> &ClientAuth {
        &self.inner.client_auth
    }

    pub fn rss_refresh_concurrency(&self) -> usize {
        self.inner.rss_refresh_concurrency
    }

    pub fn error_piece_limit(&self) -> u32 {
        self.inner.error_piece_limit
    }

    pub fn async_task_limit(&self) -> usize {
        self.inner.async_task_limit
    }

    pub fn async_task_pool_size(&self) -> usize {
        self.inner.async_task_pool_size
    }

    pub fn async_peer_start_limit(&self) -> usize {
        self.inner.async_peer_start_limit
    }

    pub fn async_peer_start_pool_size(&self) -> usize {
        self.inner.async_peer_start_pool_size
    }

    pub fn metadata_size_limit(&self) -> u32 {
        self.inner.metadata_size_limit
    }
}
