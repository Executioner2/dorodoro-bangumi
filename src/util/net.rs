use bytes::{BufMut, Bytes, BytesMut};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};
use tracing::{error, trace};

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
                        trace!("客户端主动说bye-bye");
                        return Poll::Ready(Err(io::ErrorKind::ConnectionReset.into()));
                    }
                    unsafe { this.buf.advance_mut(filled); }
                    if this.buf.len() >= this.buf.capacity() {
                        return Poll::Ready(Ok(this.buf.split().freeze()));
                    }
                }
                Poll::Ready(Err(e)) => {
                    error!("因神秘力量，和客户端失去了联系\t{}，addr: {}", e, this.addr);
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
