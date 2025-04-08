use crate::torrent::{Parse, Torrent};
use crate::tracker::http_tracker::HttpTracker;
use crate::tracker::{Event, gen_peer_id};

/// HTTP tracker 握手测试
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_announce() -> Result<(), Box<dyn std::error::Error>> {
    let torrent = Torrent::parse_torrent("tests/resources/test3.torrent").unwrap();
    let peer_id = gen_peer_id();
    let announce = "http://nyaa.tracker.wf:7777/announce";
    let tracker = HttpTracker::new(
        announce,
        &torrent.info_hash,
        peer_id,
        0,
        torrent.info.length,
        0,
        3315,
    );

    let response = tracker.announcing(Event::Started).unwrap();
    println!("response: {:?}", response);
    Ok(())
}
