//! 资源存储服务

pub mod error;
#[cfg(test)]
mod tests;

use crate::fs::OpenOptionsExt;
use ahash::HashMap;
use bytes::Bytes;
use error::Result;
use lazy_static::lazy_static;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::SeekFrom;
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

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
}

impl FileWriter {
    pub fn new(path: PathBuf, buf_limit: usize, file_len: u64) -> Self {
        Self {
            path,
            blocks: BTreeSet::default(),
            buf_size: 0,
            buf_limit,
            file_len,
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

    async fn flush(&mut self) -> Result<()> {
        if self.blocks.is_empty() {
            return Ok(());
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
        Ok(())
    }

    fn write(&mut self, offset: u64, data: Bytes) {
        self.buf_size += data.len();
        let block = Block { offset, data };
        self.blocks.insert(block);
    }
}

lazy_static! {
    static ref file_writer: Mutex<HashMap<PathBuf, FileWriter>> = Mutex::new(HashMap::default());
}

pub async fn flush(path: &PathBuf) -> Result<()> {
    if let Some(writer) = file_writer.lock().await.get_mut(path) {
        writer.flush().await
    } else {
        Ok(())
    }
}

pub async fn write(path: PathBuf, offset: u64, data: Bytes, file_size: u64) {
    file_writer
        .lock()
        .await
        .entry(path.clone())
        .or_insert(FileWriter::new(path, 16 * 1024 * 1024, file_size))
        .write(offset, data)
}
