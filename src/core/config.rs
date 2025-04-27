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
            }),
        }
    }

    pub fn set_channel_buffer(mut self, channel_buffer: usize) -> Self {
        assert!(channel_buffer > 0, "Channel buffer must be greater than 0");
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.channel_buffer = channel_buffer;
        }
        self
    }

    pub fn set_tcp_server_addr(mut self, tcp_server_addr: SocketAddr) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.tcp_server_addr = tcp_server_addr;
        }
        self
    }
    
    pub fn set_sharding_size(mut self, sharding_size: u32) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.sharding_size = sharding_size;
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
}
