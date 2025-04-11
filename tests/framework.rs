use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[tokio::test]
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