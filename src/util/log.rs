#[cfg(test)]
mod tests;

use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use time::format_description;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// 文件拆分数量 chunks，虽然最低要求为 1。但不建议使用 1，否则容量刚好达到上限时，日志会被清空掉。
struct SizeBasedWriter {
    directory: PathBuf,
    file_prefix: String,
    max_size: u64,
    chunks: usize,
    log_file: Option<File>,
    file_size: u64,
}

impl SizeBasedWriter {
    fn new(
        directory: &Path,
        file_prefix: &str,
        max_size: u64,
        chunks: usize,
    ) -> std::io::Result<Self> {
        assert!(chunks > 0, "file chunks must be greater than 0");

        let (log_file, file_size) = Self::get_last_log(directory)?;

        let directory = directory.to_path_buf();

        Ok(Self {
            directory,
            file_prefix: file_prefix.to_string(),
            max_size,
            chunks,
            log_file,
            file_size,
        })
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        // 关闭当前文件
        if let Some(file) = self.log_file.take() {
            file.sync_all()?;
        }

        // 生成带时间戳的新文件名
        let format =
            format_description::parse("[year][month][day]_[hour][minute][second]").unwrap();
        let timestamp = time::OffsetDateTime::now_local().unwrap().format(&format).unwrap();
        let new_path = self
            .directory
            .join(format!("{}_{}.log", self.file_prefix, timestamp));

        // 创建新文件
        let file = File::create(new_path)?;
        self.log_file = Some(file);
        self.file_size = 0;

        // 清理旧文件（总大小超过阈值）
        self.cleanup_old_files()?;

        Ok(())
    }

    /// 获取最后的一个文件
    fn get_last_log(dir: &Path) -> std::io::Result<(Option<File>, u64)> {
        let entries: Vec<fs::DirEntry> =
            fs::read_dir(dir)?.filter_map(|entry| entry.ok()).collect();

        let file = entries.iter().max_by(|&a, &b| {
            a.metadata()
                .unwrap()
                .modified()
                .unwrap()
                .cmp(&b.metadata().unwrap().modified().unwrap())
        });

        match file {
            Some(file) => Ok((
                Some(OpenOptions::new().append(true).open(file.path())?),
                file.metadata()?.len(),
            )),
            None => Ok((None, 0)),
        }
    }

    fn cleanup_old_files(&self) -> std::io::Result<()> {
        let mut entries: Vec<fs::DirEntry> = fs::read_dir(&self.directory)?
            .filter_map(|entry| entry.ok())
            .collect();

        if entries.len() <= self.chunks {
            return Ok(());
        }

        // 按修改时间排序（旧文件在前）
        entries.sort_by(|a, b| {
            a.metadata()
                .unwrap()
                .modified()
                .unwrap()
                .cmp(&b.metadata().unwrap().modified().unwrap())
        });

        // 计算总大小并删除最旧文件
        let take = entries.len() - self.chunks;
        for entry in entries.iter().take(take) {
            // 这里不用判断大小，因为有可能最后一条日志记录，超过了当前日志的大小限制
            // 但是当前日志又没被填满。所以这里直接删除就好。
            fs::remove_file(entry.path())?;
        }

        Ok(())
    }
}

impl Write for SizeBasedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // 满了或者文件不存在，则创建新文件
        if self.log_file.is_none() || self.file_size + buf.len() as u64 > self.max_size {
            self.rotate()?;
        }

        // 写入日志
        if let Some(file) = &mut self.log_file {
            let written = file.write(buf)?;
            self.file_size += written as u64;
            Ok(written)
        } else {
            Ok(0)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = self.log_file.as_mut() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

/// 注册日志记录器
///
/// # Arguments
///
/// * `dir` - 日志目录
/// * `file_prefix` - 日志文件前缀
/// * `max_size` - 一个文件最大大小（单位：字节）
/// * `chunks` - 保留的文件数量
/// * `level` - 日志输出等级
///
/// # Returns
///
/// 返回一个 `WorkerGuard`，避免日志线程退出导致日志丢失。
pub fn register_logger(
    dir: &str,
    file_prefix: &str,
    max_size: u64,
    chunks: usize,
    level: Level,
) -> std::io::Result<WorkerGuard> {
    // 创建日志目录
    let log_dir = Path::new(dir);
    fs::create_dir_all(log_dir)?;

    // 初始化自定义写入器（10MB 上限）
    let writer = SizeBasedWriter::new(log_dir, file_prefix, max_size, chunks)?;
    let (non_blocking, guard) = tracing_appender::non_blocking(writer);

    // 格式化日期
    let time_fmt = time::format_description::parse(
        "[year]-[month padding:zero]-[day padding:zero] [hour]:[minute]:[second].[subsecond digits:4]",
    ).unwrap();
    let timer = fmt::time::LocalTime::new(time_fmt);

    // 输出到文件
    let out_file = fmt::layer()
        .with_timer(timer.clone())
        .with_line_number(true)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_writer(non_blocking.with_max_level(level));

    // 输出到控制台
    let console = fmt::layer()
        .with_timer(timer)
        .with_line_number(true)
        .with_thread_names(false)
        .with_thread_ids(true)
        .with_writer(std::io::stderr.with_max_level(level));

    // 注册订阅
    tracing_subscriber::registry()
        .with(out_file)
        .with(console)
        .init();

    Ok(guard)
}
