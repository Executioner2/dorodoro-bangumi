//! 种子文件解析下载测试

use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use dorodoro_bangumi::bencoding;
use dorodoro_bangumi::bt::parse::Torrent;
use dorodoro_bangumi::parse::Parse;
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use rand::Rng;
use std::fs;
use std::net::UdpSocket;
use dorodoro_bangumi::util::bytes::Bytes2Int;

#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_parse_bencoded_string() {
    let bytes = fs::read("tests/resources/test1.torrent").unwrap();
    let data = bencoding::decode(Bytes::from_owner(bytes)).unwrap();
    println!("decoded data: {:?}", data);
}

#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_parse_announce_file() {
    let bytes = fs::read("tests/resources/announce").unwrap();
    let data = bencoding::decode(Bytes::from_owner(bytes)).unwrap();
    println!("decoded data: {:?}", data);
    let peers = data
        .as_dict()
        .unwrap()
        .get("peers")
        .unwrap()
        .as_str()
        .unwrap();
    println!("peers: {:?}", peers);
}

/// 种子文件解析测试
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_parse_torrent_file() {
    assert!(Torrent::parse_torrent("tests/resources/test2.torrent").is_ok())
}

/// UDP tracker 握手测试
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_udp_tracker_handshake() {
    let torrent = Torrent::parse_torrent("tests/resources/test3.torrent").unwrap();

    println!("tracker: {}", torrent.announce);

    let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(15))).unwrap();
    // socket.connect("tracker.torrent.eu.org:451").unwrap();

    let mut request = Vec::new();
    let transaction_id = rand::rng().random::<u32>();
    request.write_u64::<BigEndian>(0x41727101980).unwrap(); // protocol_id
    request.write_u32::<BigEndian>(0).unwrap(); // action = 0 (connect)
    request.write_u32::<BigEndian>(transaction_id).unwrap(); // transaction_id
    println!("send_transaction_id: {}", transaction_id);

    socket.send_to(&request, "tracker.torrent.eu.org:451").unwrap();
    let mut response = [0u8; 16];
    let (size, _) = socket.recv_from(&mut response).unwrap();
    println!("收到的响应大小: {}\n收到的数据: {:?}", size, response);

    let action = u32::from_be_slice(&response[0..4]);
    let transaction_id = u32::from_be_slice(&response[4..8]);
    let connection_id = u64::from_be_slice(&response[8..16]);

    println!("action: {}, transaction_id: {}, connection_id: {}", action, transaction_id, connection_id)
}

/// HTTP tracker 握手测试
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_http_tracker_handshake() -> Result<(), Box<dyn std::error::Error>> {
    let torrent = Torrent::parse_torrent("tests/resources/test3.torrent").unwrap();

    let announce = "http://open.acgtracker.com:1096/announce";
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
        "{}?info_hash={}&peer_id={}&port={}&uploaded=0&downloaded={}",
        announce, encoded_info, encoded_peer, port, downloaded
    );

    let response = reqwest::blocking::get(url)?;
    println!("url: {}", response.url());
    let response = response.bytes()?;
    println!("response: {:?}", response);
    let response = bencoding::decode(response).unwrap();
    println!("decoded response: {:?}", response);
    Ok(())
}
