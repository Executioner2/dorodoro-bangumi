//! udp_tracker 的单元测试

use crate::bytes::Bytes2Int;
use crate::parse::{Parse, Torrent};
use crate::tracker;
use crate::tracker::udp_tracker::UdpTracker;
use crate::tracker::udp_tracker::socket::SocketArc;

/// 测试是否能发起 connect 请求
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_connect() {
    let socket = SocketArc::new().unwrap();
    let info_hash = [0u8; 20];
    let peer_id = tracker::gen_peer_id();
    let mut tracker = UdpTracker::new(
        socket.clone(),
        "tracker.torrent.eu.org:451",
        &info_hash,
        &peer_id,
        0,
        9999,
        9987,
    )
    .unwrap();
    println!("connect before: {:?}", tracker.connect);
    tracker.update_connect().unwrap();
    println!("connect after: {:?}", tracker.connect);
}

/// 测试是否能发起 announce 请求
#[test]
#[cfg_attr(miri, ignore)]
fn test_announce() {
    let torrent = Torrent::parse_torrent("tests/resources/test3.torrent").unwrap();

    let socket = SocketArc::new().unwrap();
    let peer_id = tracker::gen_peer_id();
    let mut tracker = UdpTracker::new(
        socket.clone(),
        "tracker.torrent.eu.org:451",
        &torrent.info_hash,
        &peer_id,
        0,
        torrent.info.length,
        9987,
    )
    .unwrap();
    let announce = tracker.announcing().unwrap();
    println!("announce result: {:?}", announce);
}

#[test]
fn test_parse_peers() {
    let peers: [i32; 6] = [103, 151, 172, 91, 39, 3];
    let peers: [[u8; 4]; 6] = peers.map(|x| x.to_be_bytes());
    let peers: [u8; 24] = unsafe { std::mem::transmute_copy(&peers) };
    let ip1 = u32::from_be_slice(&peers[0..4]);
    let ip2 = u32::from_be_slice(&peers[4..8]);
    let ip3 = u32::from_be_slice(&peers[8..12]);
    let ip4 = u32::from_be_slice(&peers[12..16]);
    println!("ip1: {}, ip2: {}, ip3: {}, ip4: {}", ip1, ip2, ip3, ip4)
    // let port1 = Bytes2Int::from_bytes(&peers[16..18]);
}
