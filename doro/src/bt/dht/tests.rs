// use crate::config::DATABASE_CONN_LIMIT;
// use crate::db::Db;
// use crate::dht::command::Spread;
// use crate::dht::DHT;
// use crate::emitter::Emitter;
// use crate::mapper;
// use crate::runtime::Runnable;
// use crate::udp_server::UdpServer;
// use core::str::FromStr;
// use std::net::SocketAddr;
// use tokio_util::sync::CancellationToken;
// use tracing::info;
//
// /// 从 DHT 中获取 peer
// #[tokio::test]
// #[ignore]
// #[cfg_attr(miri, ignore)]
// async fn test_peer_from_dht() {
//     let info_hash: [u8; 20] = hex::decode("c6bbdb50bd685bacf8c0d615bb58a3a0023986ef").unwrap().try_into().unwrap();
//     let id = 0;
//     let addr = SocketAddr::from_str("192.168.2.242:3115").unwrap();
//
//     let db = Db::new(mapper::DB_SAVE_PATH, mapper::DB_NAME, mapper::INIT_SQL, DATABASE_CONN_LIMIT).unwrap();
//     let context = crate::core::bootstrap::load_context(db).await;
//     info!("routing: {:?}", context.get_node_id());
//
//     // 命令发射器
//     let emitter = Emitter::new();
//     let udp_server = UdpServer::new(context.clone()).await.unwrap();
//     let udp_server_handle = tokio::spawn(udp_server.clone().run());
//
//     let dht = DHT::new(id, info_hash, emitter.clone(), context, CancellationToken::new(), udp_server);
//     let dht_handle = tokio::task::spawn(dht.run());
//
//     // 发送一个查找 peer 的请求
//     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await; // 避免 dht 还未注册到 emitter 中
//     emitter.send(&DHT::get_transfer_id(id), Spread { addr }.into()).await.unwrap();
//
//     dht_handle.await.unwrap();
//     udp_server_handle.await.unwrap();
// }

use crate::dht::entity::{DHTBase, FindNodeReq, FindNodeResp, GetPeersReq, GetPeersResp, Ping};
use crate::dht::routing::NodeId;
use anyhow::{Result, anyhow};
use bendy::decoding::FromBencode;
use bendy::encoding::ToBencode;
use core::str::FromStr;
use doro_util::buffer::ByteBuffer;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tracing::info;

async fn domain_node_dns_resolve(domain: &str) -> Option<SocketAddr> {
    tokio::net::lookup_host(domain).await.ok()?.next()
}

#[tokio::test]
async fn test_bootstrap_node_dns_resolve() -> Result<()> {
    let host = "dht.libtorrent.org:25401";
    // let host = "192.168.2.242:3315";
    let res = domain_node_dns_resolve(host).await;
    if res.is_none() {
        return Err(anyhow!("解析域名失败: {}", host));
    }
    info!("解析出来的ip地址: {}", res.unwrap());
    Ok(())
}

async fn send_ping(socket: &mut UdpSocket, addr: &SocketAddr, id: &NodeId) -> Result<()> {
    let ping = DHTBase::<Ping>::request(Ping::new(id.cow()), "ping".to_string(), 0);

    socket.send_to(&ping.to_bencode().unwrap(), &addr).await?;

    // 接收响应
    let mut buf = ByteBuffer::new(2048);
    let (len, _) = socket.recv_from(buf.as_mut()).await?;
    info!("接收到了响应，长度: {}", len);
    info!("响应原始数据: {:?}", &buf[0..len]);

    match DHTBase::<Ping>::from_bencode(&buf[0..len]) {
        Ok(resp) => {
            info!("响应内容: {:?}", resp);
        }
        Err(e) => {
            return Err(anyhow!("解析响应失败: {}", e));
        }
    }

    Ok(())
}

async fn send_get_peers(
    socket: &mut UdpSocket,
    addr: &SocketAddr,
    id: &NodeId,
    info_hash: &NodeId,
) -> Result<()> {
    let get_peers = DHTBase::<GetPeersReq>::request(
        GetPeersReq::new(id.cow(), info_hash.cow()),
        "get_peers".to_string(),
        0,
    );

    socket
        .send_to(&get_peers.to_bencode().unwrap(), &addr)
        .await?;

    // 接收响应
    let mut buf = ByteBuffer::new(2048);
    let (len, _) = socket.recv_from(buf.as_mut()).await?;
    info!("接收到了响应，长度: {}", len);
    info!("响应原始数据: {:?}", &buf[0..len]);

    match DHTBase::<GetPeersResp>::from_bencode(&buf[0..len]) {
        Ok(resp) => {
            info!("响应内容: {:?}", resp);
        }
        Err(e) => {
            return Err(anyhow!("解析响应失败: {}", e));
        }
    }

    Ok(())
}

async fn send_find_node(
    socket: &mut UdpSocket,
    addr: &SocketAddr,
    id: &NodeId,
    target: &NodeId,
) -> Result<()> {
    let find_node = DHTBase::<FindNodeReq>::request(
        FindNodeReq::new(id.cow(), target.cow()),
        "find_node".to_string(),
        1,
    );

    socket
        .send_to(&find_node.to_bencode().unwrap(), &addr)
        .await?;

    // 接收响应
    let mut buf = ByteBuffer::new(2048);
    let (len, _) = socket.recv_from(buf.as_mut()).await?;
    info!("接收到了响应，长度: {}", len);
    info!("响应原始数据: {:?}", &buf[0..len]);

    match DHTBase::<FindNodeResp>::from_bencode(&buf[0..len]) {
        Ok(resp) => {
            info!("响应内容: {:?}", resp);
        }
        Err(e) => {
            return Err(anyhow!("解析响应失败: {}", e));
        }
    }

    Ok(())
}

#[ignore]
#[cfg_attr(miri, ignore)]
#[tokio::test]
async fn test_request_bootstrap_node() -> Result<()> {
    // let host = "192.168.2.242:3115";
    // let host = "dht.libtorrent.org:25401";
    // let host = "dht.transmissionbt.com:6881";
    // let host = "router.bittorrent.com:6881";
    // let host = "router.bittorrent.com:25401";
    // let host = "router.utorrent.com:6881";
    // let host = "dht.aelitis.com:6881";
    let info_hash_str = "c6bbdb50bd685bacf8c0d615bb58a3a0023986ef";
    // let host = "dht.libtorrent.org:6881";

    // let addr = SocketAddr::from_str("185.157.221.247:25401")?;
    // let addr = SocketAddr::from_str("67.215.246.10:6881")?;
    // let addr = SocketAddr::from_str("87.98.162.88:6881")?;
    let addr = SocketAddr::from_str("82.221.103.244:6881")?;
    // let addr = domain_node_dns_resolve(host).await.unwrap();
    info!("解析出来的ip地址: {}", addr);

    let mut socket = UdpSocket::bind("0.0.0.0:0").await?;

    let id = NodeId::random();
    let info_hash = NodeId::try_from(hex::decode(info_hash_str)?.as_slice())?;

    // 发送 ping 请求
    send_ping(&mut socket, &addr, &id).await?;

    // 发送 get_peers 请求
    send_get_peers(&mut socket, &addr, &id, &info_hash).await?;

    // 发送 find_node 请求
    send_find_node(&mut socket, &addr, &id, &info_hash).await?;

    Ok(())
}
