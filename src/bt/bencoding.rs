//! BEncoding 实现。原项目地址：https://github.com/max-muoto/bencoding-rs/tree/master
//! 修改如下：
//! - 用 Bytes 代替原始字节流
//! - 保留字节块，便于后续对 info 的 sha1 计算
//! - 暴露 ParseError，便于错误处理
//! - 实现编码功能
//!
//! TODO - 这个鬼东西以后一定找时间重构下

#[cfg(test)]
mod tests;

use crate::Integer;
use bytes::{BufMut, Bytes, BytesMut};
use hashlink::LinkedHashMap;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};

/// Possible errors that can occur during bencode parsing.
#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    /// Indicates an invalid byte was encountered at the given position.
    InvalidByte(usize),
    /// Indicates the end of the stream was reached unexpectedly.
    UnexpectedEndOfStream,
    /// Indicates the stream contained invalid UTF-8.
    InvalidUtf8,
    /// 类型转换错误
    TransformError
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            ParseError::InvalidByte(pos) => write!(f, "Invalid byte at position {}", pos),
            ParseError::UnexpectedEndOfStream => write!(f, "Unexpected end of stream"),
            ParseError::InvalidUtf8 => write!(f, "Invalid UTF-8"),
            ParseError::TransformError => write!(f, "Transform Error"),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

#[derive(Eq, PartialEq)]
pub struct BEncode {
    bytes: Bytes,       // 原始字节流
    value: BencodeItem, // 编码后的值
}

impl Debug for BEncode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.value)
    }
}

impl BEncode {
    fn new(bytes: Bytes, value: BencodeItem) -> Self {
        Self { bytes, value }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub trait BEncoder {
    fn encode(self) -> Bytes;
}

impl BEncoder for &[u8] {
    fn encode(self) -> Bytes {
        let binding = self.len().to_string();
        let mut bytes = BytesMut::with_capacity(binding.as_bytes().len() + 1 + self.len()); // 1个字节的 ':'
        bytes.put_slice(binding.as_bytes());
        bytes.put_u8(b':' as u8);
        bytes.put_slice(self);
        bytes.freeze()
    }
}

impl BEncoder for &str {
    fn encode(self) -> Bytes {
        self.as_bytes().encode()
    }
}

impl<T> BEncoder for T
where
    T: Integer + ToString,
{
    fn encode(self) -> Bytes {
        let binding = self.to_string();
        let data = binding.as_bytes();
        let mut bytes = BytesMut::with_capacity(data.len() + 2); // 1个字节的 'i' 和 1个字节的 'e'
        bytes.put_u8('i' as u8);
        bytes.put_slice(data);
        bytes.put_u8(b'e' as u8);
        bytes.freeze()
    }
}

impl<T> BEncoder for Vec<T>
where
    T: BEncoder,
{
    fn encode(self) -> Bytes {
        let mut bytes = BytesMut::new();
        bytes.put_u8(b'l' as u8);
        for item in self {
            bytes.put_slice(&item.encode());
        }
        bytes.put_u8(b'e' as u8);
        bytes.freeze()
    }
}

impl<T> BEncoder for LinkedHashMap<String, T>
where
    T: BEncoder,
{
    fn encode(self) -> Bytes {
        let mut bytes = BytesMut::new();
        bytes.put_u8(b'd' as u8);
        for (key, value) in self.into_iter() {
            bytes.put_slice(&key.as_bytes().encode());
            bytes.put_slice(&value.encode())
        }
        bytes.put_u8(b'e' as u8);
        bytes.freeze()
    }
}

/// Represents a bencode value.
#[derive(PartialEq, Eq, Debug)]
pub enum BencodeItem {
    /// Represents an integer value.
    Int(i64),
    /// Represents a string value.
    Str(Bytes),
    /// Represents a list of bencode values.
    List(Vec<BEncode>),
    /// Represents a dictionary of bencode values.
    Dict(HashMap<String, BEncode>),
}

impl BEncode {
    /// Returns the integer value if this is a `BEncode::Int`.
    ///
    /// # Returns
    ///
    /// An `Option` containing the integer value or `None` if this is not a `BEncode::Int`.
    pub fn as_int(&self) -> Option<i64> {
        match self.value {
            BencodeItem::Int(value) => Some(value),
            _ => None,
        }
    }

    /// 返回原始字节块
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..]
    }

    /// Returns the string value if this is a `BEncode::Str`.
    ///
    /// # Returns
    ///
    /// An `Option` containing the string value or `None` if this is not a `BEncode::Str`.
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            BencodeItem::Str(byte) => Some(std::str::from_utf8(&byte[..]).unwrap()),
            _ => None,
        }
    }

    /// 返回内容的字节串（也就是不包含起始字符和结束字符）
    ///
    /// PS. 仅限字符串类型
    pub fn as_bytes_conetnt(&self) -> Option<&[u8]> {
        match &self.value {
            BencodeItem::Str(byte) => Some(&byte[..]),
            _ => None,
        }
    }

    /// Returns the list value if this is a `BEncode::List`.
    ///
    /// # Returns
    ///
    /// An `Option` containing the list value or `None` if this is not a `BEncode::List`.
    pub fn as_list(&self) -> Option<&Vec<BEncode>> {
        match &self.value {
            BencodeItem::List(value) => Some(value),
            _ => None,
        }
    }

    /// Returns the dictionary value if this is a `BEncode::Dict`.
    ///
    /// # Returns
    ///
    /// An `Option` containing the dictionary value or `None` if this is not a `BEncode::Dict`.
    pub fn as_dict(&self) -> Option<&HashMap<String, BEncode>> {
        match &self.value {
            BencodeItem::Dict(value) => Some(value),
            _ => None,
        }
    }
}

pub trait BEncodeHashMap {
    fn get_int(&self, key: &str) -> Option<i64>;
    fn get_str(&self, key: &str) -> Option<&str>;
    fn as_bytes_conetnt(&self, key: &str) -> Option<&[u8]>;
    fn get_list(&self, key: &str) -> Option<&Vec<BEncode>>;
    fn get_dict(&self, key: &str) -> Option<&HashMap<String, BEncode>>;
    fn get_bytes(&self, key: &str) -> Option<&[u8]>;
}

impl BEncodeHashMap for HashMap<String, BEncode> {

    fn get_int(&self, key: &str) -> Option<i64> {
        self.get(key).map_or(None, |value| value.as_int())
    }

    fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).map_or(None, |value| value.as_str())
    }

    fn as_bytes_conetnt(&self, key: &str) -> Option<&[u8]> {
        self.get(key).map_or(None, |value| value.as_bytes_conetnt())
    }

    fn get_list(&self, key: &str) -> Option<&Vec<BEncode>> {
        self.get(key).map_or(None, |value| value.as_list())
    }

    fn get_dict(&self, key: &str) -> Option<&HashMap<String, BEncode>> {
        self.get(key).map_or(None, |value| value.as_dict())
    }

    fn get_bytes(&self, key: &str) -> Option<&[u8]> {
        self.get(key).map(|value| value.as_bytes())
    }
}

struct Decoder {
    stream: Bytes,
    pos: usize,
}

impl Decoder {
    pub fn new(stream: Bytes) -> Self {
        Decoder { stream, pos: 0 }
    }

    pub fn decode(&mut self) -> Result<BEncode, ParseError> {
        self.parse()
    }

    fn parse(&mut self) -> Result<BEncode, ParseError> {
        if self.pos >= self.stream.len() {
            return Err(ParseError::UnexpectedEndOfStream);
        }

        let curr_byte = self.stream[self.pos];
        match curr_byte {
            b'd' => self.parse_dict(),
            b'l' => self.parse_list(),
            b'i' => self.parse_int(),
            b'0'..=b'9' => self.parse_str(),
            _ => Err(ParseError::InvalidByte(self.pos)),
        }
    }

    fn parse_list(&mut self) -> Result<BEncode, ParseError> {
        let mut list: Vec<BEncode> = Vec::new();
        let start_pos = self.pos;
        self.pos += 1; // Skip the 'l'
        while self.stream[self.pos] != b'e' {
            let parsed = self.parse()?;
            list.push(parsed);
        }
        self.pos += 1; // Skip the 'e'
        Ok(BEncode::new(
            self.stream.slice(start_pos..self.pos),
            BencodeItem::List(list),
        ))
    }

    fn parse_dict(&mut self) -> Result<BEncode, ParseError> {
        let mut dict: HashMap<String, BEncode> = HashMap::new();
        let start_pos = self.pos;
        self.pos += 1; // Skip the 'd'
        while self.stream[self.pos] != b'e' {
            let key = match self.parse_str()? {
                BEncode {
                    value: BencodeItem::Str(bytes),
                    ..
                } => bytes,
                _ => return Err(ParseError::InvalidByte(self.pos)),
            };
            let value = self.parse()?;
            let key = match String::from_utf8(key.to_vec()) {
                Ok(s) => s,
                Err(_) => return Err(ParseError::InvalidUtf8),
            };
            dict.insert(key, value);
        }
        self.pos += 1; // Skip the 'e'
        Ok(BEncode::new(
            self.stream.slice(start_pos..self.pos),
            BencodeItem::Dict(dict),
        ))
    }

    fn parse_str(&mut self) -> Result<BEncode, ParseError> {
        let mut str_size: usize = 0;
        let start_pos = self.pos;
        while self.stream[self.pos] != b':' {
            if self.stream[self.pos].is_ascii_digit() {
                str_size = str_size * 10 + (self.stream[self.pos] - b'0') as usize;
            } else {
                return Err(ParseError::InvalidByte(self.pos));
            }
            self.pos += 1;
        }
        self.pos += 1;
        if self.pos + str_size > self.stream.len() {
            return Err(ParseError::UnexpectedEndOfStream);
        }

        let string = Bytes::from(self.stream.slice(self.pos..self.pos + str_size));
        self.pos += str_size;
        Ok(BEncode::new(
            self.stream.slice(start_pos..self.pos),
            BencodeItem::Str(string),
        ))
    }

    fn parse_int(&mut self) -> Result<BEncode, ParseError> {
        let start_pos = self.pos;
        self.pos += 1; // Skip the 'i'
        let mut is_negative = false;
        if self.stream[self.pos] == b'-' {
            is_negative = true;
            self.pos += 1;
        }

        let mut curr_int: i64 = 0;
        while self.stream[self.pos] != b'e' {
            if self.stream[self.pos].is_ascii_digit() {
                curr_int = curr_int * 10 + (self.stream[self.pos] - b'0') as i64;
            } else {
                return Err(ParseError::InvalidByte(self.pos));
            }
            self.pos += 1;
        }

        self.pos += 1;

        if is_negative {
            curr_int = -curr_int;
        }

        Ok(BEncode::new(
            self.stream.slice(start_pos..self.pos),
            BencodeItem::Int(curr_int),
        ))
    }
}

/// 对一个字节流进行 BEncode 解码。输出 BEncode
pub fn decode(stream: Bytes) -> Result<BEncode, ParseError> {
    Decoder::new(stream.into()).decode()
}

/// 编码一个实现了 BEncoder trait 的实例。输出 Bytes
pub fn encode<T: BEncoder>(data: T) -> Bytes {
    data.encode()
}
