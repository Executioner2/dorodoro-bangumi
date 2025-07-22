use crate::api::task_api::{File, Task, TorrentRet};
use crate::torrent::{Parse, Torrent};
use anyhow::Result;

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
    todo!("add task: {:#?}", task)
}
