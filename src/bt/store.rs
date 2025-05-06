//! 资源存储服务

pub mod error;
#[cfg(test)]
mod tests;

use crate::buffer::ByteBuffer;
use crate::config::Config;
use crate::emitter::Emitter;
use crate::fs::OpenOptionsExt;
use crate::torrent::BlockInfo;
use bytes::Bytes;
use dashmap::DashMap;
use error::Result;
use memmap2::MmapMut;
use sha1::{Digest, Sha1};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering as AtomicOrdering;
use tokio::sync::Semaphore;
use crate::store::error::Error::FileLengthError;

struct FileWriter {
    /// 文件路径
    path: PathBuf,

    /// 写入缓冲区
    mmap: Option<MmapMut>,

    /// 总缓冲大小
    buf_size: usize,

    /// 文件大小，用于预分配
    file_len: u64,

    /// 写入数据量统计
    write_len: u64,
}

impl FileWriter {
    pub fn new(path: PathBuf, file_len: u64) -> Self {
        Self {
            path,
            mmap: None,
            buf_size: 0,
            file_len,
            write_len: 0,
        }
    }

    fn get_file(&self) -> Result<File> {
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .open_with_parent_dirs(&self.path)?;
        if file.metadata()?.len() < self.file_len {
            file.set_len(self.file_len)?;
        }
        Ok(file)
    }

    /// 返回 false 表示还没有完结，否则反之
    fn flush(&mut self) -> Result<(bool, usize)> {
        if self.mmap.is_none() {
            return Ok((false, 0));
        }

        let mmap = self.mmap.as_ref().unwrap();
        mmap.flush()?;

        let wrtie_len = self.buf_size;
        self.write_len += wrtie_len as u64;
        self.buf_size = 0;

        Ok((self.write_len >= self.file_len, wrtie_len))
    }

    fn write(&mut self, offset: u64, data: Bytes) -> Result<()> {
        self.buf_size += data.len();
        if self.mmap.is_none() {
            self.mmap = Some(unsafe { MmapMut::map_mut(&self.get_file()?)? })
        }
        let end = (offset + data.len() as u64) as usize;
        if end as u64 > self.file_len {
            return Err(FileLengthError(self.file_len, end as u64));
        }
        self.mmap.as_mut().map(|mmap| {
            mmap[offset as usize..end].copy_from_slice(&data);
        });
        let mmap = match self.mmap.take() {
            None => unsafe { MmapMut::map_mut(&self.get_file()?)? },
            Some(mmap) => mmap,
        };
        self.mmap = Some(mmap);
        Ok(())
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

    pub fn flush(&self, path: &PathBuf) -> Result<()> {
        let mut remove = false;
        if let Some(mut writer) = self.file_writer.get_mut(path) {
            let (r, n) = writer.flush()?;
            self.buf_size.fetch_sub(n, AtomicOrdering::Relaxed);
            remove = r;
        }
        if remove {
            self.file_writer.remove(path);
        }
        Ok(())
    }

    pub fn flush_all(&self) -> Result<()> {
        let mut remove_keys = vec![];
        for mut item in self.file_writer.iter_mut() {
            if item.flush()?.0 {
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
            .or_insert(FileWriter::new(block_info.filepath, block_info.file_len))
            .write(block_info.start, data)?;

        if self.buf_size.load(AtomicOrdering::Acquire) >= self.config.buf_limit() {
            self.flush_all()
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
            self.flush(&block_info.filepath)?;
            let mut file = OpenOptions::new()
                .read(true)
                .open_with_parent_dirs(block_info.filepath)?;
            file.seek(SeekFrom::Start(block_info.start))?;
            while block_info.len > 0 {
                let end = chunk.capacity().min(block_info.len);
                let n = file.read(&mut chunk[0..end])?;
                hasher.update(&chunk[0..n]);
                block_info.len -= n;
            }
        }

        drop(permit);
        Ok(hasher.finalize().as_slice() == hash)
    }
}
