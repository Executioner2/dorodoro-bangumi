use crate::torrent::{Parse, Torrent};
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::{Event, gen_peer_id, AnnounceInfo};

/// HTTP tracker 握手测试
#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_announce() -> Result<(), Box<dyn std::error::Error>> {
    let torrent = Torrent::parse_torrent("tests/resources/test3.torrent").unwrap();
    let peer_id = gen_peer_id();
    let announce = "http://nyaa.tracker.wf:7777/announce";
    let tracker = HttpTracker::new(
        announce,
        &torrent.info_hash,
        &peer_id,
    );

    let info = AnnounceInfo {
        download: 0,
        left: torrent.info.length,
        uploaded: 0,
        port: 9987,
    };

    let response = tracker.announcing(Event::Started, info).await?;
    println!("response: {:?}", response);
    Ok(())
}
