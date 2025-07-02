use crate::config::DATABASE_CONN_LIMIT;
use crate::db::Db;
use crate::dht::command::Spread;
use crate::dht::DHT;
use crate::emitter::Emitter;
use crate::mapper;
use crate::runtime::Runnable;
use crate::udp_server::UdpServer;
use core::str::FromStr;
use std::net::SocketAddr;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// 从 DHT 中获取 peer
#[tokio::test]
#[ignore]
#[cfg_attr(miri, ignore)]
async fn test_peer_from_dht() {
    let info_hash: [u8; 20] = hex::decode("c6bbdb50bd685bacf8c0d615bb58a3a0023986ef").unwrap().try_into().unwrap();
    let id = 0;
    let addr = SocketAddr::from_str("192.168.2.242:3115").unwrap(); 
    
    let db = Db::new(mapper::DB_SAVE_PATH, mapper::DB_NAME, mapper::INIT_SQL, DATABASE_CONN_LIMIT).unwrap();
    let context = crate::core::bootstrap::load_context(db).await;
    info!("node_id: {:?}", context.get_node_id());

    // 命令发射器
    let emitter = Emitter::new();
    let udp_server = UdpServer::new(context.clone()).await.unwrap();
    let udp_server_handle = tokio::spawn(udp_server.clone().run());

    let dht = DHT::new(id, info_hash, emitter.clone(), context, CancellationToken::new(), udp_server);
    let dht_handle = tokio::task::spawn(dht.run());
    
    // 发送一个查找 peer 的请求
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await; // 避免 dht 还未注册到 emitter 中
    emitter.send(&DHT::get_transfer_id(id), Spread { addr }.into()).await.unwrap();
    
    dht_handle.await.unwrap();
    udp_server_handle.await.unwrap();
}