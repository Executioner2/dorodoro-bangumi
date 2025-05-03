use std::net::SocketAddr;
use std::sync::Arc;

pub struct Config {
    inner: Arc<ConfigInner>,
}

struct ConfigInner {
    /// 信道大小
    channel_buffer: usize,

    /// tcp server 监听地址
    tcp_server_addr: SocketAddr,

    /// 分片大小
    sharding_size: u32,

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
 
    /// 每个 torrent 的 peer 配额
    torrent_peer_conn_limit: usize 
}

impl Clone for Config {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ConfigInner {
                channel_buffer: 100,
                tcp_server_addr: "127.0.0.1:3300".parse().unwrap(),
                sharding_size: 1 << 14,
                con_req_piece_limit: 5,
                sucessed_recv_piece: 64, // 按照一次响应 16384 个字节，64 次响应成功，即为响应了 1MB 的数据
                buf_limit: 16 << 20, // 16MB 的写入缓存
                hash_chunk_size: 512,
                hash_concurrency: 1, // 默认就一个
                peer_conn_limit: 500,
                torrent_peer_conn_limit: 100,
            }),
        }
    }

    pub fn set_channel_buffer(mut self, channel_buffer: usize) -> Self {
        assert!(channel_buffer > 0, "Channel buffer must be greater than 0");
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.channel_buffer = channel_buffer;
        });
        self
    }

    pub fn set_tcp_server_addr(mut self, tcp_server_addr: SocketAddr) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.tcp_server_addr = tcp_server_addr;
        });
        self
    }
    
    pub fn set_sharding_size(mut self, sharding_size: u32) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.sharding_size = sharding_size;
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
    
    pub fn set_torrent_peer_conn_limit(mut self, limit: usize) -> Self {
        Arc::get_mut(&mut self.inner).map(|inner| {
            inner.torrent_peer_conn_limit = limit;
        });
        self
    }

    pub fn channel_buffer(&self) -> usize {
        self.inner.channel_buffer
    }

    pub fn tcp_server_addr(&self) -> SocketAddr {
        self.inner.tcp_server_addr
    }
    
    pub fn sharding_size(&self) -> u32 {
        self.inner.sharding_size
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
    
    pub fn torrent_peer_conn_limit(&self) -> usize {
        self.inner.torrent_peer_conn_limit
    }
}
