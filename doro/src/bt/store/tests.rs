use dashmap::DashMap;
use doro_util::buffer::ByteBuffer;
use doro_util::fs::{AsyncOpenOptionsExt, OpenOptionsExt};
use memmap2::MmapMut;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use tokio::fs::{File, OpenOptions};
use tokio::io::{
    AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
    SeekFrom,
};
use tokio::time::Duration;
use tracing::info;

// 包装 File 并记录写入次数
struct InstrumentedFile {
    inner: File,
    write_count: Arc<AtomicUsize>,
}

impl InstrumentedFile {
    fn new(file: File) -> Self {
        Self {
            inner: file,
            write_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn _write_count(&self) -> usize {
        self.write_count.load(Ordering::SeqCst)
    }
}

impl AsyncWrite for InstrumentedFile {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let pinned_inner = Pin::new(&mut self.inner);
        let result = pinned_inner.poll_write(cx, buf);
        if let Poll::Ready(Ok(_)) = &result {
            self.write_count.fetch_add(1, Ordering::SeqCst);
        }
        result
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl AsyncSeek for InstrumentedFile {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        Pin::new(&mut self.inner).start_seek(position)
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        Pin::new(&mut self.inner).poll_complete(cx)
    }
}

#[tokio::test]
async fn test_buf_writer() {
    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open("../../../../tests/resources/test_buf_writer.txt")
        .await
        .unwrap();

    let instrumented_file = InstrumentedFile::new(file);
    let write_count = instrumented_file.write_count.clone();

    let mut buf = BufWriter::new(instrumented_file);

    tokio::time::sleep(Duration::from_secs(5)).await;

    buf.seek(SeekFrom::Start(10)).await.unwrap();
    info!("5秒到了，写入 hello world");
    buf.write_all(b"hello world").await.unwrap();

    tokio::time::sleep(Duration::from_secs(5)).await;
    info!("5秒到了，设置起始位置");
    // buf.seek(SeekFrom::Start(4)).await.unwrap();
    buf.seek(SeekFrom::Start(21)).await.unwrap();

    tokio::time::sleep(Duration::from_secs(5)).await;
    info!("5秒到了，写入 ni hao");
    buf.write_all(b"ni hao").await.unwrap();

    tokio::time::sleep(Duration::from_secs(5)).await;
    info!("刷新 ni hao 到磁盘");
    buf.flush().await.unwrap();

    // 获取最终写入次数
    let count = write_count.load(Ordering::SeqCst);
    info!("实际磁盘写入次数: {}", count);
}

/// 测试在刷新数据到磁盘之前读取
///
/// 结论：不 flush 到磁盘，是读不到的
#[tokio::test]
async fn test_flush_before_read() {
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open_with_parent_dirs("../../../../tests/resources/test_flush_before_read.txt")
        .await
        .unwrap();

    let mut writer = BufWriter::new(file);
    let _ = writer.write(b"hello world").await.unwrap();
    // writer.flush().await.unwrap();

    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open_with_parent_dirs("../../../../tests/resources/test_flush_before_read.txt")
        .await
        .unwrap();
    let mut reader = BufReader::new(file);
    let mut buff = ByteBuffer::new(11);
    let n = reader.read(buff.as_mut()).await.unwrap();
    info!("数据量: {}\t数据: {:?}", n, buff.as_ref())
}

/// 测试迭代过程删除
#[test]
#[ignore]
fn test_iter_remove() {
    let map = DashMap::new();
    map.insert(1, 1);
    map.insert(2, 2);
    map.insert(3, 3);

    for item in map.iter_mut() {
        info!("key: {}\tvalue: {}", item.key(), item.value());
        // map.remove(&item.key()); // 不能在里面删除，会卡死
    }

    // 这个同理
    // if let Some(item) = map.get_mut(&1) {
    //     info!("key: {}\tvalue: {}", item.key(), item.value());
    //     map.remove(&1);
    // }

    info!("{:?}", map);
}

/// 尝试使用 mmap 落盘
#[test]
fn test_mmap() {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open_with_parent_dirs("../../../../tests/resources/test_mmap.txt")
        .unwrap();
    file.set_len(10).unwrap();

    let mut mmap = unsafe { MmapMut::map_mut(&file) }.unwrap();
    mmap[0..5].copy_from_slice(b"hello");
    // thread::sleep(Duration::from_secs(5));
    mmap[5..10].copy_from_slice(b"world");
    mmap.flush().unwrap()
}
