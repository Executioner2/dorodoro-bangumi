use serde::{Deserialize, Serialize};
use crate::register_route;
use doro_macro::route;
use crate::client::common::Ret;
use crate::torrent::{Parse, Torrent};

#[derive(Serialize, Deserialize, Debug)]
pub struct Task {
    /// 任务名
    pub task_name: Option<String>,

    /// 下载路径
    pub download_path: Option<String>,

    /// 种子文件的路径
    pub file_path: String,
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

/// 解析 torrent 链接，支持磁力链接和文件哈希值
#[route(code = 1001)]
pub async fn parse_torrent_link(_link: String) -> Ret<TorrentRet> {
    todo!("parse_torrent_link")
}

/// 解析本地种子文件
#[route(code = 1002)]
pub async fn parse_torrent_file(file_path: String) -> Ret<TorrentRet> {
    let torrent = Torrent::parse_torrent(file_path.as_str()).unwrap();
    let info = torrent.info;
    let mut files = Vec::new();
    let name = info.name;
    let length = info.length;
    let info_hash = info.md5sum;
    let comment = torrent.comment;

    for file in info.files {
        files.push(File {
            path: file.path,
            length: file.length,
        });
    }
    let ret = TorrentRet {
        name,
        length,
        info_hash,
        comment,
        files,
    };

    Ret::ok(ret)
}

/// 添加下载任务
#[route(code = 1003)]
pub async fn add_task(_task: Task) -> Ret<bool> {
    Ret::ok(true)
}