use byteorder::{BigEndian, WriteBytesExt};
use crate::bt::pe_crypto;
use crate::{default_logger, util};
use tokio::net::TcpStream;
use tracing::{error, info, Level};
use crate::bt::pe_crypto::CryptoProvide;
use crate::peer::reserved;
use crate::protocol::{BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN};

default_logger!(Level::DEBUG);

#[tokio::test]
async fn test_init_handshake() {
    // let socket = TcpStream::connect("106.73.62.197:40370").await.unwrap();
    let socket = TcpStream::connect("192.168.2.242:3115").await.unwrap();
    let info_hash = hex::decode("c6bbdb50bd685bacf8c0d615bb58a3a0023986ef").unwrap();
    info!("开始握手");
    let res = pe_crypto::init_handshake(socket, &info_hash, CryptoProvide::Plaintext).await;
    if let Err(e) = res {
        error!("{}", e);
        return;
    }
    let mut socket = res.unwrap();
    
    // 发送 peer 握手数据包
    let peer_id = util::rand::gen_peer_id();
    let mut bytes =
        Vec::with_capacity(1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN);
    WriteBytesExt::write_u8(&mut bytes, BIT_TORRENT_PROTOCOL_LEN).unwrap();
    std::io::Write::write(&mut bytes, BIT_TORRENT_PROTOCOL).unwrap();
    let ext = reserved::LTEP | reserved::DHT;
    WriteBytesExt::write_u64::<BigEndian>(&mut bytes, ext).unwrap();
    std::io::Write::write(&mut bytes, &info_hash).unwrap();
    std::io::Write::write(&mut bytes, &peer_id).unwrap();
    socket.write_all(&bytes).await.unwrap();

    socket.readable().await.unwrap();
    let mut handshake_resp = vec![0u8; bytes.len()];
    let size = socket.read(&mut handshake_resp).await.unwrap();
    if size != bytes.len() {
        error!("响应数据长度与预期不符 [{}]\t[{}]", size, bytes.len());
        return;
    }

    let protocol_len = u8::from_be_bytes([handshake_resp[0]]) as usize;
    info!("协议长度: {}", protocol_len);
    
    let resp_info_hash = &handshake_resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
    let peer_id = &handshake_resp[1 + protocol_len + 8 + 20..];
    
    info!("对端响应的 info_hash: {}", hex::encode(resp_info_hash));
    info!("对端响应的 peer_id: {}", hex::encode(peer_id));
    assert_eq!(info_hash, resp_info_hash, "没有在讨论同一个资源文件");
}
