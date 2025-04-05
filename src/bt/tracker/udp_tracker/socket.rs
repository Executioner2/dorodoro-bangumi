//! 主要对 udp socket 的封装，包括创建、发送、接收等操作。
use crate::bt::constant::udp_tracker::{DEFAULT_ADDR, MAX_PAYLOAD_SIZE, SOCKET_READ_TIMEOUT};
use crate::tracker::udp_tracker::buffer::ByteBuffer;
use crate::tracker::udp_tracker::error;
use bytes::Bytes;
use error::Result;
use std::net::UdpSocket;
use std::sync::Arc;

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

    /// 创建一个新的套接字
    ///
    /// # Examples
    /// ```
    /// use dorodoro_bangumi::bt::tracker::udp_tracker::socket::SocketArc;
    /// let socket = SocketArc::new().unwrap();
    /// ```
    pub fn new() -> Result<Self> {
        let socket = UdpSocket::bind(DEFAULT_ADDR)?;
        socket.set_read_timeout(Some(SOCKET_READ_TIMEOUT))?;
        Ok(Self {
            socket: Arc::new(socket),
        })
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
}
