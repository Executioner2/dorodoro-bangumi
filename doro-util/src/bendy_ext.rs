use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};

use bendy::decoding::{Error, FromBencode, Object};
use bendy::encoding::{SingleItemEncoder, ToBencode};

/// bytes 转为具体对象
pub trait Bytes2Object<T> {
    fn to_object(&self) -> Result<T, Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub struct SocketAddrExt {
    pub addr: SocketAddr,
}

impl From<SocketAddr> for SocketAddrExt {
    fn from(addr: SocketAddr) -> Self {
        Self { addr }
    }
}

impl From<SocketAddrExt> for SocketAddr {
    fn from(val: SocketAddrExt) -> Self {
        val.addr
    }
}

impl Deref for SocketAddrExt {
    type Target = SocketAddr;

    fn deref(&self) -> &Self::Target {
        &self.addr
    }
}

impl DerefMut for SocketAddrExt {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.addr
    }
}

impl FromBencode for SocketAddrExt {
    fn decode_bencode_object(object: Object) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let bytes = object.try_into_bytes()?;
        if bytes.len() == 6 {
            let bytes: [u8; 6] = bytes.try_into()?;
            Ok(bytes.to_object()?)
        } else if bytes.len() == 18 {
            let bytes: [u8; 18] = bytes.try_into()?;
            Ok(bytes.to_object()?)
        } else {
            Err(Error::unexpected_token(
                "6 or 18 bytes expected",
                format!("Invalid socket address length: {}", bytes.len()),
            ))
        }
    }
}

impl ToBencode for SocketAddrExt {
    const MAX_DEPTH: usize = 10;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        match self.addr {
            SocketAddr::V4(v4) => {
                let mut bytes = Vec::with_capacity(6);
                bytes.extend_from_slice(&v4.ip().octets()); // 4 字节 IP
                bytes.extend_from_slice(&v4.port().to_be_bytes()); // 2 字节端口
                encoder.emit_bytes(&bytes)?;
            }
            SocketAddr::V6(v6) => {
                let mut bytes = Vec::with_capacity(18);
                bytes.extend_from_slice(&v6.ip().octets()); // 16 字节 IP
                bytes.extend_from_slice(&v6.port().to_be_bytes()); // 2 字节端口
                encoder.emit_bytes(&bytes)?;
            }
        }
        Ok(())
    }
}

impl Bytes2Object<SocketAddrExt> for [u8] {
    fn to_object(&self) -> Result<SocketAddrExt, Error> {
        if self.len() == 6 {
            let bytes: [u8; 6] = self.try_into()?;
            Ok(bytes.to_object()?)
        } else if self.len() == 18 {
            let bytes: [u8; 18] = self.try_into()?;
            Ok(bytes.to_object()?)
        } else {
            Err(Error::unexpected_token(
                "6 or 18 bytes expected",
                format!("Invalid socket address length: {}", self.len()),
            ))
        }
    }
}

impl Bytes2Object<SocketAddrExt> for [u8; 6] {
    fn to_object(&self) -> Result<SocketAddrExt, Error> {
        let ip_bytes: [u8; 4] = self[..4].try_into()?;
        let port = u16::from_be_bytes(self[4..6].try_into()?);
        let addr = SocketAddr::from((ip_bytes, port));
        Ok(SocketAddrExt { addr })
    }
}

impl Bytes2Object<SocketAddrExt> for [u8; 18] {
    fn to_object(&self) -> Result<SocketAddrExt, Error> {
        let ip_bytes: [u8; 16] = self[..16].try_into()?;
        let port = u16::from_be_bytes(self[16..18].try_into()?);
        let addr = SocketAddr::from((ip_bytes, port));
        Ok(SocketAddrExt { addr })
    }
}

impl Bytes2Object<Vec<SocketAddrExt>> for [u8] {
    fn to_object(&self) -> Result<Vec<SocketAddrExt>, Error> {
        let chunk_size = if self.len() % 6 == 0 {
            6
        } else if self.len() % 18 == 0 {
            18
        } else {
            return Err(Error::unexpected_token(
                "6 or 18 bytes per socket address expected",
                format!("Invalid socket address list length: {}", self.len()),
            ));
        };

        let mut addrs = Vec::new();

        for data in self.chunks(chunk_size) {
            addrs.push(data.to_object()?);
        }

        Ok(addrs)
    }
}
