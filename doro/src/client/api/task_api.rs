use crate::{register_route, task_service};
use doro_macro::route;
use serde::{Deserialize, Serialize};
use anyhow::Result;
use crate::router::ret::Ret;

/// 种子来源
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Hash)]
pub enum TorrentSource {
    /// 本地文件
    LocalFile(String),

    /// 磁力链接
    MagnetURI,

    /// RSS 订阅
    RSSFeed(u64, String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Task {
    /// 任务名
    pub task_name: Option<String>,

    /// 下载路径
    pub download_path: Option<String>,

    /// 种子源
    pub source: TorrentSource,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TorrentRet {
    /// 种子文件名
    pub name: String,

    /// 文件大小
    pub length: u64,

    /// 哈希值
    pub info_hash: Option<String>,

    /// 注释
    pub comment: Option<String>,

    /// 资源文件
    pub files: Vec<File>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct File {
    /// 文件路径
    pub path: Vec<String>,

    /// 文件大小
    pub length: u64,
}

// ===========================================================================
// API 接口
// ===========================================================================

/// 解析 torrent 链接，支持磁力链接和文件哈希值
#[route(code = 1001)]
pub async fn parse_torrent_link(_link: String) -> Result<Ret<TorrentRet>> {
    todo!("parse_torrent_link")
}

/// 解析本地种子文件
#[route(code = 1002)]
pub async fn parse_torrent_file(file_path: String) -> Result<Ret<TorrentRet>> {
    let r = task_service::parse_torrent_file(file_path.as_str())?;
    Ok(Ret::ok(r))
}

/// 添加下载任务
#[route(code = 1003)]
pub async fn add_task(task: Task) -> Result<Ret<bool>> {
    let r = task_service::add_task(task).await?;
    Ok(Ret::ok(r))
}
