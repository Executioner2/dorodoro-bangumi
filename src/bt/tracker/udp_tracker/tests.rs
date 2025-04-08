//! udp_tracker 的单元测试

use crate::bt::peer::MsgType;
use crate::bytes::Bytes2Int;
use crate::torrent::{Parse, Torrent};
use crate::tracker;
use crate::tracker::Event;
use crate::tracker::udp_tracker::socket::SocketBuilder;
use crate::tracker::udp_tracker::UdpTracker;
use byteorder::{BigEndian, WriteBytesExt};
use sha1::{Digest, Sha1};
use std::cmp::min;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedReadHalf;
use tokio::runtime::Builder;
use tokio::time::timeout;

/// 测试是否能发起 connect 请求
#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_connect() {
    let socket = SocketBuilder::new().build().unwrap();
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

    let socket = SocketBuilder::new().build().unwrap();
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
    let announce = tracker.announcing(Event::None).unwrap();
    println!("announce result: {:?}", announce);
    println!("peers length: {}", announce.peers.len());
}

// ===========================================================================
// 以下的测试是功能可行性的探索，而非功能的单元测试
// ===========================================================================

/// 测试 peers 的解析
#[test]
#[ignore]
fn test_parse_peers() {
    let peers = [
        36u8, 250, 241, 179, 110, 64, 36, 250, 249, 115, 79, 92, 44, 222, 241, 211, 207, 30, 61,
        160, 241, 102, 92, 213, 103, 151, 172, 91, 39, 3, 110, 85, 5, 71, 105, 100, 111, 255, 124,
        205, 125, 56, 118, 86, 250, 192, 35, 39, 124, 225, 118, 4, 132, 57, 183, 131, 169, 60, 80,
        180, 194, 166, 72, 33, 97, 192, 223, 81, 251, 245, 107, 101, 223, 73, 207, 230, 49, 15,
        221, 113, 116, 221, 134, 251, 219, 140, 54, 93, 246, 145, 218, 84, 89, 49, 228, 158, 202,
        168, 187, 166, 26, 225, 193, 239, 86, 3, 229, 97, 192, 166, 244, 43, 40, 200, 183, 213, 86,
        178, 112, 115, 183, 208, 208, 46, 246, 243, 183, 135, 152, 33, 11, 13, 180, 176, 65, 94,
        65, 241, 175, 0, 74, 25, 246, 243, 125, 111, 236, 135, 246, 243, 124, 235, 125, 14, 246,
        243, 123, 152, 189, 103, 246, 243, 120, 234, 200, 54, 118, 43, 120, 231, 168, 156, 77, 248,
        119, 92, 10, 155, 246, 243, 117, 172, 254, 194, 216, 71, 117, 170, 79, 92, 222, 158, 117,
        148, 110, 82, 26, 225, 117, 135, 95, 190, 27, 89, 115, 60, 16, 32, 217, 164, 113, 201, 50,
        8, 221, 213, 113, 132, 201, 238, 42, 25, 111, 196, 244, 15, 65, 241, 106, 41, 14, 209, 183,
        111, 103, 151, 172, 91, 105, 199, 73, 35, 152, 121, 200, 213, 66, 183, 69, 17, 89, 87, 61,
        230, 7, 141, 62, 255, 61, 93, 148, 124, 133, 71, 61, 58, 160, 9, 209, 180, 60, 135, 104,
        55, 246, 243, 58, 252, 122, 78, 105, 1, 39, 64, 41, 68, 246, 243, 27, 9, 20, 84, 110, 189,
        14, 112, 106, 31, 26, 225,
    ];

    assert_eq!(peers.len() % 6, 0, "peers length should be a multiple of 6");
    let peers = peers
        .chunks(6)
        .map(|chunk| {
            (
                chunk[0],
                chunk[1],
                chunk[2],
                chunk[3],
                u32::from_be_slice(&chunk[4..6]),
            )
        })
        .collect::<Vec<(u8, u8, u8, u8, u32)>>();
    println!("peers: {:?}", peers);
}

/// 请求 peer 下载分块
#[test]
#[ignore]
fn test_request_download() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(request_download())
}

async fn request_download() {
    // 本机的 peer 客户端
    let torrent = Arc::new(Torrent::parse_torrent("tests/resources/test3.torrent").unwrap());

    let protocol_len = 19u8;
    let protocol = b"BitTorrent protocol";
    let retain = 0u64;
    let info_hash = torrent.info_hash;
    let peer_id = b"L4AZCBzQ_h5yo6djjbSL";
    println!("peer_id: {}", String::from_utf8_lossy(peer_id));
    println!("info_hash: {}", hex::encode(&info_hash));

    let peer = "192.168.2.177:3115";
    // 下面两个是直接从 tracker 中获取到的远端 peer。测试是可以连接上并下载资源的。
    // let peer = "175.0.72.46:63219";
    // let peer = "124.91.148.150:25667";
    let stream = TcpStream::connect(peer).await.unwrap();
    println!("启动的地址: {:?}", stream.local_addr().unwrap());
    let (mut reader, mut writer) = stream.into_split();

    // 发送握手请求
    let mut handshake = vec![];
    WriteBytesExt::write_u8(&mut handshake, protocol_len).unwrap();
    std::io::Write::write(&mut handshake, protocol).unwrap();
    WriteBytesExt::write_u64::<BigEndian>(&mut handshake, retain).unwrap();
    std::io::Write::write(&mut handshake, &info_hash).unwrap();
    std::io::Write::write(&mut handshake, peer_id).unwrap();
    writer.write_all(&handshake).await.unwrap();

    let mut handshake_resp = vec![0u8; handshake.len()];

    reader.readable().await.unwrap();
    let size = reader.read(&mut handshake_resp).await.unwrap();
    let protocol_len = u8::from_be_bytes([handshake_resp[0]]) as usize;
    let resp_info_hash = &handshake_resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
    let peer_id = &handshake_resp[1 + protocol_len + 8 + 20..];
    let peer_id_str = String::from_utf8_lossy(peer_id);

    println!("请求大小: {}\t响应大小: {}", handshake.len(), size);
    println!("响应内容: {:?}", handshake_resp);
    println!("是否在讨论同一个文件？: {}", info_hash == resp_info_hash);
    println!("对方的peer_id: {}", peer_id_str);
    println!("文件大小: {}", torrent.info.length);

    if info_hash != resp_info_hash {
        println!("没有在讨论同一个文件");
        return;
    }

    println!(
        "区块数量: {}",
        (torrent.info.length + torrent.info.piece_length - 1) / torrent.info.piece_length
    );

    // return;

    async fn reader_handler(reader: &mut OwnedReadHalf, torrent: &Torrent) {
        loop {
            match timeout(Duration::from_secs(5), reader.readable()).await {
                Ok(Ok(())) => {
                    // println!("reader 可读");
                    ()
                }
                Ok(Err(e)) => {
                    println!("reader 读错误: {}", e);
                    break;
                }
                Err(_) => {
                    println!("reader 读超时");
                    break;
                }
            }
            // println!("================分割线================");

            let mut length = [0u8; 4];
            reader.read_exact(&mut length).await.unwrap();
            let length = u32::from_be_bytes(length);
            // println!("对方响应的长度: {}", length);

            let mut resp = vec![0u8; length as usize];
            reader.read_exact(&mut resp).await.unwrap();
            // println!("对方响应的内容: {:?}", resp);

            if let Ok(msg_type) = MsgType::try_from(resp[0]) {
                match msg_type {
                    MsgType::Piece => {
                        let data = &resp[1..];
                        writer_handler(data, &torrent.info.name, torrent.info.piece_length).await;
                    }
                    _ => {
                        println!("其它响应，暂不处理: {:?}", resp)
                    }
                }
            }
            break;
        }
    }

    async fn writer_handler(writer: &[u8], filename: &str, piece_length: u64) {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(filename)
            .unwrap();

        let piece_index = u32::from_be_slice(&writer[0..4]);
        let block_offset = u32::from_be_slice(&writer[4..8]);
        let data = &writer[8..];
        file.seek(SeekFrom::Start(
            piece_index as u64 * piece_length + block_offset as u64,
        ))
        .unwrap();
        file.write_all(data).unwrap();
        file.flush().unwrap();
        // if piece_index == 15 {
        //     println!("写入文件成功: {}-{}-{}", piece_index, block_offset, data.len());
        // }
    }

    println!("================分割线================");

    // 告诉对方，我是一点东西都没有
    println!("告诉对方，我是一点东西都没有");
    let mut req = vec![];
    // let pieces_len = torrent.info.pieces.len();
    let bitmap = vec![0; 2];
    WriteBytesExt::write_u32::<BigEndian>(&mut req, 1 + bitmap.len() as u32).unwrap();
    WriteBytesExt::write_u8(&mut req, MsgType::Bitfield as u8).unwrap();
    std::io::Write::write(&mut req, &bitmap).unwrap();
    writer.write_all(&req).await.unwrap();
    reader_handler(&mut reader, &torrent).await;

    println!("告诉对方允许对方发起请求");
    let mut req = vec![];
    WriteBytesExt::write_u32::<BigEndian>(&mut req, 1).unwrap();
    WriteBytesExt::write_u8(&mut req, MsgType::UnChoke as u8).unwrap();
    writer.write_all(&req).await.unwrap();
    reader_handler(&mut reader, &torrent).await;

    // 同时再告诉对方，我对你的资源感兴趣
    println!("同时再告诉对方，我对你的资源感兴趣");
    let mut req = vec![];
    WriteBytesExt::write_u32::<BigEndian>(&mut req, 1).unwrap();
    WriteBytesExt::write_u8(&mut req, MsgType::Interested as u8).unwrap();
    writer.write_all(&req).await.unwrap();
    reader_handler(&mut reader, &torrent).await;

    // 启动异步读取任务
    // let read_handle = tokio::spawn({
    //     let torrent = Arc::clone(&torrent);
    //     async move {
    //         reader_handler(reader, &torrent).await;
    //     }
    // });

    // 我要开始向你请求数据了
    let sharding_size = 1 << 14; // 分片大小
    let piece_num =
        (torrent.info.length + torrent.info.piece_length - 1) / torrent.info.piece_length;
    for i in 0..piece_num {
        let mut sharding_offset = 0u64;
        let piece_length = torrent.info.piece_length.min(
            torrent
                .info
                .length
                .saturating_sub(i * torrent.info.piece_length),
        );
        println!("循环请求区块: {}\t区块大小: {}", i, piece_length);
        while sharding_offset < piece_length {
            let mut req = vec![];
            WriteBytesExt::write_u32::<BigEndian>(&mut req, 0).unwrap();
            WriteBytesExt::write_u8(&mut req, MsgType::Request as u8).unwrap();
            WriteBytesExt::write_u32::<BigEndian>(&mut req, i as u32).unwrap();
            WriteBytesExt::write_u32::<BigEndian>(&mut req, sharding_offset as u32).unwrap();
            WriteBytesExt::write_u32::<BigEndian>(
                &mut req,
                min(piece_length - sharding_offset, sharding_size) as u32,
            )
            .unwrap();
            // if i == piece_num - 1 {
            //     println!("分片偏移: {}\t下载分片大小: {}", sharding_offset, min(piece_length - sharding_offset, sharding_size))
            // }
            let len = req.len() as u32 - 4;
            req[0..4].copy_from_slice(&len.to_be_bytes());
            writer.write_all(&req).await.unwrap();
            sharding_offset += sharding_size;

            // 阻塞读取
            reader_handler(&mut reader, &torrent).await;
        }

        let mut file = OpenOptions::new()
            .read(true)
            .open(&torrent.info.name)
            .unwrap();
        println!(
            "seek: {}\tpiece_length: {}",
            i * torrent.info.piece_length,
            piece_length
        );
        file.seek(SeekFrom::Start(i * torrent.info.piece_length))
            .unwrap();
        let mut data = vec![0u8; piece_length as usize];
        file.read(&mut data).unwrap();

        let mut hasher = Sha1::new();
        hasher.update(data);
        let mut result = [0; 20];
        result.copy_from_slice(&hasher.finalize());
        let hash = &torrent.info.pieces[i as usize * 20..(i as usize + 1) * 20];
        let check = result == hash;
        if check {
            println!("第{}个区块校验成功", i);
        } else {
            eprintln!(
                "第{}个区块校验不通过\n校验值: {:?}\n实际值: {:?}",
                i, hash, result
            );
            return;
        }
    }

    // tokio::join!(read_handle);
}

/// 握手响应数据解析
#[test]
#[ignore]
fn test_parse_peer_handshake() {
    let resp = [
        19u8, 66, 105, 116, 84, 111, 114, 114, 101, 110, 116, 32, 112, 114, 111, 116, 111, 99, 111,
        108, 0, 0, 0, 0, 0, 24, 0, 5, 164, 168, 130, 72, 240, 183, 106, 63, 247, 215, 201, 189,
        122, 122, 19, 76, 18, 9, 12, 190, 45, 113, 66, 53, 48, 52, 48, 45, 76, 40, 86, 105, 115,
        99, 71, 71, 76, 88, 84, 111,
    ];
    let protocol_len = u8::from_be_bytes([resp[0]]) as usize;
    let protocol = String::from_utf8_lossy(&resp[1..1 + protocol_len]);
    let retain = u64::from_be_slice(&resp[1 + protocol_len..1 + protocol_len + 8]);
    let info_hash = &resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
    let peer_id = &resp[1 + protocol_len + 8 + 20..];
    println!("protocol_len: {}", protocol_len);
    println!("protocol: {}", protocol);
    println!("retain: {}", retain); // 1572869
    println!("info_hash: {}", hex::encode(info_hash));
    println!("peer_id: {}", String::from_utf8_lossy(peer_id)); // -qB5040-L(ViscGGLXTo
}

/// 将数据写入到指定位置的观察测试
#[test]
#[ignore]
fn test_write_data_on_seek_to_file() {
    // 要写入的数据
    // let data = [1u8; 10];
    let filename = "write_test";
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(filename)
        .unwrap();

    let _ = file.seek(SeekFrom::Start(5)).unwrap();
    file.write_all(b"hello world").unwrap();

    let mut file = OpenOptions::new().read(true).open(filename).unwrap();
    let mut read_data = [0u8; 20];
    file.read(&mut read_data).unwrap();
    assert_eq!(read_data[0..5], [0; 5]);
    assert_eq!(read_data[5..16], b"hello world"[..]);
    assert_eq!(read_data[16..], [0; 4]);

    fs::remove_file(filename).unwrap()
}
