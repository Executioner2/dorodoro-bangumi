//! 资源存储服务

pub mod error;
#[cfg(test)]
mod tests;

use crate::buffer::ByteBuffer;
use crate::config::Config;
use crate::emitter::Emitter;
use crate::fs::OpenOptionsExt;
use bytes::Bytes;
use dashmap::DashMap;
use error::Result;
use sha1::{Digest, Sha1};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering as AtomicOrdering;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio::sync::Semaphore;
use crate::torrent::BlockInfo;

struct Block {
    offset: u64,
    data: Bytes,
}

impl PartialEq for Block {
    fn eq(&self, other: &Self) -> bool {
        self.offset == other.offset
    }
}

impl Eq for Block {}

impl PartialOrd for Block {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.offset.cmp(&other.offset))
    }
}

impl Ord for Block {
    fn cmp(&self, other: &Self) -> Ordering {
        self.offset.cmp(&other.offset)
    }
}

struct FileWriter {
    /// 文件路径
    path: PathBuf,

    /// 写入缓冲区
    blocks: BTreeSet<Block>,

    /// 总缓冲大小
    buf_size: usize,

    /// 缓存大小上限
    buf_limit: usize,

    /// 文件大小，用于预分配
    file_len: u64,

    /// 写入数据量统计
    write_len: u64,
}

impl FileWriter {
    pub fn new(path: PathBuf, buf_limit: usize, file_len: u64) -> Self {
        Self {
            path,
            blocks: BTreeSet::default(),
            buf_size: 0,
            buf_limit,
            file_len,
            write_len: 0,
        }
    }

    async fn get_file(&self) -> Result<File> {
        let mut file = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .open_with_parent_dirs(&self.path)
            .await?;
        if file.metadata().await?.len() == 0 {
            file.set_len(self.file_len).await?;
        }
        file.set_max_buf_size(self.buf_limit);
        Ok(file)
    }

    /// 返回 false 表示还没有完结，否则反之
    async fn flush(&mut self) -> Result<(bool, usize)> {
        if self.blocks.is_empty() {
            return Ok((false, 0));
        }

        // 合并连续块，并刷进磁盘
        let mut buf = BufWriter::with_capacity(self.buf_size, self.get_file().await?);
        let mut prev_offset = 0;
        while let Some(block) = self.blocks.pop_first() {
            if prev_offset != block.offset {
                buf.seek(SeekFrom::Start(block.offset)).await?;
            }
            buf.write_all(&block.data).await?;
            prev_offset = block.offset + block.data.len() as u64;
        }

        buf.flush().await?;
        let wrtie_len = self.buf_size;
        self.write_len += wrtie_len as u64;
        self.buf_size = 0;

        Ok((self.write_len >= self.file_len, wrtie_len))
    }

    fn write(&mut self, offset: u64, data: Bytes) {
        self.buf_size += data.len();
        let block = Block { offset, data };
        self.blocks.insert(block);
    }
}

#[derive(Clone)]
pub struct Store {
    /// 配置
    config: Config,

    /// 命令发射器
    _emitter: Emitter,

    /// 文件写入 handle
    file_writer: Arc<DashMap<PathBuf, FileWriter>>,

    /// 总的缓存量
    buf_size: Arc<AtomicUsize>,

    /// hash 计算并发控制
    hash_semaphore: Arc<Semaphore>,
}

impl Store {
    pub fn new(config: Config, emitter: Emitter) -> Self {
        let permits = config.hash_concurrency();
        Self {
            config,
            _emitter: emitter,
            file_writer: Arc::new(DashMap::new()),
            buf_size: Arc::new(AtomicUsize::new(0)),
            hash_semaphore: Arc::new(Semaphore::new(permits)),
        }
    }

    pub async fn flush(&self, path: &PathBuf) -> Result<()> {
        let mut remove = false;
        if let Some(mut writer) = self.file_writer.get_mut(path) {
            let (r, n) = writer.flush().await?;
            self.buf_size.fetch_sub(n, AtomicOrdering::Relaxed);
            remove = r;
        }
        if remove {
            self.file_writer.remove(path);
        }
        Ok(())
    }

    pub async fn flush_all(&self) -> Result<()> {
        let mut remove_keys = vec![];
        for mut item in self.file_writer.iter_mut() {
            if item.flush().await?.0 {
                remove_keys.push(item.path.clone());
            }
        }
        for item in remove_keys {
            self.file_writer.remove(&item);
        }
        self.buf_size.store(0, AtomicOrdering::Relaxed);
        Ok(())
    }

    pub async fn write(&self, block_info: BlockInfo, data: Bytes) -> Result<()> {
        self.buf_size
            .fetch_add(block_info.len, AtomicOrdering::Release);
        self.file_writer
            .entry(block_info.filepath.clone())
            .or_insert(FileWriter::new(
                block_info.filepath,
                self.config.buf_limit(),
                block_info.file_len,
            ))
            .write(block_info.start, data);

        if self.buf_size.load(AtomicOrdering::Acquire) >= self.config.buf_limit() {
            self.flush_all().await
        } else {
            Ok(())
        }
    }

    /// 流式校验分块 hash
    pub async fn checkout(&self, src: Vec<BlockInfo>, hash: &[u8]) -> Result<bool> {
        let permit = self.hash_semaphore.clone().acquire_owned().await?;
        let mut hasher = Sha1::new();
        let mut chunk = ByteBuffer::new(self.config.hash_chunk_size());

        for mut block_info in src {
            self.flush(&block_info.filepath).await?;
            let mut file = OpenOptions::new()
                .read(true)
                .open_with_parent_dirs(block_info.filepath)
                .await?;
            file.seek(SeekFrom::Start(block_info.start)).await?;
            while block_info.len > 0 {
                let end = chunk.capacity().min(block_info.len);
                let n = file.read(&mut chunk[0..end]).await?;
                hasher.update(&chunk[0..n]);
                block_info.len -= n;
            }
        }

        drop(permit);
        Ok(hasher.finalize().as_slice() == hash)
    }
}
