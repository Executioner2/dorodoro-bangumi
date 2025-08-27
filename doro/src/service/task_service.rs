use std::path::PathBuf;

use anyhow::{Result, anyhow};
use doro_util::global::GlobalId;
use tokio::sync::mpsc::channel;

use crate::api::task_api::{File, Task, TorrentRet, TorrentSource};
use crate::config::CHANNEL_BUFFER;
use crate::context::Context;
use crate::mapper::rss::RSSMapper;
use crate::mapper::torrent::TorrentMapper;
use crate::rss::HTTP_REQUEST_TIMEOUT;
use crate::task::content::DownloadContent;
use crate::task::magnet::{self, Magnet, ParseMagnet};
use crate::task::Task as TaskTrait;
use crate::task_manager::TaskManager;
use crate::torrent::{Parse, Torrent, TorrentArc};

// ===========================================================================
// 内部函数
// ===========================================================================

/// 从本地文件中加载种子
fn load_torrent_from_local_file(file_path: &str) -> Result<TorrentArc> {
    TorrentArc::parse_torrent(file_path)
}

/// 从磁力链接中加载种子
async fn load_torrent_from_magnet_uri(magnet_uri: &str) -> Result<TorrentArc> {
    let magent = Magnet::from_url(magnet_uri)?;
    let (tx, mut rx) = channel(CHANNEL_BUFFER);

    let id = GlobalId::next_id();
    let peer_id = TaskManager::global().get_peer_id();
    let task = ParseMagnet::new(id, peer_id, magent);
    task.subscribe_inside_info(tx);
    TaskManager::global().handle_task(Box::new(task)).await?;
    
    while let Some(ret) = rx.recv().await {
        if let Ok(ret) = ret.downcast::<magnet::event::Event>() {
            #[allow(irrefutable_let_patterns)] // Event 枚举中只有一个成员，这里不要警告
            if let magnet::event::Event::ParseSuccess(torrent) = *ret {
                TaskManager::global().shutdown_task(id).await;
                return Ok(torrent);
            }
        }
    }

    Err(anyhow!("解析种子失败，解析任务提前退出，但并未返回成功后的结果"))
}

/// 从 rss 订阅中加载种子
async fn load_torrent_from_rss_feed(rss_id: u64, guid: &str, url: &str) -> Result<TorrentArc> {
    let content = tokio::time::timeout(HTTP_REQUEST_TIMEOUT, reqwest::get(url))
        .await??
        .bytes()
        .await?;
    let res = TorrentArc::parse_torrent(content);
    if res.is_ok() {
        let mut conn = Context::get_conn().await?;
        conn.mark_read(rss_id, guid)?;
    }
    res
}

// ===========================================================================
// 对外开放的函数
// ===========================================================================

/// 解析种子链接
pub async fn parse_torrent_link(link: &str) -> Result<TorrentRet> {
    load_torrent_from_magnet_uri(link).await
        .map(|torrent| TorrentRet {
            name: torrent.info.name.clone(),
            length: torrent.info.length,
            info_hash: Some(hex::encode(torrent.info_hash)),
            comment: torrent.comment.clone(),
            files: torrent.info.files.iter().map(|file| File {
                path: file.path.clone(),
                length: file.length,
            }).collect()
        })
}

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

/// 添加任务
pub async fn add_task(task: Task) -> Result<bool> {
    // 添加到数据库
    let torrent = {
        match task.source {
            TorrentSource::LocalFile(file_path) => {
                load_torrent_from_local_file(file_path.as_str())?
            }
            TorrentSource::MagnetURI(info_hash) => {
                load_torrent_from_magnet_uri(info_hash.as_str()).await?
            }
            TorrentSource::RSSFeed(rss_id, guid, url) => {
                load_torrent_from_rss_feed(rss_id, guid.as_str(), url.as_str()).await?
            }
        }
    };

    let mut save_path = task
        .download_path
        .map(PathBuf::from)
        .unwrap_or(Context::get_config().default_download_dir());
    if task.mkdir_torrent_name == Some(true) {
        save_path.push(torrent.info.name.clone());
    }

    let ret = {
        let mut conn = Context::get_conn().await?;
        conn.add_torrent(&torrent, &save_path)?
    };

    if ret {
        // 保存数据库成功，添加到下载队列
        let id = GlobalId::next_id();
        let peer_id = TaskManager::global().get_peer_id();
        let task = DownloadContent::new(id, peer_id, torrent, save_path).await?;
        TaskManager::global().handle_task(Box::new(task)).await?;
    }

    Ok(ret)
}
