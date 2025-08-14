//! 种子内容下载

#[cfg(test)]
mod tests;

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use bendy::decoding::{Error, FromBencode, Object, ResultExt};
use bendy::encoding::AsString;
use bincode::{Decode, Encode};
use bytes::Bytes;
use doro_util::if_else;
use sha1::{Digest, Sha1};
use tracing::warn;

/// 种子，多线程共享
#[derive(Debug, Clone, Hash, Eq, PartialEq, Default)]
pub struct TorrentArc {
    inner: Arc<Torrent>,
}

impl TorrentArc {
    pub fn new(torrent: Torrent) -> Self {
        Self {
            inner: Arc::new(torrent),
        }
    }
}

impl TorrentArc {
    pub fn as_ptr(&self) -> *const Torrent {
        self.inner.as_ref() as *const Torrent
    }

    pub fn inner(&self) -> &Torrent {
        &self.inner
    }
}

impl Deref for TorrentArc {
    type Target = Torrent;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// 种子结构体
#[derive(Debug, Hash, Eq, PartialEq, Encode, Decode, Default)]
pub struct Torrent {
    pub announce: String,                // Tracker地址
    pub announce_list: Vec<Vec<String>>, // Tracker列表
    pub created_by: Option<String>,      // 制作程序
    pub creation_date: u64,              // 创建时间
    pub info: Info,                      // 种子信息
    pub info_hash: [u8; 20],             // 种子信息hash值
    pub comment: Option<String>,         // 种子描述
    pub encoding: Option<String>,        // 编码方式
}

impl Torrent {
    pub fn new(info: Info, info_hash: [u8; 20], announces: Vec<String>) -> Self {
        Self {
            announce: "".to_string(),
            announce_list: vec![announces],
            created_by: None,
            creation_date: 0,
            info,
            info_hash,
            comment: None,
            encoding: None,
        }
    }

    /// 获取 Tracker 地址
    pub fn get_trackers(&self) -> Vec<Vec<String>> {
        let mut trackers = vec![vec![self.announce.to_string()]];
        trackers.extend_from_slice(&self.announce_list);
        trackers
    }

    /// 根据分片下标，查询处相关文件
    #[rustfmt::skip]
    pub fn find_file_of_piece_index(
        &self,
        path_buf: &Path,
        piece_index: u32,
        offset: u32,
        len: usize,
    ) -> Vec<BlockInfo> {
        self.info.find_file_of_piece_index(path_buf, piece_index, offset, len)
    }

    /// 分片数量
    pub fn piece_num(&self) -> usize {
        self.info.pieces.len() / 20
    }

    /// bitfield 数组的长度
    pub fn bitfield_len(&self) -> usize {
        (self.piece_num() + 7) >> 3
    }

    /// 获取分片大小
    pub fn piece_length(&self, piece_idx: u32) -> u32 {
        let piece_length = self.info.piece_length;
        let resource_length = self.info.length;
        piece_length.min(resource_length.saturating_sub(piece_idx as u64 * piece_length)) as u32
    }
}

impl FromBencode for Torrent {
    fn decode_bencode_object(object: Object) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let mut announce = None;
        let mut announce_list = None;
        let mut created_by = None;
        let mut creation_date = None;
        let mut info = None;
        let mut info_hash = None;
        let mut comment = None;
        let mut encoding = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"announce", value) => {
                    announce = String::decode_bencode_object(value)
                        .context("announce")
                        .map(Some)?;
                }
                (b"announce-list", value) => {
                    announce_list = Vec::<Vec<String>>::decode_bencode_object(value)
                        .context("announce-list")
                        .map(Some)?;
                }
                (b"created by", value) => {
                    created_by = String::decode_bencode_object(value)
                        .context("created by")
                        .map(Some)?;
                }
                (b"creation date", value) => {
                    creation_date = u64::decode_bencode_object(value)
                        .context("creation date")
                        .map(Some)?;
                }
                (b"info", Object::Dict(dict)) => {
                    let info_bytes = dict.into_raw()?;
                    info_hash = Some(calculate_info_hash(info_bytes));
                    info = Info::from_bencode(info_bytes).context("info").map(Some)?;
                }
                (b"comment", value) => {
                    comment = String::decode_bencode_object(value)
                        .context("comment")
                        .map(Some)?;
                }
                (b"encoding", value) => {
                    encoding = String::decode_bencode_object(value)
                        .context("encoding")
                        .map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let announce = announce.ok_or_else(|| Error::missing_field("announce"))?;
        let announce_list = announce_list.ok_or_else(|| Error::missing_field("announce_list"))?;
        let creation_date = creation_date.ok_or_else(|| Error::missing_field("creation_date"))?;
        let info = info.ok_or_else(|| Error::missing_field("info"))?;
        let info_hash = info_hash.ok_or_else(|| Error::missing_field("info_hash"))?;

        Ok(Self {
            announce,
            announce_list,
            created_by,
            creation_date,
            info,
            info_hash,
            comment,
            encoding,
        })
    }
}

/// 种子信息结构体
#[derive(Debug, Hash, Eq, PartialEq, Encode, Decode, Default)]
pub struct Info {
    pub length: u64,             // 文件大小
    pub piece_length: u64,       // 分片大小
    pub pieces: Vec<u8>,         // 每20个字节一块的校验码
    pub name: String,            // 文件名
    pub files: Vec<File>,        // 文件列表
    pub md5sum: Option<String>,  // 文件md5值
    pub private: Option<u8>,     // 是否私有
    file_piece: Vec<(u32, u32)>, // 多文件情况下，分片下标对应的文件
}

impl Info {
    fn new(
        name: String, length: u64, piece_length: u64, pieces: Vec<u8>, mut files: Vec<File>,
        md5sum: Option<String>, private: Option<u8>,
    ) -> Self {
        let file_piece = Self::calculate_file_piece(&mut files, piece_length);
        Self {
            name,
            length,
            piece_length,
            pieces,
            files,
            md5sum,
            private,
            file_piece,
        }
    }

    fn calculate_file_piece(files: &mut Vec<File>, piece_length: u64) -> Vec<(u32, u32)> {
        let mut file_piece = Vec::with_capacity(files.len());
        let mut sum = 0;
        let mut prev = 0;
        for file in files {
            let start = (prev + (sum % piece_length == 0) as i64) as u64;
            sum += file.length;
            let end = sum.div_ceil(piece_length);
            file_piece.push((start as u32 - 1, end as u32 - 1));
            prev = end as i64;
            file.length_prefix_sum = sum;
        }
        file_piece
    }

    fn find_file_of_piece_index(
        &self, path_buf: &Path, piece_index: u32, offset: u32, len: usize,
    ) -> Vec<BlockInfo> {
        if self.file_piece.is_empty() {
            return vec![BlockInfo {
                filepath: path_buf.join(&self.name),
                start: piece_index as u64 * self.piece_length + offset as u64,
                len,
                file_len: self.length,
            }];
        }

        let m_low = self.file_piece.partition_point(|&(_, r)| r < piece_index);
        let n_high = self.file_piece.partition_point(|&(l, _)| l <= piece_index);

        let mut res = vec![];
        let begin = piece_index as u64 * self.piece_length + offset as u64;
        let end = begin + len as u64;
        let mut left = len as u64;

        for i in m_low..n_high {
            let file = &self.files[i];
            let lps = file.length_prefix_sum;
            if lps >= begin && lps - file.length < end {
                let start = if_else!(begin <= lps - file.length, 0, begin - (lps - file.length));
                let len =
                    if_else!(end <= lps, end - (lps - file.length), file.length - start).min(left);

                let file = &self.files[i];
                let filepath = path_buf
                    .join(&self.name)
                    .join(file.path.iter().collect::<PathBuf>());

                res.push(BlockInfo {
                    filepath,
                    start,
                    len: len as usize,
                    file_len: file.length,
                });

                left -= len;
            }
        }

        res
    }
}

impl FromBencode for Info {
    fn decode_bencode_object(object: Object) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let mut name = None;
        let mut length = None;
        let mut piece_length = None;
        let mut pieces = None;
        let mut files = None;
        let mut md5sum = None;
        let mut private = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"name", value) => {
                    name = String::decode_bencode_object(value)
                        .context("name")
                        .map(Some)?;
                }
                (b"length", value) => {
                    length = u64::decode_bencode_object(value)
                        .context("length")
                        .map(Some)?;
                }
                (b"piece length", value) => {
                    piece_length = u64::decode_bencode_object(value)
                        .context("piece length")
                        .map(Some)?;
                }
                (b"pieces", value) => {
                    pieces = Some(AsString::decode_bencode_object(value)?.0);
                }
                (b"files", value) => {
                    files = Vec::<File>::decode_bencode_object(value)
                        .context("files")
                        .map(Some)?;
                }
                (b"md5sum", value) => {
                    md5sum = String::decode_bencode_object(value)
                        .context("md5sum")
                        .map(Some)?;
                }
                (b"private", value) => {
                    private = u8::decode_bencode_object(value)
                        .context("private")
                        .map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let name = name.ok_or_else(|| Error::missing_field("name"))?;
        let piece_length = piece_length.ok_or_else(|| Error::missing_field("piece length"))?;
        let pieces = pieces.ok_or_else(|| Error::missing_field("pieces"))?;
        let length = if let Some(files) = files.as_ref() {
            files.iter().map(|file| file.length).sum()
        } else {
            length.ok_or_else(|| Error::missing_field("length"))?
        };

        Ok(Info::new(
            name,
            length,
            piece_length,
            pieces,
            files.unwrap_or_default(),
            md5sum,
            private,
        ))
    }
}

/// 文件结构体，适用于多文件种子
#[derive(Debug, Hash, Eq, PartialEq, Encode, Decode)]
pub struct File {
    pub length: u64,        // 文件大小
    pub path: Vec<String>,  // 文件路径
    length_prefix_sum: u64, // 总大小的前缀和
    md5sum: Option<String>, // 文件md5值
}

impl File {
    fn new(length: u64, path: Vec<String>, md5sum: Option<String>) -> Self {
        Self {
            length,
            path,
            length_prefix_sum: 0,
            md5sum,
        }
    }
}

impl FromBencode for File {
    fn decode_bencode_object(object: Object) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let mut length = None;
        let mut path = None;
        let mut md5sum = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"length", value) => {
                    length = u64::decode_bencode_object(value)
                        .context("length")
                        .map(Some)?;
                }
                (b"path", value) => {
                    path = Vec::<String>::decode_bencode_object(value)
                        .context("path")
                        .map(Some)?;
                }
                (b"md5sum", value) => {
                    md5sum = String::decode_bencode_object(value)
                        .context("md5sum")
                        .map(Some)?;
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let length = length.ok_or_else(|| Error::missing_field("length"))?;
        let path = path.ok_or_else(|| Error::missing_field("path"))?;

        Ok(File::new(length, path, md5sum))
    }
}

#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub filepath: PathBuf,
    pub start: u64,
    pub len: usize,
    pub file_len: u64,
}

fn calculate_info_hash(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data);
    let mut result = [0; 20];
    result.copy_from_slice(&hasher.finalize());
    result
}

pub trait Parse<T, U = Self> {
    /// 解析种子文件
    fn parse_torrent(data: T) -> Result<U>;
}

/// 直接传入字节数组
impl Parse<Vec<u8>> for Torrent {
    fn parse_torrent(data: Vec<u8>) -> Result<Torrent> {
        match Torrent::from_bencode(&data) {
            Ok(torrent) => Ok(torrent),
            Err(e) => Err(anyhow!("解析种子文件失败: {}", e)),
        }
    }
}

impl Parse<Bytes> for Torrent {
    fn parse_torrent(data: Bytes) -> Result<Self> {
        match Torrent::from_bencode(&data) {
            Ok(torrent) => Ok(torrent),
            Err(e) => Err(anyhow!("解析种子文件失败: {}", e)),
        }
    }
}

/// 传入文件路径
impl Parse<&str> for Torrent {
    fn parse_torrent(data: &str) -> Result<Torrent> {
        let data = fs::read(data)?;
        Torrent::parse_torrent(data)
    }
}

/// 传入字节引用
impl Parse<&[u8]> for Torrent {
    fn parse_torrent(data: &[u8]) -> Result<Torrent> {
        match Torrent::from_bencode(data) {
            Ok(torrent) => Ok(torrent),
            Err(e) => Err(anyhow!("解析种子文件失败: {}", e)),
        }
    }
}

/// 传入文件路径
impl Parse<&str> for TorrentArc {
    fn parse_torrent(data: &str) -> Result<TorrentArc> {
        Ok(TorrentArc {
            inner: Arc::new(Torrent::parse_torrent(data)?),
        })
    }
}

/// 传入字节数组
impl Parse<Vec<u8>> for TorrentArc {
    fn parse_torrent(data: Vec<u8>) -> Result<TorrentArc> {
        Ok(TorrentArc {
            inner: Arc::new(Torrent::parse_torrent(data)?),
        })
    }
}

impl Parse<Bytes> for TorrentArc {
    fn parse_torrent(data: Bytes) -> Result<Self> {
        Ok(TorrentArc {
            inner: Arc::new(Torrent::parse_torrent(&*data)?),
        })
    }
}

impl Parse<&[u8]> for TorrentArc {
    fn parse_torrent(data: &[u8]) -> Result<Self> {
        Ok(TorrentArc {
            inner: Arc::new(Torrent::parse_torrent(data)?),
        })
    }
}
