use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use bincode::{Decode, Encode};
// ===========================================================================
// 写死的配置值，一般也不会改的
// ===========================================================================

/// channel 大小
pub const CHANNEL_BUFFER: usize = 100;

/// 数据库连接池大小
pub const DATABASE_CONN_LIMIT: usize = 10;


#[derive(Clone, Encode, Decode)]
pub struct Config {
    inner: Arc<ConfigInner>,
}

#[derive(Encode, Decode)]
struct ConfigInner {
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
}

impl Config {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ConfigInner {
                tcp_server_addr: "0.0.0.0:3300".parse().unwrap(),
                udp_server_addr: "0.0.0.0:3300".parse().unwrap(),
                udp_packet_limit: 65535,
                block_size: 1 << 14,
                con_req_piece_limit: 100,
                sucessed_recv_piece: 64, // 按照一次响应 16384 个字节，64 次响应成功，即为响应了 1MB 的数据
                buf_limit: 16 << 20, // 16MB 的写入缓存
                hash_chunk_size: 512,
                hash_concurrency: 1, // 默认就一个
                peer_conn_limit: 500,
                // torrent_lt_peer_conn_limit: 100,
                torrent_lt_peer_conn_limit: 5,
                torrent_temp_peer_conn_limit: 1,
                peer_connection_timeout: Duration::from_secs(5),
                default_download_dir: PathBuf::from("./downloads/")
            }),
        }
    }

    pub fn set_tcp_server_addr(mut self, tcp_server_addr: SocketAddr) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.tcp_server_addr = tcp_server_addr;
        });
        self
    }
    
    pub fn set_udp_server_addr(mut self, udp_server_addr: SocketAddr) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.udp_server_addr = udp_server_addr;
        });
        self
    }
    
    pub fn set_udp_packet_limit(mut self, limit: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.udp_packet_limit = limit;
        });
        self
    }
    
    pub fn set_block_size(mut self, block_size: u32) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.block_size = block_size;
        });
        self
    }

    pub fn set_con_req_piece_limit(mut self, limit: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.con_req_piece_limit = limit;
        });
        self
    }

    pub fn set_sucessed_recv_piece(mut self, num: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.sucessed_recv_piece = num;
        });
        self
    }

    pub fn set_buf_limit(mut self, limit: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.buf_limit = limit;
        });
        self
    }

    pub fn set_hash_chunk_size(mut self, size: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.hash_chunk_size = size;
        });
        self
    }

    pub fn set_hash_concurrency(mut self, len: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.hash_concurrency = len;
        });
        self
    }
    
    pub fn set_peer_conn_limit(mut self, limit: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.peer_conn_limit = limit;
        });
        self
    }
    
    pub fn set_torrent_lt_peer_conn_limit(mut self, limit: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.torrent_lt_peer_conn_limit = limit;
        });
        self
    }
    
    pub fn set_torrent_temp_peer_conn_limit(mut self, limit: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.torrent_temp_peer_conn_limit = limit;
        });
        self
    }
    
    pub fn set_peer_connection_timeout(mut self, timeout: Duration) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.peer_connection_timeout = timeout;
        });
        self
    }
    
    pub fn set_default_download_dir(mut self, dir: PathBuf) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.default_download_dir = dir;
        });
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
}
