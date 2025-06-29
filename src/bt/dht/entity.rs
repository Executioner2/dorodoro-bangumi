use alloc::borrow::Cow;
use bendy::decoding::{Error, FromBencode, Object, ResultExt};
use bendy::encoding::{SingleItemEncoder, ToBencode};
use bendy::value::Value;
use tracing::warn;
use crate::bendy_ext::{Bytes2Object, SocketAddrExt};
use crate::bytes::Bytes2Int;

#[derive(Debug, Eq, PartialEq)]
pub struct Host {
    pub id: [u8; 20],
    pub addr: SocketAddrExt,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DHTBase<'a, T> {
    pub a: Option<T>,
    pub r: Option<T>,
    pub q: Option<String>,
    pub t: Value<'a>,
    pub y: String,
    pub p: Option<u16>,
    pub v: Option<Value<'a>>,
    pub ip: Option<SocketAddrExt>,
    pub ipv6: Option<SocketAddrExt>,
    pub ipv4: Option<SocketAddrExt>,
    pub reqq: Option<u32>,
}

impl<'a, T> DHTBase<'a, T> {
    pub fn request(a: T, q: String, t: u16) -> Self {
        Self {
            a: Some(a),
            r: None,
            q: Some(q),
            t: Value::Bytes(Cow::from(t.to_be_bytes().to_vec())),
            y: "q".to_string(),
            p: None,
            v: None,
            ip: None,
            ipv6: None,
            ipv4: None,
            reqq: None,
        }
    }
    
    pub fn t(&self) -> u16 {
        if let Value::Bytes(ref bytes) = self.t {
            u16::from_be_slice(bytes.as_ref())
        } else {
            unreachable!()
        }
    }
}

impl<'a, T: FromBencode> FromBencode for DHTBase<'a, T> {
    fn decode_bencode_object(object: Object) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let mut a = None;
        let mut r = None;
        let mut q = None;
        let mut t = None;
        let mut y = None;
        let mut p = None;
        let mut v = None;
        let mut ip = None;
        let mut ipv6 = None;
        let mut ipv4 = None;
        let mut reqq = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"a", value) => {
                    a = T::decode_bencode_object(value).context("a").map(Some)?;
                }
                (b"r", value) => {
                    r = T::decode_bencode_object(value).context("r").map(Some)?;
                }
                (b"q", value) => {
                    q = String::decode_bencode_object(value)
                        .context("q")
                        .map(Some)?;
                }
                (b"t", value) => {
                    t = Value::decode_bencode_object(value)
                        .context("t")
                        .map(Some)?;
                }
                (b"y", value) => {
                    y = String::decode_bencode_object(value)
                        .context("y")
                        .map(Some)?;
                }
                (b"p", value) => {
                    p = u16::decode_bencode_object(value).context("p").map(Some)?;
                }
                (b"v", value) => {
                    v = Value::decode_bencode_object(value).context("v").map(Some)?;
                }
                (b"ip", value) => {
                    ip = value.try_into_bytes()?.to_object().map(Some)?;
                }
                (b"ipv6", value) => {
                    ipv6 = value.try_into_bytes()?.to_object().map(Some)?;
                }
                (b"ipv4", value) => {
                    ipv4 = value.try_into_bytes()?.to_object().map(Some)?;
                }
                (b"reqq", value) => {
                    reqq = u32::decode_bencode_object(value)
                        .context("reqq")
                        .map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let t = t.ok_or_else(|| Error::missing_field("t"))?;
        let y = y.ok_or_else(|| Error::missing_field("y"))?;
        if a.is_none() && r.is_none() {
            return Err(Error::missing_field("a or r"));
        }
        if a.is_some() && q.is_none() {
            return Err(Error::missing_field("q"));
        }

        Ok(DHTBase {
            a,
            r,
            q,
            t,
            y,
            p,
            v,
            ip,
            ipv6,
            ipv4,
            reqq,
        })
    }
}

impl<'a, T: ToBencode> ToBencode for DHTBase<'a, T> {
    const MAX_DEPTH: usize = 10;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        encoder.emit_dict(|mut e| {
            if self.a.is_some() {
                e.emit_pair(b"a", self.a.as_ref().unwrap())?;
            }
            if self.r.is_some() {
                e.emit_pair(b"r", self.r.as_ref().unwrap())?;
            }
            if self.q.is_some() {
                e.emit_pair(b"q", self.q.as_ref().unwrap())?;
            } else if self.a.is_some() {
                return Err(bendy::encoding::Error::malformed_content(
                    Error::missing_field("missing q field"),
                ));
            }
            e.emit_pair(b"t", &self.t)?;
            e.emit_pair(b"y", &self.y)?;
            Ok(())
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Ping<'a> {
    pub id: Value<'a>,
    pub port: Option<u16>,
}

impl<'a> Ping<'a> {
    pub fn new(id: Value<'a>) -> Self {
        Self { id, port: None }
    }
}

impl<'a> ToBencode for Ping<'a> {
    const MAX_DEPTH: usize = 10;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        encoder.emit_dict(|mut e| {
            e.emit_pair(b"id", &self.id)?;
            if let Some(port) = self.port {
                e.emit_pair(b"port", &port)?;
            }
            Ok(())
        })
    }
}

impl<'a> FromBencode for Ping<'a> {
    fn decode_bencode_object(object: Object) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let mut id = None;
        let mut port = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"id", value) => {
                    id = Value::decode_bencode_object(value)
                        .context("id")
                        .map(Some)?;
                }
                (b"p", value) => {
                    port = u16::decode_bencode_object(value)
                        .context("p")
                        .map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let id = id.ok_or_else(|| Error::missing_field("id"))?;

        Ok(Ping { id, port })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct GetPeersResp<'a> {
    pub id: Value<'a>,
    pub nodes: Option<Vec<Host>>,
    pub values: Option<Vec<SocketAddrExt>>,
    pub token: Value<'a>,
    pub p: Option<u16>,
}

impl<'a> FromBencode for GetPeersResp<'a> {
    fn decode_bencode_object(object: Object) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let mut id = None;
        let mut nodes = None;
        let mut token = None;
        let mut values = None;
        let mut p = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"id", value) => {
                    id = Value::decode_bencode_object(value)
                        .context("id")
                        .map(Some)?;
                }
                (b"nodes", Object::Bytes(value)) => {
                    nodes = Some(value.to_object()?);
                }
                (b"values", value) => {
                    values = Vec::decode_bencode_object(value)
                        .context("values")
                        .map(Some)?;
                }
                (b"token", value) => {
                    token = Value::decode_bencode_object(value)
                        .context("token")
                        .map(Some)?;
                }
                (b"p", value) => {
                    p = u16::decode_bencode_object(value).context("p").map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let id = id.ok_or_else(|| Error::missing_field("id"))?;
        let token = token.ok_or_else(|| Error::missing_field("token"))?;

        Ok(GetPeersResp {
            id,
            nodes,
            values,
            token,
            p
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct GetPeersReq<'a> {
    pub id: Value<'a>,
    pub info_hash: Value<'a>,
}

impl<'a> GetPeersReq<'a> {
    pub fn new(id: Value<'a>, info_hash: Value<'a>) -> Self {
        Self { id, info_hash }
    }
}

impl<'a> ToBencode for GetPeersReq<'a> {
    const MAX_DEPTH: usize = 10;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        encoder.emit_dict(|mut e| {
            e.emit_pair(b"id", &self.id)?;
            e.emit_pair(b"info_hash", &self.info_hash)?;
            Ok(())
        })
    }
}
