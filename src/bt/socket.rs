//! socket 相关扩展

use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use crate::bt::pe_crypto;
use crate::bt::pe_crypto::Rc4Cipher;

pub enum Crypto {
    /// 明文传输
    Plaintext,
    
    /// RC4 加密传输（local_cipher, remote_cipher）
    Rc4(Rc4Cipher)
}  

/// TcpStream 的扩展
pub struct TcpStreamExt {
    /// TcpStream
    stream: TcpStream,
    
    /// 本端加密方式
    lc: Crypto,
    
    /// 对端加密方式
    rc: Crypto
}

impl TcpStreamExt {
    pub fn new(stream: TcpStream, lc: Crypto, rc: Crypto) -> Self {
        Self {
            stream,
            lc,
            rc
        }
    }
    
    pub async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.rc {
            Crypto::Plaintext => self.stream.read(buf).await,
            Crypto::Rc4(rc4) => {
                let size = self.stream.read(buf).await?;
                pe_crypto::decrypt_payload(rc4, &mut buf[0..size]);
                Ok(size)
            }
        }
    }
    
    pub async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match &mut self.lc {
            Crypto::Plaintext => self.stream.write_all(buf).await,
            Crypto::Rc4(rc4) => {
                let mut buf = buf.to_vec();
                pe_crypto::encrypt_payload(rc4, &mut buf);
                self.stream.write_all(&buf).await
            }   
        }
    }
    
    pub async fn readable(&self) -> std::io::Result<()> {
        self.stream.readable().await
    }
    
    pub fn into_split(self) -> (OwnedReadHalfExt, OwnedWriteHalfExt) {
        let (r, w) = self.stream.into_split();
        (OwnedReadHalfExt { inner: r, crypto: self.rc }, OwnedWriteHalfExt { inner: w, crypto: self.lc })
    }
}

pub struct OwnedReadHalfExt {
    inner: OwnedReadHalf,
    crypto: Crypto
}

impl AsyncRead for OwnedReadHalfExt {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = unsafe { self.get_unchecked_mut() };
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                match &mut this.crypto {
                    Crypto::Plaintext => Poll::Ready(Ok(())),
                    Crypto::Rc4(rc4) => {
                        pe_crypto::decrypt_payload(rc4, buf.filled_mut());
                        Poll::Ready(Ok(()))
                    }
                }
            },
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending
        }
    }
}


pub struct OwnedWriteHalfExt {
    inner: OwnedWriteHalf,
    crypto: Crypto
}

impl OwnedWriteHalfExt {
    pub async fn write_all(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        match &mut self.crypto {
            Crypto::Plaintext => self.inner.write_all(buf).await,
            Crypto::Rc4(rc4) => {
                pe_crypto::encrypt_payload(rc4, buf);
                self.inner.write_all(&buf).await
            }
        }
    }
}

