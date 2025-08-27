use std::sync::Arc;

#[cfg(target_has_atomic = "64")]
use std::sync::atomic::AtomicU64;
#[cfg(not(target_has_atomic = "64"))]
use portable_atomic::AtomicU64;

use tracing::info;

use crate::task_manager::PeerId;
use crate::torrent::{Parse, Torrent};
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::{AnnounceInfo, Event};

/// HTTP tracker 握手测试
#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_announce() -> Result<(), Box<dyn std::error::Error>> {
    let torrent = Torrent::parse_torrent("tests/resources/Grisaia Phantom Trigger - 12.torrent")?;
    let peer_id = doro_util::rand::gen_peer_id();
    let announce = "http://nyaa.tracker.wf:7777/announce".to_string();
    let mut tracker = HttpTracker::new(announce, Arc::new(torrent.info_hash), PeerId::new(peer_id));

    let info = AnnounceInfo {
        download: Arc::new(AtomicU64::new(0)),
        uploaded: Arc::new(AtomicU64::new(0)),
        resource_size: torrent.info.length,
        port: 9987,
    };

    let response = tracker.announcing(Event::Started, &info).await?;
    info!("response: {:?}", response);
    Ok(())
}
