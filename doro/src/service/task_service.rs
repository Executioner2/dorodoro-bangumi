use std::path::PathBuf;
use crate::api::task_api::{File, Task, TorrentRet, TorrentSource};
use crate::torrent::{Parse, Torrent, TorrentArc};
use anyhow::Result;
use crate::context::Context;
use crate::mapper::rss::RSSMapper;
use crate::mapper::torrent::TorrentMapper;
use crate::rss::HTTP_REQUEST_TIMEOUT;
use crate::task_handler::TaskHandler;

/// 解析种子文件
pub fn parse_torrent_file(file_path: &str) -> Result<TorrentRet> {
    let torrent = Torrent::parse_torrent(file_path)?;
    let info = torrent.info;
    let name = info.name;
    let length = info.length;
    let info_hash = info.md5sum;
    let comment = torrent.comment;

    #[rustfmt::skip]
    let files = info.files.into_iter().map(|file| {
        File { path: file.path, length: file.length }
    }).collect();

    Ok(TorrentRet {
        name,
        length,
        info_hash,
        comment,
        files,
    })
}

/// 从本地文件中加载种子
pub fn load_torrent_from_local_file(file_path: &str) -> Result<TorrentArc> {
    TorrentArc::parse_torrent(file_path)
}

/// 从磁力链接中加载种子
pub fn load_torrent_from_magnet_uri(magnet_uri: &str) -> Result<TorrentArc> {
    todo!("待实现: {}", magnet_uri)
}

/// 从 rss 订阅中加载种子
pub async fn load_torrent_from_rss_feed(rss_id: u64, guid: &str, url: &str) -> Result<TorrentArc> {
    let content = tokio::time::timeout(HTTP_REQUEST_TIMEOUT, reqwest::get(url))
        .await??
        .bytes()
        .await?;
    let res = TorrentArc::parse_torrent(content);
    if res.is_ok() {
        let mut conn = Context::global().get_conn().await?;
        conn.mark_read(rss_id, &guid)?;
    }
    res
}

/// 添加任务
pub async fn add_task(task: Task) -> Result<bool> {
    // 添加到数据库
    let torrent = {
        match task.source {
            TorrentSource::LocalFile(file_path) => {
                load_torrent_from_local_file(file_path.as_str())?
            }
            TorrentSource::MagnetURI(info_hash) => {
                load_torrent_from_magnet_uri(info_hash.as_str())?
            }
            TorrentSource::RSSFeed(rss_id, guid, url) => {
                load_torrent_from_rss_feed(rss_id, guid.as_str(), url.as_str()).await?
            }
        }
    };

    let ret = {
        let mut conn = Context::global().get_conn().await?;
        let save_path = task.download_path
            .map(|path| PathBuf::from(path))
            .unwrap_or(Context::global().get_config().default_download_dir().clone());
        conn.add_torrent(&torrent, &save_path)?
    };

    if ret {
        // 保存数据库成功，添加到下载队列
        TaskHandler::global().handle_task(torrent).await;
    }

    Ok(ret)
}
