//! 扩展协议的数据 B 编码/解码

use ahash::HashMap;
use bendy::{decoding::{Error, FromBencode, Object, ResultExt}, encoding::ToBencode};
use tracing::warn;

use crate::config::MAX_DEPTH;

/// 握手数据
#[derive(Debug, Eq, PartialEq, Default)]
pub struct HandshakeData {
    /// 扩展协议的 name 和 id 的映射
    pub m: HashMap<String, u8>,

    /// tcp 监听端口
    pub p: Option<u16>,

    /// 客户端名字和版本号
    pub v: Option<String>,

    /// 握手数据接收方的 ip
    pub yourip: Option<Vec<u8>>,

    /// 发送方的 ipv6 地址
    pub ipv6: Option<Vec<u8>>,

    /// 发送方的 ipv4 地址
    pub ipv4: Option<Vec<u8>>,

    /// 请求队列大小，客户端可以接受的未处理请求数
    pub reqq: Option<usize>,

    /// 种子元数据大小
    pub metadata_size: Option<u32>,
}

impl HandshakeData {
    pub fn new() -> Self {
        Self {
            m: HashMap::default(),
            ..Default::default()
        }
    }
}

impl FromBencode for HandshakeData {
    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error>
    where
        Self: Sized,
    {
        let mut m = None;
        let mut p = None;
        let mut v = None;
        let mut yourip = None;
        let mut ipv6 = None;
        let mut ipv4 = None;
        let mut reqq = None;
        let mut metadata_size = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"m", value) => {
                    m = HashMap::decode_bencode_object(value)
                       .context("m")
                       .map(Some)?;
                }
                (b"p", value) => {
                    p = u16::decode_bencode_object(value).context("p").map(Some)?;
                }
                (b"v", value) => {
                    v = String::decode_bencode_object(value)
                       .context("v")
                       .map(Some)?;
                }
                (b"yourip", value) => { 
                    yourip = value.try_into_bytes()
                        .context("yourip")
                        .map(|v| Some(v.to_vec()))?;
                }
                (b"ipv6", value) => {
                    ipv6 = value.try_into_bytes()
                        .context("ipv6")
                        .map(|v| Some(v.to_vec()))?;
                }
                (b"ipv4", value) => {
                    ipv4 = value.try_into_bytes()
                        .context("ipv4")
                        .map(|v| Some(v.to_vec()))?;
                }
                (b"reqq", value) => {
                    reqq = usize::decode_bencode_object(value)
                       .context("reqq")
                       .map(Some)?;
                }
                (b"metadata_size", value) => {
                    metadata_size = u32::decode_bencode_object(value)
                       .context("metadata_size")
                       .map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let m = m.ok_or_else(|| Error::missing_field("m"))?;

        Ok(Self {
            m,
            p,
            v,
            yourip,
            ipv6,
            ipv4,
            reqq,
            metadata_size,
        })
    }
}

impl ToBencode for HandshakeData {
    const MAX_DEPTH: usize = MAX_DEPTH;
    
    fn encode(&self, encoder: bendy::encoding::SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        encoder.emit_dict(|mut e| {
            e.emit_pair(b"m", &self.m)?;
            if let Some(p) = self.p {
                e.emit_pair(b"p", p)?;
            }
            if let Some(ref v) = self.v {
                e.emit_pair(b"v", v)?;
            }
            if let Some(ref yourip) = self.yourip {
                e.emit_pair(b"yourip", yourip)?;
            }
            if let Some(ref ipv6) = self.ipv6 {
                e.emit_pair(b"ipv6", ipv6)?;
            }
            if let Some(ref ipv4) = self.ipv4 {
                e.emit_pair(b"ipv4", ipv4)?;
            }
            if let Some(reqq) = self.reqq {
                e.emit_pair(b"reqq", reqq)?;
            }
            if let Some(metadata_size) = self.metadata_size {
                e.emit_pair(b"metadata_size", metadata_size)?;
            }
            Ok(())
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum MetadataType {
    /// 请求数据
    Request,

    /// 响应数据
    Data,

    /// 拒绝数据
    Reject,
}

impl FromBencode for MetadataType {
    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error>
    where
        Self: Sized,
    {
        let msg_type = u8::decode_bencode_object(object)?;
        match msg_type {
            0 => Ok(MetadataType::Request),
            1 => Ok(MetadataType::Data),
            2 => Ok(MetadataType::Reject),
            _ => Err(Error::unexpected_token("MetadataDataType", msg_type)),
        }
    }
}

impl ToBencode for MetadataType {
    const MAX_DEPTH: usize = MAX_DEPTH;
    
    fn encode(&self, encoder: bendy::encoding::SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        match self {
            MetadataType::Request => encoder.emit_int(0),
            MetadataType::Data => encoder.emit_int(1),
            MetadataType::Reject => encoder.emit_int(2),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Metadata {
    /// 消息类型    
    /// 0: Request
    /// 1: Data
    /// 2: Reject
    pub msg_type: MetadataType,

    /// 分片序号      
    pub piece: u32,

    /// 元数据总大小        
    /// 消息类型为 1 时有效
    pub total_size: Option<u32>,
}

impl Metadata {
    pub fn request(piece: u32) -> Self {
        Self {
            msg_type: MetadataType::Request,
            piece,
            total_size: None,
        }
    }
}

impl FromBencode for Metadata {
    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error>
    where
        Self: Sized,
    {
        let mut msg_type = None;
        let mut piece = None;
        let mut total_size = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"msg_type", value) => {
                    msg_type = MetadataType::decode_bencode_object(value)
                       .context("msg_type")
                       .map(Some)?;
                }
                (b"piece", value) => {
                    piece = u32::decode_bencode_object(value)
                       .context("piece")
                       .map(Some)?;
                }
                (b"total_size", value) => {
                    total_size = u32::decode_bencode_object(value)
                       .context("total_size")
                       .map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let msg_type = msg_type.ok_or_else(|| Error::missing_field("msg_type"))?;
        let piece = piece.ok_or_else(|| Error::missing_field("piece"))?;

        if msg_type == MetadataType::Data && total_size.is_none() {
            return Err(Error::missing_field("total_size"));
        }

        Ok(Self {
            msg_type,
            piece,
            total_size,
        })
    }
}

impl ToBencode for Metadata {
    const MAX_DEPTH: usize = MAX_DEPTH;
    
    fn encode(&self, encoder: bendy::encoding::SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        encoder.emit_dict(|mut e| {
            e.emit_pair(b"msg_type", &self.msg_type)?;
            e.emit_pair(b"piece", self.piece)?;
            if let Some(total_size) = self.total_size {
                e.emit_pair(b"total_size", total_size)?;
            }
            Ok(())
        })
    }
}