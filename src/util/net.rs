use crate::buffer::ByteBuffer;
use bytes::Bytes;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};
use tracing::{error, trace};

pub struct ReaderHandle<'a, T: AsyncRead + Unpin> {
    stream: &'a mut T,
    buf: ByteBuffer,
    read_count: usize,
    addr: SocketAddr,
}

impl<'a, T: AsyncRead + Unpin> ReaderHandle<'a, T> {
    pub fn new(stream: &'a mut T, addr: SocketAddr, read_len: usize) -> Self {
        Self {
            stream,
            buf: ByteBuffer::new(read_len),
            read_count: 0,
            addr,
        }
    }

    pub fn reset(&mut self, read_len: usize) {
        self.buf = ByteBuffer::new(read_len);
        self.read_count = 0;
    }
}

impl<'a, T: AsyncRead + Unpin> Future for ReaderHandle<'_, T> {
    type Output = Result<Bytes, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let reader = unsafe { self.get_unchecked_mut() };
        if reader.buf.len() == 0 {
            return Poll::Ready(Ok(Bytes::new()));
        }

        loop {
            let mut read_buf = ReadBuf::new(&mut reader.buf[reader.read_count..]);
            match Pin::new(&mut reader.stream).poll_read(cx, &mut read_buf) {
                Poll::Ready(Ok(())) => {
                    let filled = read_buf.filled().len();
                    if filled == 0 {
                        trace!("客户端主动说bye-bye");
                        return Poll::Ready(Err(io::ErrorKind::ConnectionReset.into()));
                    }
                    reader.read_count += filled;
                    if reader.read_count >= reader.buf.capacity() {
                        let buf = reader.buf.take();
                        return Poll::Ready(Ok(buf));
                    }
                }
                Poll::Ready(Err(e)) => {
                    error!(
                        "因神秘力量，和客户端失去了联系\t{}，addr: {}",
                        e, reader.addr
                    );
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
