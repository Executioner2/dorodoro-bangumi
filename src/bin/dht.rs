//! 这是 dht 的一个演示 demo。包含 node 维护，torrent 元数据获取

use bytes::Bytes;
use dorodoro_bangumi::bencoding::{BEncodeHashMap, BEncoder};
use dorodoro_bangumi::buffer::ByteBuffer;
use dorodoro_bangumi::bytes::Bytes2Int;
use dorodoro_bangumi::tracker::Host;
use dorodoro_bangumi::{bencoding, hashmap, BoxWrapper};
use hashlink::LinkedHashMap;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;

static _BOOTSTRAP: [&str; 4] = [
    "router.bittorrent.com",
    "router.utorrent.com",
    "router.bitcomet.com",
    "dht.transmissionbt.com",
];
static NODE_ID: &[u8; 20] = b"adkoqwei123jk3341ks0";

fn distancemetric(a: &[u8], b: &[u8]) -> [u8; 20] {
    if a.len() != 20 || b.len() != 20 {
        return [255u8; 20];
    }
    let mut res = [0u8; 20];
    for i in 0..20 {
        res[i] = a[i] ^ b[i];
    }
    res
}

// impl PartialOrd for [u8; 20] {
//
// }

#[tokio::main]
async fn main() {
    // 我们先尝试向其中一个 bootstrap 发送 dht ping
    // let addr = "31.13.95.37:6881".parse::<SocketAddr>().unwrap();

    let mut queue = VecDeque::new();
    let addr = "192.168.2.177:3115".parse::<SocketAddr>().unwrap();
    // let addr = "74.105.197.144:61883".parse::<SocketAddr>().unwrap();
    // let addr = "46.138.71.8:44626".parse::<SocketAddr>().unwrap();
    queue.push_back(addr);
    // let info_hash = hex::decode("a4a88248f0b76a3ff7d7c9bd7a7a134c12090cbe").unwrap();
    let info_hash = hex::decode("d29f9f178a057aed06c7a1b986f8c7b725ff3bd8").unwrap();
    let mut min_dist = distancemetric(NODE_ID, &info_hash);
    let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();

    while !queue.is_empty() {
        let addr = queue.pop_front().unwrap();
        println!("尝试对{}node的访问", addr);
        let mut ping: LinkedHashMap<&str, Box<dyn BEncoder>> = LinkedHashMap::new();
        let t = b"12";
        ping.insert("t", t.to_box());
        ping.insert("y", b"q".to_box());
        ping.insert("q", b"ping".to_box());
        ping.insert(
            "a",
            hashmap!("id" => NODE_ID.to_box() as Box<dyn BEncoder>).to_box(),
        );
        
        // 发送一个 ping
        let data = ping.encode();
        println!("发送一个ping");
        let res = tokio::time::timeout(Duration::from_secs(10), socket.send_to(&data, addr)).await;
        if res.is_err() {
            println!("访问超时");
            continue;
        }
        let res = res.unwrap();
        // let res = socket.send_to(&data, addr).await;
        if res.is_err() {
            println!("拒绝了ping");
            continue;
        }

        // println!("等待消息可读");
        // if socket.readable().await.is_err() {
        //     println!("不是很可读");
        //     continue;
        // }

        println!("从{}那里读取ping的数据", addr);
        let mut buff = ByteBuffer::new(5120);
        let res = tokio::time::timeout(Duration::from_secs(10), socket.recv_from(&mut buff.as_mut())).await;
        if res.is_err() {
            println!("读取ping失败超时");
            continue;
        }
        let res = res.unwrap();
        // let res = socket.recv_from(&mut buff.as_mut()).await;
        if res.is_err() {
            println!("读取ping失败");
            continue;
        }

        let (n, _) = res.unwrap();
        buff.resize(n);
        if bencoding::decode(Bytes::from_owner(buff)).is_err() {
            continue;
        }

        println!("这个[{}]老表罗觉，可以连通", addr);

        // 告诉我这个种子的peers
        let mut get_peers: HashMap<&str, Box<dyn BEncoder>> = HashMap::new();
        get_peers.insert("t", t.to_box());
        get_peers.insert("y", b"q".to_box());
        get_peers.insert("q", b"get_peers".to_box());
        get_peers.insert(
            "a",
            hashmap!(
                "id" => NODE_ID.to_box() as Box<dyn BEncoder>,
                "info_hash" => info_hash.clone().to_box()
            )
            .to_box(),
        );

        let data = get_peers.encode();
        if socket.send_to(&data, addr).await.is_err() {
            continue;
        }

        if socket.readable().await.is_err() {
            continue;
        }

        let mut buff = ByteBuffer::new(5120);
        let res = socket.recv_from(&mut buff.as_mut()).await;
        if res.is_err() {
            continue;
        }
        let (n, _) = res.unwrap();
        buff.resize(n);
        let encode = bencoding::decode(Bytes::from_owner(buff));
        if encode.is_err() {
            continue;
        }
        let encode = encode.unwrap();

        let map = encode.as_dict().unwrap();
        let map = map.get_dict("r").unwrap();
        if map.contains_key("nodes") {
            let nodes = map.get_bytes_conetnt("nodes").unwrap();
            let mut min = min_dist.clone();
            nodes.chunks(26).for_each(|data| {
                let dist = distancemetric(&data[..20], &info_hash);
                if min_dist >= dist {
                    let id = hex::encode(&data[..20]);
                    let ip_bytes: [u8; 4] = data[20..24].try_into().unwrap();
                    let addr = Host::from((ip_bytes, u16::from_be_slice(&data[4..])));
                    println!("加入了一个新的node: {}\taddr: {:?}", id, addr);
                    queue.push_back(addr.into());
                    min = min.min(dist);
                }
            });
            min_dist = min;
        } else if map.contains_key("peers") {
            println!("卧槽，找到可用的 peers 了");
            let peers = map.get_bytes_conetnt("peers").unwrap();
            let peers = peers.chunks(6).map(|data| {
                let ip_bytes: [u8; 4] = data[20..24].try_into().unwrap();
                Host::from((ip_bytes, u16::from_be_slice(&data[4..])))
            }).collect::<Vec<Host>>();
            println!("peers: {:?}", peers);
        }
    }
}
