use dorodoro_bangumi::core::protocol;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[tokio::test]
#[ignore]
async fn test_framework() {
    let mut socket = TcpStream::connect("127.0.0.1:3315").await.unwrap();
    let mut bytes = vec![];
    bytes.write_u8(19).await.unwrap();
    bytes.write(b"BitTorrent protocol").await.unwrap();
    socket.write(&bytes).await.unwrap();

    let mut bytes = vec![];
    bytes.write_u32(13).await.unwrap();
    bytes.write(b"Hello, world!").await.unwrap();
    socket.write(&bytes).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_framework_2() {
    let mut socket = TcpStream::connect("127.0.0.1:3315").await.unwrap();
    let mut bytes = vec![];
    bytes.write_u8(16).await.unwrap();
    bytes.write(b"dorodoro-bangumi").await.unwrap();
    socket.write(&bytes).await.unwrap();

    socket.write(&[1u8]).await.unwrap();
    socket.write(&[2u8]).await.unwrap();
    socket.write(&[0u8]).await.unwrap();
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
    bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();

    socket1.write(&bytes).await.unwrap();

    let mut socket2 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();

    socket2.write(&bytes).await.unwrap();
}

/// 测试延迟链接会不会导致读取数据时阻塞
#[tokio::test]
async fn test_lazy_send() {
    let mut socket0 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();
    socket0.write(&bytes).await.unwrap(); // socket0 连接上
    drop(socket0); // 观察 controller 是否会将 Exit 事件送达给 Tcp Server

    let mut socket1 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();
    socket1.write(&bytes).await.unwrap(); // socket1 连接上

    tokio::spawn(async move {
        let mut socket2 = TcpStream::connect("127.0.0.1:3300").await.unwrap();
        socket2
            .write(&[protocol::REMOTE_CONTROL_PROTOCOL.len() as u8])
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        socket2
            .write(protocol::REMOTE_CONTROL_PROTOCOL)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_secs(5)).await;
    socket1.write(&[0u8]).await.unwrap();
}

/// 发送添加种子的命令
#[tokio::test]
async fn test_add_torrent() {
    let mut socket = TcpStream::connect("127.0.0.1:3300").await.unwrap();
    let mut bytes = vec![];
    bytes
        .write_u8(protocol::REMOTE_CONTROL_PROTOCOL.len() as u8)
        .await
        .unwrap();
    bytes
        .write(protocol::REMOTE_CONTROL_PROTOCOL)
        .await
        .unwrap();
    socket.write(&bytes).await.unwrap(); // socket 连接上

    socket.write(&[1u8]).await.unwrap();
}
