//! BT 调度器

use crate::peer::PeerManager;
use crate::torrent::Torrent;

/// 上下文
struct Context {
    torrent: Vec<Torrent>
}

/// 调度器
pub struct Scheduler {
    context: Context,
    peer_manager: PeerManager
}

impl Scheduler {
    /// 启动调度器
    pub fn start() {

    }
}