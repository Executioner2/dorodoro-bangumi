//! 主要对 udp socket 的封装，包括创建、发送、接收等操作。SocketBuilder
//! 会创建一个共享的 udp socket。供多个线程使用同一个 socket。
//!
//! 这个模块其实已经过时不会再用到了。它在和 `一个 Tracker 一个 Socket` 的
//! 比拼中败下阵来，在差不多的吞吐量下，它会比后者使用更多的 CPU 资源。

use crate::bt::constant::udp_tracker::{DEFAULT_ADDR, MAX_PAYLOAD_SIZE};
use crate::tracker::udp_tracker::buffer::ByteBuffer;
use crate::tracker::udp_tracker::error;
use bytes::Bytes;
use error::Result;
use std::net::UdpSocket;
use std::sync::Arc;

pub struct SocketBuilder<'a> {
    local_addr: &'a str,
    read_timeout: Option<u64>,
    write_timeout: Option<u64>,
    nonblocking: bool,
}

impl<'a> SocketBuilder<'a> {
    pub fn new() -> Self {
        Self {
            local_addr: DEFAULT_ADDR,
            read_timeout: None,
            write_timeout: None,
            nonblocking: false,
        }
    }

    pub fn local_addr(mut self, local_addr: &'a str) -> Self {
        self.local_addr = local_addr;
        self
    }

    pub fn read_timeout(mut self, read_timeout: Option<u64>) -> Self {
        self.read_timeout = read_timeout;
        self
    }

    pub fn write_timeout(mut self, write_timeout: Option<u64>) -> Self {
        self.write_timeout = write_timeout;
        self
    }

    pub fn nonblocking(mut self, nonblocking: bool) -> Self {
        self.nonblocking = nonblocking;
        self
    }

    pub fn build(self) -> Result<SocketArc> {
        let socket = UdpSocket::bind(self.local_addr)?;
        if let Some(read_timeout) = self.read_timeout {
            socket.set_read_timeout(Some(std::time::Duration::from_millis(read_timeout)))?;
        }
        if let Some(write_timeout) = self.write_timeout {
            socket.set_write_timeout(Some(std::time::Duration::from_millis(write_timeout)))?;
        }
        if self.nonblocking {
            socket.set_nonblocking(true)?;
        }
        Ok(SocketArc::new(socket))
    }
}

/// 套接字的公共封装，便于多线程下，可以共享同一个套接字
pub struct SocketArc {
    socket: Arc<UdpSocket>,
}

impl Clone for SocketArc {
    fn clone(&self) -> Self {
        Self {
            socket: self.socket.clone(),
        }
    }
}

impl SocketArc {
    /// 创建一个新的套接字，只能由 SocketBuilder 构建
    fn new(socket: UdpSocket) -> Self {
        Self {
            socket: Arc::new(socket),
        }
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
    pub fn send_recv(&self, data: &[u8], target: &str, expect_size: isize) -> Result<Bytes> {
        self.socket.send_to(data, target)?;
        let expect_size = if expect_size < 0 {
            MAX_PAYLOAD_SIZE
        } else {
            expect_size as usize
        };

        let mut buffer = ByteBuffer::new(expect_size);
        let (size, _socket_addr) = self.socket.recv_from(buffer.as_mut())?;

        // 转换为已初始化的缓冲区
        buffer.resize(size);

        Ok(Bytes::from_owner(buffer))
    }

    /// 判断当前 socket 是否是 ipv4
    pub fn is_ipv4(&self) -> bool {
        self.socket.local_addr().unwrap().is_ipv4()
    }

    /// 判断当前 socket 是否是 ipv6
    pub fn is_ipv6(&self) -> bool {
        self.socket.local_addr().unwrap().is_ipv6()
    }
}
