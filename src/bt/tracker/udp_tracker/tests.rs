//! udp_tracker 的单元测试

use crate::tracker::udp_tracker::socket::SocketArc;
use crate::tracker::udp_tracker::UdpTracker;

/// 测试UDP连接是否成功
#[test]
fn test_connect() {
    let socket = SocketArc::new().unwrap();
    let mut tracker = UdpTracker::new(socket.clone(), "tracker.torrent.eu.org:451").unwrap();
    println!("connect before: {:?}", tracker.connect);
    tracker.update_connect().unwrap();
    println!("connect after: {:?}", tracker.connect);
}