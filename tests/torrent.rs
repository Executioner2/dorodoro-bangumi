//! 种子文件解析下载测试

use bendy::decoding::FromBencode;
use byteorder::{BigEndian, WriteBytesExt};
use dorodoro_bangumi::bt::torrent::Torrent;
use dorodoro_bangumi::default_logger;
use dorodoro_bangumi::torrent::Parse;
use dorodoro_bangumi::tracker::http_tracker::Announce;
use dorodoro_bangumi::util::bytes_util::Bytes2Int;
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use rand::Rng;
use std::fs;
use std::net::UdpSocket;
use tracing::{Level, debug};

default_logger!(Level::DEBUG);

#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_parse_bencoded_string() {
    let bytes = fs::read("tests/resources/test6.torrent").unwrap();
    let data = Torrent::parse_torrent(bytes).unwrap();
    debug!("decoded data: {:?}", data);
}

#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_parse_announce_file() {
    let bytes = fs::read("tests/resources/announce").unwrap();
    let data = Announce::from_bencode(&bytes).unwrap();
    debug!("decoded data: {:?}", data);
    let peers = data.peers;
    debug!("peers: {:?}", peers);
}

/// 种子文件解析测试
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_parse_torrent_file() {
    assert!(Torrent::parse_torrent("tests/resources/test2.torrent").is_ok())
}

/// 验证 info hash 正确性
#[test]
#[cfg_attr(miri, ignore)]
fn test_info_hash() {
    let torrent = Torrent::parse_torrent("tests/resources/test3.torrent").unwrap();
    let s = hex::encode(torrent.info_hash);
    assert_eq!(s, "a4a88248f0b76a3ff7d7c9bd7a7a134c12090cbe");
}

/// UDP tracker 握手测试
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_udp_tracker_handshake() {
    let torrent = Torrent::parse_torrent("tests/resources/test6.torrent").unwrap();

    debug!("tracker: {}", torrent.announce);

    let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(15)))
        .unwrap();
    // socket.connect("tracker.torrent.eu.org:451").unwrap();

    let mut request = Vec::new();
    let transaction_id = rand::rng().random::<u32>();
    request.write_u64::<BigEndian>(0x41727101980).unwrap(); // protocol_id
    request.write_u32::<BigEndian>(0).unwrap(); // action = 0 (connect)
    request.write_u32::<BigEndian>(transaction_id).unwrap(); // transaction_id
    debug!("send_transaction_id: {}", transaction_id);

    socket
        .send_to(&request, "tracker.torrent.eu.org:451")
        .unwrap();
    let mut response = [0u8; 16];
    let (size, _) = socket.recv_from(&mut response).unwrap();
    debug!("收到的响应大小: {}\n收到的数据: {:?}", size, response);

    let action = u32::from_be_slice(&response[0..4]);
    let transaction_id = u32::from_be_slice(&response[4..8]);
    let connection_id = u64::from_be_slice(&response[8..16]);

    debug!(
        "action: {}, transaction_id: {}, connection_id: {}",
        action, transaction_id, connection_id
    )
}

/// HTTP tracker 握手测试
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_http_tracker_handshake() -> Result<(), Box<dyn std::error::Error>> {
    let torrent = Torrent::parse_torrent("tests/resources/test3.torrent")?;

    let announce = "http://nyaa.tracker.wf:7777/announce";
    let port = "6881";
    let _left = "0";
    let downloaded = "0";
    let peer_id: [u8; 20] = {
        let mut id = [0u8; 20];
        id[0] = b'-';
        id[1..3].copy_from_slice(b"MY");
        id[3..7].copy_from_slice(b"0001");
        id[7..].copy_from_slice(&rand::random::<[u8; 13]>());
        id
    };
    let encoded_info = percent_encode(&torrent.info_hash, NON_ALPHANUMERIC).to_string();
    let encoded_peer = percent_encode(&peer_id, NON_ALPHANUMERIC).to_string();

    let url = format!(
        "{}?info_hash={}&peer_id={}&port={}&uploaded=0&downloaded={}&compact=1",
        announce, encoded_info, encoded_peer, port, downloaded
    );

    let response = reqwest::blocking::get(url)?;
    debug!("url: {}", response.url());
    let response = response.bytes()?;
    debug!("response: {:?}", response);

    let response = Announce::from_bencode(&response).unwrap();
    debug!("decoded response: {:?}", response);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_piece_hash() {
    let torrent = Torrent::parse_torrent("tests/resources/test6.torrent").unwrap();
    debug!("文件序: {:?}", torrent.info.files);
    debug!("tracker: {}", torrent.announce);
    for (i, data) in torrent.info.pieces.chunks(20).enumerate() {
        let x = hex::encode(data);
        debug!("第[{}]个分块的hash: {}", i, x)
    }
}
