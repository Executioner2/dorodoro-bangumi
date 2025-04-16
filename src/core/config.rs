use std::net::SocketAddr;
use std::sync::Arc;

pub struct Config {
    inner: Arc<ConfigInner>,
}

struct ConfigInner {
    channel_buffer: usize,
    tcp_server_addr: SocketAddr,
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

    pub fn channel_buffer(&self) -> usize {
        self.inner.channel_buffer
    }

    pub fn tcp_server_addr(&self) -> SocketAddr {
        self.inner.tcp_server_addr
    }
}
