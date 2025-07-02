use bytes::{BufMut, Bytes, BytesMut};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use tracing::debug;

pub struct ReaderHandle<'a, T: AsyncRead + Unpin> {
    stream: &'a mut T,
    buf: BytesMut,
    addr: &'a SocketAddr,
}

impl<'a, T: AsyncRead + Unpin> ReaderHandle<'a, T> {
    pub fn new(stream: &'a mut T, addr: &'a SocketAddr, read_len: usize) -> Self {
        Self {
            stream,
            buf: BytesMut::with_capacity(read_len),
            addr,
        }
    }

    pub fn reset(&mut self, read_len: usize) {
        self.buf = BytesMut::with_capacity(read_len);
    }
}

impl<'a, T: AsyncRead + Unpin> Future for ReaderHandle<'_, T> {
    type Output = Result<Bytes, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        if this.buf.capacity() == 0 {
            return Poll::Ready(Ok(Bytes::new()));
        }
        loop {
            let spare = this.buf.spare_capacity_mut();
            let mut read_buf = ReadBuf::uninit(spare);
            match Pin::new(&mut this.stream).poll_read(cx, &mut read_buf) {
                Poll::Ready(Ok(())) => {
                    let filled = read_buf.filled().len();
                    if filled == 0 {
                        debug!("客户端主动说bye-bye");
                        return Poll::Ready(Err(io::ErrorKind::ConnectionAborted.into()));
                    }
                    unsafe {
                        this.buf.advance_mut(filled);
                    }
                    if this.buf.len() >= this.buf.capacity() {
                        return Poll::Ready(Ok(this.buf.split().freeze()));
                    }
                }
                Poll::Ready(Err(e)) => {
                    debug!("因神秘力量，和客户端失去了联系\t{}，addr: {}", e, this.addr);
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// 速率格式化
pub fn rate_formatting<T: Into<u64>>(bw: T) -> (f64, &'static str) {
    let rate: f64;
    let unit: &str;
    let bw = bw.into();
    if bw >= 1 << 20 {
        rate = bw as f64 / 1024.0 / 1024.0;
        unit = "MiB/s";
    } else if bw >= 1024 {
        rate = bw as f64 / 1024.0;
        unit = "KiB/s";
    } else {
        rate = bw as f64;
        unit = "B/s";
    }
    (rate, unit)
}

/// 异步读取扩展的扩展
#[allow(async_fn_in_trait)]
pub trait AsyncReadExtExt: AsyncRead {
    /// 超时读取
    async fn read_with_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;

    /// 超时读取
    async fn read_buf_with_timeout<B: BufMut + ?Sized>(&mut self, buf: &mut B, timeout: Duration) -> io::Result<usize>;

    /// 超时读取指定的长度
    async fn read_exact_with_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;

    /// 判断是否有数据可读
    async fn has_data_available(&mut self) -> io::Result<bool>;
}

impl<T: AsyncRead + Unpin> AsyncReadExtExt for T {
    async fn read_with_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        match tokio::time::timeout(timeout, self.read(buf)).await {
            Ok(Ok(size)) => Ok(size),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "read timeout")),
        }
    }

    async fn read_buf_with_timeout<B: BufMut + ?Sized>(&mut self, buf: &mut B, timeout: Duration) -> io::Result<usize> {
        match tokio::time::timeout(timeout, self.read_buf(buf)).await {
            Ok(Ok(size)) => Ok(size),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "read timeout")),
        }
    }

    async fn read_exact_with_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        match tokio::time::timeout(timeout, self.read_exact(buf)).await {
            Ok(Ok(size)) => Ok(size),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "read timeout")),
        }
    }

    async fn has_data_available(&mut self) -> io::Result<bool> {
        let mut data = [0u8; 1];
        let size = self.read(&mut data).await?;
        Ok(size != 0)
    }
}


#[allow(async_fn_in_trait)]
pub trait TcpStreamExt {
    /// 超时读取指定的长度，并清空缓冲区
    async fn read_extra_with_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;
}

impl TcpStreamExt for tokio::net::TcpStream {
    async fn read_extra_with_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        match tokio::time::timeout(timeout, self.read_exact(buf)).await {
            Ok(Ok(_)) => {
                let mut total_size = 0;
                let mut buf = BytesMut::with_capacity(4096);
                loop {
                    match self.try_read_buf(&mut buf) {
                        Ok(0) => break,
                        Ok(size) => {
                            buf.clear();
                            total_size += size;
                        },
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            break;
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(total_size)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "read timeout")),
        }
    }
}