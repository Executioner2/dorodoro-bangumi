use std::time::Duration;

use doro::api::task_api;
use doro::api::task_api::TorrentSource;
use doro::protocol;
use doro_util::default_logger;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::Level;

default_logger!(Level::DEBUG);

#[tokio::test]
#[ignore]
async fn test_framework() {
    let mut socket = TcpStream::connect("127.0.0.1:3315").await.unwrap();
    let mut bytes = vec![];
    bytes.write_u8(19).await.unwrap();
    let _ = bytes.write(b"BitTorrent protocol").await.unwrap();
    let _ = socket.write(&bytes).await.unwrap();

    let mut bytes = vec![];
    bytes.write_u32(13).await.unwrap();
    let _ = bytes.write(b"Hello, world!").await.unwrap();
    let _ = socket.write(&bytes).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_framework_2() {
    let mut socket = TcpStream::connect("127.0.0.1:3315").await.unwrap();
    let mut bytes = vec![];
    bytes.write_u8(16).await.unwrap();
    let _ = bytes.write(b"dorodoro-bangumi").await.unwrap();
    let _ = socket.write(&bytes).await.unwrap();

    let _ = socket.write(&[1u8]).await.unwrap();
    let _ = socket.write(&[2u8]).await.unwrap();
    let _ = socket.write(&[0u8]).await.unwrap();
}

/// 测试发起控制器连接，测试并发发过去有无问题
#[tokio::test]
#[ignore]
async fn test_connect_controller() {
    let mut socket1 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    let _ = bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();

    let _ = socket1.write(&bytes).await.unwrap();

    let mut socket2 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    let _ = bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();

    let _ = socket2.write(&bytes).await.unwrap();
}

/// 测试延迟链接会不会导致读取数据时阻塞
#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_lazy_send() {
    let mut socket0 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    let _ = bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();
    let _ = socket0.write(&bytes).await.unwrap(); // socket0 连接上
    drop(socket0); // 观察 base 是否会将 Exit 事件送达给 Tcp Server

    let mut socket1 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    let _ = bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();
    let _ = socket1.write(&bytes).await.unwrap(); // socket1 连接上

    tokio::spawn(async move {
        let mut socket2 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
        let _ = socket2
            .write(&[protocol::REMOTE_CONTROL_PROTOCOL.len() as u8])
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let _ = socket2
            .write(protocol::REMOTE_CONTROL_PROTOCOL)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_secs(5)).await;
    let _ = socket1.write(&[0u8]).await.unwrap();
}

/// 发送添加种子的命令
#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_add_torrent() {
    use byteorder::{BigEndian, WriteBytesExt};
    let mut socket = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    WriteBytesExt::write_u8(&mut bytes, protocol::REMOTE_CONTROL_PROTOCOL.len() as u8).unwrap();
    let _ = bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();
    let _ = socket.write(&bytes).await.unwrap(); // socket 连接上

    let request = task_api::Task {
        task_name: Some("好东西".to_string()),
        download_path: Some("./download".to_string()),
        source: TorrentSource::LocalFile("./tests/resources/test6.torrent".to_string()),
        mkdir_torrent_name: Some(true),
    };
    let data = serde_json::to_vec(&request).unwrap();

    let mut btyes = vec![];
    WriteBytesExt::write_u32::<BigEndian>(&mut btyes, 1001).unwrap();
    WriteBytesExt::write_u32::<BigEndian>(&mut btyes, data.len() as u32).unwrap();
    btyes.extend_from_slice(&data);
    let _ = socket.write(&btyes).await.unwrap();

    tokio::time::sleep(Duration::from_secs(1050)).await;
}

/// 发送关机指令
#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_shutdown() {
    let mut socket = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    let _ = bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();
    let _ = socket.write(&bytes).await.unwrap(); // socket 连接上

    let _ = socket.write(&[0u8]).await.unwrap();
}
