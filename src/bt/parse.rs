//! 种子内容下载

use super::bencoding;
use TorrentError::*;
use bencoding::BEncode;
use bytes::Bytes;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::fs;
type Result<T> = std::result::Result<T, TorrentError>;

/// 错误类型
#[derive(Debug)]
pub enum TorrentError {
    InvalidTorrent(&'static str),
    TransformError,
    DecodeError(bencoding::ParseError),
    Utf8Error(std::string::FromUtf8Error),
    FileError(std::io::Error),
}

impl Display for TorrentError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            InvalidTorrent(msg) => write!(f, "Invalid torrent file: {}", msg),
            TransformError => write!(f, "transform error"),
            DecodeError(e) => write!(f, "Decode error: {}", e),
            Utf8Error(e) => write!(f, "UTF8 error: {}", e),
            FileError(e) => write!(f, "File error: {}", e),
        }
    }
}

impl Error for TorrentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DecodeError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<bencoding::ParseError> for TorrentError {
    fn from(e: bencoding::ParseError) -> Self {
        DecodeError(e)
    }
}

impl From<std::string::FromUtf8Error> for TorrentError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        Utf8Error(e)
    }
}

impl From<std::io::Error> for TorrentError {
    fn from(e: std::io::Error) -> Self {
        FileError(e)
    }
}

/// 种子结构体
#[derive(Debug, Hash, Eq, PartialEq)]
pub struct Torrent {
    pub announce: String,                // Tracker地址
    pub announce_list: Vec<Vec<String>>, // Tracker列表
    pub created_by: Option<String>,      // 制作程序
    pub creation_date: u64,              // 创建时间
    pub info: Info,                      // 种子信息
    pub info_hash: [u8; 20],             // 种子信息hash值
    _comment: Option<String>,            // 种子描述
    _encoding: Option<String>,           // 编码方式
}

/// 种子信息结构体
#[derive(Debug, Hash, Eq, PartialEq)]
pub struct Info {
    pub length: u64,         // 文件大小
    pub piece_length: u64,   // 分片大小
    pub pieces: Vec<u8>,     // 每20个字节一块的校验码
    pub name: String,        // 文件名
    pub files: Vec<File>,    // 文件列表
    _md5sum: Option<String>, // 文件md5值
    _private: Option<u8>,    // 是否私有
}

/// 文件结构体，适用于多文件种子
#[derive(Debug, Hash, Eq, PartialEq)]
pub struct File {
    pub length: u64,         // 文件大小
    pub path: Vec<String>,   // 文件路径
    _md5sum: Option<String>, // 文件md5值
}

impl Torrent {
    fn new(
        announce: String,
        announce_list: Vec<Vec<String>>,
        created_by: Option<String>,
        creation_date: u64,
        info: Info,
        info_hash: [u8; 20],
    ) -> Self {
        Self {
            announce,
            announce_list,
            created_by,
            creation_date,
            info,
            info_hash,
            _comment: None,
            _encoding: None,
        }
    }
}

impl Info {
    fn new(
        name: String,
        length: u64,
        piece_length: u64,
        pieces: Vec<u8>,
        files: Vec<File>,
    ) -> Self {
        Self {
            name,
            length,
            piece_length,
            pieces,
            files,
            _md5sum: None,
            _private: None,
        }
    }
}

impl File {
    fn new(length: u64, path: Vec<String>) -> Self {
        Self {
            length,
            path,
            _md5sum: None,
        }
    }
}

/// 解析 announce
fn parse_announce(encode: &HashMap<String, BEncode>) -> Result<String> {
    let announce = encode
        .get("announce")
        .ok_or(InvalidTorrent("缺少announce"))?
        .as_bytes()
        .ok_or(TransformError)?;
    Ok(String::from_utf8(announce.to_vec())?)
}

/// 解析 List<String> 这种格式的数据。重复出现，所以抽出来
fn parse_list_str(encode: &BEncode) -> Result<Vec<String>> {
    encode
        .as_list()
        .ok_or(TransformError)?
        .iter()
        .try_fold(Vec::new(), |mut acc, announce| {
            let announce = announce.as_bytes().ok_or(TransformError)?;
            let announce = String::from_utf8(announce.to_vec())?;
            acc.push(announce);
            Ok(acc)
        })
}

/// 解析 announce-list
fn parse_announce_list(encode: &HashMap<String, BEncode>) -> Result<Vec<Vec<String>>> {
    match encode.get("announce-list") {
        Some(announce_list) => announce_list
            .as_list()
            .ok_or(TransformError)?
            .iter()
            .try_fold(Vec::new(), |mut acc, announce_list| {
                let announce_list = parse_list_str(announce_list)?;
                acc.push(announce_list);
                Ok(acc)
            }),
        None => Ok(Vec::new()),
    }
}

/// 解析 created by
fn parse_created_by(encode: &HashMap<String, BEncode>) -> Result<Option<String>> {
    let created_by = match encode.get("created by") {
        Some(created_by) => Some(String::from_utf8(created_by.as_bytes().ok_or(TransformError)?.to_vec())?),
        None => None,
    };
    Ok(created_by)
}

/// 解析创建时间
fn parse_creation_date(encode: &HashMap<String, BEncode>) -> Result<u64> {
    let creation_date = encode
        .get("creation date")
        .ok_or(InvalidTorrent("缺少creation date"))?
        .as_int()
        .ok_or(TransformError)?;
    if creation_date < 0 {
        return Err(InvalidTorrent("creation date 不能为负数"));
    }
    Ok(creation_date as u64)
}

fn info_hash(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data.to_vec());
    let mut result = [0; 20];
    result.copy_from_slice(&hasher.finalize());
    result
}

/// 解析 info
fn parse_info(encode: &HashMap<String, BEncode>) -> Result<(Info, [u8; 20])> {
    let info_bencode = encode.get("info").ok_or(InvalidTorrent("缺少info"))?;

    let info = info_bencode.as_dict().ok_or(TransformError)?;
    let mut length = match info.get("length") {
        Some(length) => length.as_int().ok_or(TransformError)?,
        None => 0,
    };
    let piece_length = info
        .get("piece length")
        .ok_or(InvalidTorrent("缺少info.piece length"))?
        .as_int()
        .ok_or(TransformError)?;

    if length < 0 || piece_length < 0 {
        return Err(InvalidTorrent("info.length或info.piece length 不能为负数"));
    }

    let pieces = info
        .get("pieces")
        .ok_or(InvalidTorrent("缺少info.pieces"))?
        .as_bytes()
        .ok_or(TransformError)?
        .to_vec();
    let name = info
        .get("name")
        .ok_or(InvalidTorrent("缺少info.name"))?
        .as_bytes()
        .ok_or(TransformError)?;
    let name = String::from_utf8(name.to_vec())?;
    let files = parse_info_files(info)?;

    if length == 0 && !files.is_empty() {
        length = files.iter().fold(0, |acc, file| acc + file.length as i64);
    }

    Ok((
        Info::new(name, length as u64, piece_length as u64, pieces, files),
        info_hash(info_bencode.bytes()),
    ))
}

/// 解析 info.files
fn parse_info_files(info: &HashMap<String, BEncode>) -> Result<Vec<File>> {
    match info.get("files") {
        Some(files) => {
            files
                .as_list()
                .ok_or(TransformError)?
                .iter()
                .try_fold(Vec::new(), |mut acc, file| {
                    let file = file.as_dict().ok_or(TransformError)?;
                    let length = file
                        .get("length")
                        .ok_or(InvalidTorrent("缺少info.files.length"))?
                        .as_int()
                        .ok_or(TransformError)?;
                    let path = parse_list_str(
                        file.get("path")
                            .ok_or(InvalidTorrent("缺少info.files.path"))?,
                    )?;
                    if length < 0 {
                        return Err(InvalidTorrent("info.files.length 不能为负数"));
                    }
                    acc.push(File::new(length as u64, path));
                    Ok(acc)
                })
        }
        None => Ok(Vec::new()),
    }
}

pub trait Parse<T> {
    /// 解析种子文件
    fn parse_torrent(data: T) -> Result<Torrent>;
}

/// 直接传入字节数组
impl Parse<Vec<u8>> for Torrent {
    fn parse_torrent(data: Vec<u8>) -> Result<Torrent> {
        let binding = bencoding::decode(Bytes::from(data))?;
        let encode = binding.as_dict().ok_or(TransformError)?;
        let (info, info_hash) = parse_info(encode)?;
        Ok(Torrent::new(
            parse_announce(encode)?,
            parse_announce_list(encode)?,
            parse_created_by(encode)?,
            parse_creation_date(encode)?,
            info,
            info_hash,
        ))
    }
}

/// 传入文件路径
impl Parse<&str> for Torrent {
    fn parse_torrent(data: &str) -> Result<Torrent> {
        let data = fs::read(data)?;
        Torrent::parse_torrent(data)
    }
}
