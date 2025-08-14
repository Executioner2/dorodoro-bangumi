//! 订阅事件

use crate::torrent::TorrentArc;

/// content 的事件
#[derive(Clone)]
pub enum Event {
    /// 解析磁力链接成功
    ParseSuccess(TorrentArc),
}