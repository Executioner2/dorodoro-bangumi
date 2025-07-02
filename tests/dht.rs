//! DHT 测试

use bendy::decoding::FromBencode;
use bendy::encoding::ToBencode;
use bendy::value::Value;
use dorodoro_bangumi::buffer::ByteBuffer;
use dorodoro_bangumi::default_logger;
use dorodoro_bangumi::dht::entity::{DHTBase, GetPeersReq, GetPeersResp, Ping};
use rand::Rng;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::net::UdpSocket;
use tracing::{Level, error, info};
use dorodoro_bangumi::dht::node_id::NodeId;

default_logger!(Level::DEBUG);
//
// /// 获取通过 DHT 获取种子文件
// #[ignore]
// #[cfg_attr(miri, ignore)]
// #[tokio::test]
// async fn test_get_torrent() -> Result<(), Box<dyn std::error::Error>> {
//     // magnet:?xt=urn:btih:c6bbdb50bd685bacf8c0d615bb58a3a0023986ef&dn=%5BNekomoe%20kissaten%5D%5BHibi%20wa%20Sugiredo%20Meshi%20Umashi%5D%5B08%5D%5B1080p%5D%5BJPTC%5D.mp4&tr=http%3A%2F%2Ft.nyaatracker.com%2Fannounce&tr=http%3A%2F%2Ftracker.kamigami.org%3A2710%2Fannounce&tr=http%3A%2F%2Fshare.camoe.cn%3A8080%2Fannounce&tr=http%3A%2F%2Fopentracker.acgnx.se%2Fannounce&tr=http%3A%2F%2Fanidex.moe%3A6969%2Fannounce&tr=http%3A%2F%2Ft.acg.rip%3A6699%2Fannounce&tr=udp%3A%2F%2Ftr.bangumi.moe%3A6969%2Fannounce&tr=https%3A%2F%2Ftr.bangumi.moe%3A9696%2Fannounce&tr=http%3A%2F%2Fopen.acgtracker.com%3A1096%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce
//     // 种子文件 hash
//     let info_hash_str = "c6bbdb50bd685bacf8c0d615bb58a3a0023986ef";
//     let info_hash = hex::decode(info_hash_str)?;
//
//     // 拥有种子文件的 peer 地址
//     // let peer = "192.168.2.242:3115";
//     // let peer = "123.156.68.196:20252";
//     let peer = "218.102.187.76:12266";
//     let peer = SocketAddr::from_str(peer)?;
//
//     // 创建链接
//     let mut socket = TcpStreamExt::connect(peer).await?;
//
//     // 发起握手请求
//     let mut bytes =
//         Vec::with_capacity(1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN);
//     WriteBytesExt::write_u8(&mut bytes, BIT_TORRENT_PROTOCOL_LEN)?;
//     std::io::Write::write(&mut bytes, BIT_TORRENT_PROTOCOL)?;
//     let ext = reserved::LTEP | reserved::DHT;
//     WriteBytesExt::write_u64::<BigEndian>(&mut bytes, ext)?;
//     std::io::Write::write(&mut bytes, &info_hash)?;
//     let peer_id = util::rand::gen_peer_id();
//     std::io::Write::write(&mut bytes, &peer_id)?;
//     if let Err(e) = socket.write_all(&bytes).await {
//         error!("发送握手请求失败\t{}", e);
//         return Err(e.into());
//     }
//
//     // 接收握手响应
//     if socket.readable().await.is_err() {
//         error!("没有等到响应结果");
//         return Err("No response".into());
//     }
//     let mut handshake_resp = vec![0u8; bytes.len()];
//     let size = socket.read(&mut handshake_resp).await.unwrap();
//     if size != bytes.len() {
//         error!("握手响应长度不一致\tsize: {}", size);
//         return Err(format!("Invalid response, size: {size}").into());
//     }
//
//     let protocol_len = u8::from_be_bytes([handshake_resp[0]]) as usize;
//     let resp_info_hash = &handshake_resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
//     let reserved = u64::from_be_slice(&handshake_resp[1 + protocol_len..1 + protocol_len + 8]);
//     assert_eq!(info_hash, resp_info_hash, "Invalid info hash");
//
//     // 判断是否支持 torrent 下载（LTEP）
//     if reserved & reserved::LTEP == 0 {
//         return Err(format!(
//             "Peer does not support torrent download\nreserved: {:?}",
//             reserved
//         )
//         .into());
//     }
//
//     let (mut read, mut write) = socket.into_split();
//
//     let metadata_size;
//     let ut_metadata;
//
//     // 等待对方发起 extend 协议
//     loop {
//         let resp_type = PeerResp::new(&mut read, &peer).await;
//         if let RespType::Continue(msg_type, mut bytes) = resp_type {
//             if msg_type == MsgType::Port {
//                 info!("port: {:?}", u32::from_be_slice(&bytes[0..4]))
//             } else if msg_type == MsgType::LTEPHandshake {
//                 info!("ltep: {:?}", bytes);
//                 let id = bytes[0];
//                 let bytes = bytes.split_off(1);
//                 if id == 0 {
//                     debug!("是握手消息");
//                 }
//                 let res = bencoding::decode(Bytes::from(bytes)).unwrap();
//                 metadata_size = res.as_dict().unwrap().get_int("metadata_size");
//                 ut_metadata = res
//                     .as_dict()
//                     .unwrap()
//                     .get_dict("m")
//                     .map(|m| m.get_int("ut_metadata"))
//                     .unwrap();
//                 break;
//             }
//         }
//     }
//
//     // 向对方发送握手的 LTEP 请求
//     let mut request = LinkedHashMap::new();
//     request.insert("p", 8080.to_box() as Box<dyn BEncoder>);
//     request.insert("v", "qBittorrent/5.0.46".to_box() as Box<dyn BEncoder>);
//     request.insert("yourip", "192.168.2.242".to_box() as Box<dyn BEncoder>);
//     request.insert("reqq", 500.to_box() as Box<dyn BEncoder>);
//     request.insert(
//         "m",
//         hashmap!(
//             "ut_metadata" => 2.to_box() as Box<dyn BEncoder>
//         )
//         .to_box() as Box<dyn BEncoder>,
//     );
//     let request = request.encode();
//     let mut bytes = Vec::with_capacity(request.len() + 2);
//     WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 2 + request.len() as u32).unwrap();
//     WriteBytesExt::write_u8(&mut bytes, MsgType::LTEPHandshake as u8).unwrap();
//     WriteBytesExt::write_u8(&mut bytes, 0).unwrap();
//     std::io::Write::write(&mut bytes, &request[..]).unwrap();
//     write.write_all(&bytes).await.unwrap();
//
//     let block_size = 16 * 1024;
//     let mut metadata = Vec::with_capacity(block_size as usize);
//     if let (Some(ut_metadata), Some(metadata_size)) = (ut_metadata, metadata_size) {
//         let block_num = (metadata_size + block_size - 1) / block_size;
//         println!("开始请求 metadata");
//         for i in 0..block_num {
//             let mut request = LinkedHashMap::new();
//             request.insert("msg_type", 0.to_box() as Box<dyn BEncoder>);
//             request.insert("piece", (i as u32).to_box() as Box<dyn BEncoder>);
//             let request = request.encode();
//
//             println!(
//                 "字符串: {:?}\tlen: {}",
//                 String::from_utf8(request.clone().to_vec()),
//                 request.len()
//             );
//             let mut bytes = Vec::with_capacity(request.len() + 2);
//             WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 2 + request.len() as u32).unwrap();
//             WriteBytesExt::write_u8(&mut bytes, MsgType::LTEPHandshake as u8).unwrap();
//             WriteBytesExt::write_u8(&mut bytes, ut_metadata as u8).unwrap();
//             std::io::Write::write(&mut bytes, &request[..]).unwrap();
//             write.write_all(&bytes).await.unwrap();
//             println!("请求数据: {:?}\tlen: {}", bytes, bytes.len());
//
//             // 接收数据
//             loop {
//                 let resp_type = PeerResp::new(&mut read, &peer).await;
//                 if let RespType::Continue(msg_type, mut bytes) = resp_type {
//                     if msg_type == MsgType::Hashes {
//                         println!("收到了 hashes\n{:?}", bytes);
//                         return Ok(());
//                     } else if msg_type == MsgType::HashReject {
//                         panic!("Hash reject")
//                     } else if msg_type == MsgType::LTEPHandshake {
//                         if bytes[0] != ut_metadata as u8 {
//                             continue;
//                         }
//                         let mut bytes = bytes.split_off(1);
//                         let res = bencoding::decode(bytes.clone()).unwrap();
//                         println!("res: {:?}", res);
//                         let piece_size = res.as_dict().unwrap().get_int("total_size").unwrap();
//                         let b = bytes.split_off(bytes.len() - piece_size as usize);
//                         metadata.append(&mut b.to_vec());
//                         break;
//                     } else {
//                         println!("收到了其他消息\tmsg_type: {:?}", msg_type);
//                     }
//                 } else {
//                     panic!("什么错误消息！");
//                 }
//             }
//         }
//     }
//
//     // 校验获取到的 metadata 是否正确
//     let mut hasher = Sha1::new();
//     Digest::update(&mut hasher, &metadata);
//     let metadata_hash = hex::encode(hasher.finalize().as_slice());
//     println!("metadata hash: {}", metadata_hash);
//     println!("info hash: {}", info_hash_str);
//     assert_eq!(metadata_hash, info_hash_str, "Invalid metadata hash");
//
//     let metadata = bencoding::decode(Bytes::from(metadata)).unwrap();
//     println!("{:?}", metadata);
//
//     Ok(())
// }
//
// /// 解析 ltep 字符串
// #[test]
// fn test_parse_ltep_str() {
//     let str = b"d12:complete_agoi1886e1:md11:lt_donthavei7e10:share_modei8e11:upload_onlyi3e12:ut_holepunchi4e11:ut_metadatai2e6:ut_pexi1ee13:metadata_sizei2837e4:reqqi500e11:upload_onlyi1e1:v17:qBittorrent/5.0.46:yourip4:\xc0\xa8\x02\xcbe";
//     let res = bencoding::decode(Bytes::from_owner(str)).unwrap();
//     println!("res: {:?}", res);
//     let res = res.as_dict().unwrap();
//     let metadata_size = res.get_int("metadata_size").unwrap();
//     println!("metadata_size: {}", metadata_size);
// }

// ===========================================================================
// DHT
// ===========================================================================
/// 循环 id
struct CycleId {
    val: AtomicU16,
}

impl CycleId {
    fn new() -> Self {
        Self {
            val: AtomicU16::new(0),
        }
    }

    fn next_tran_id(&mut self) -> u16 {
        self.val.fetch_add(1, Ordering::Relaxed)
    }
}

#[test]
fn test_cycle_id() {
    let mut cycle_id = CycleId::new();
    for _ in 0..u16::MAX as usize + 10 {
        info!("next id: {}", cycle_id.next_tran_id());
    }
}

// static NODE_ID: &[u8; 20] = &[0xd9, 0x82, 0x12, 0xf0, 0xb1, 0xdb, 0x7, 0x46, 0x1a, 0x7e, 0x1f, 0x96, 0x50, 0x66, 0x42, 0x51, 0xe7, 0xe1, 0x7d, 0x8d];
static NODE_ID: &[u8; 20] = b"adkoqwei123jk3341ks0";

// 生成有效的 DHT 节点 ID
fn generate_node_id() -> [u8; 20] {
    let mut rng = rand::rng();
    let mut id = [0u8; 20];
    rng.fill(&mut id);

    // 确保 ID 不为全零（常见验证要求）
    if id.iter().all(|&b| b == 0) {
        id[0] = 1; // 设置非零值
    }
    id
}

/// 从 DHT 中获取 peer
#[tokio::test]
async fn test_peer_from_dht() {
    let mut cycle_id = CycleId::new();
    let _node_id = generate_node_id();

    let mut queue = VecDeque::new();
    // let addr = "192.168.2.242:3115".parse::<SocketAddr>().unwrap();
    // let addr = "123.156.68.196:20252".parse::<SocketAddr>().unwrap();
    let addr = "37.48.64.31:28015".parse::<SocketAddr>().unwrap();

    queue.push_back(addr);
    let info_hash = hex::decode("c6bbdb50bd685bacf8c0d615bb58a3a0023986ef").unwrap();

    let node_id = NodeId::from(*NODE_ID);
    let info_hash = NodeId::try_from(info_hash.as_slice()).unwrap();

    let mut min_dist = node_id.distance(&info_hash);
    let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    info!("本地监听端口: {:?}", addr);
    let mut peers = vec![];

    while !queue.is_empty() && peers.len() < 10 {
        let addr = queue.pop_front().unwrap();
        info!("尝试对{}node的访问", addr);
        let ping = DHTBase::<Ping>::request(
            Ping::new(node_id.to_value_bytes()),
            "ping".to_string(),
            cycle_id.next_tran_id(),
        );
        let data = ping.to_bencode().unwrap();

        info!("向[{}]发送一个ping", addr);
        let res = tokio::time::timeout(Duration::from_secs(10), socket.send_to(&data, addr)).await;
        if res.is_err() {
            error!("访问超时");
            continue;
        }
        let res = res.unwrap();
        if res.is_err() {
            error!("拒绝了ping");
            continue;
        }

        info!("从{}那里读取ping的数据", addr);
        let mut buff = ByteBuffer::new(5120);
        let res =
            tokio::time::timeout(Duration::from_secs(10), socket.recv_from(buff.as_mut())).await;
        if res.is_err() {
            error!("读取ping失败超时");
            continue;
        }
        let res = res.unwrap();
        if res.is_err() {
            error!("读取ping失败");
            continue;
        }

        let (n, _) = res.unwrap();
        buff.resize(n);
        let resp = DHTBase::<Ping>::from_bencode(buff.as_ref());
        if resp.is_err() {
            error!("ping 的响应结果不符合预期\tping: {:?}", resp);
            continue;
        }
        let resp = resp.unwrap();
        if resp.r.is_none() {
            error!("ping 的响应没有 r 字段: {:?}", resp);
            continue;
        }
        let id = if let Value::Bytes(id) = &resp.r.as_ref().unwrap().id {
            hex::encode(id.as_ref())
        } else {
            String::new()
        };
        info!(
            "这个[{}]老表罗觉，可以连通\nresp: {:?}\nnode id: {}",
            addr, resp, id
        );

        // 告诉我这个种子的peers
        info!("查询peers");

        let get_peers = DHTBase::<GetPeersReq>::request(
            GetPeersReq::new(
                node_id.to_value_bytes(),
                info_hash.to_value_bytes(),
            ),
            "get_peers".to_string(),
            cycle_id.next_tran_id(),
        );

        let data = get_peers.to_bencode().unwrap();

        if socket.send_to(&data, addr).await.is_err() {
            continue;
        }

        if socket.readable().await.is_err() {
            continue;
        }

        let mut buff = ByteBuffer::new(5120);

        let res = tokio::time::timeout(
            Duration::from_secs(10),
            socket.recv_from(&mut buff.as_mut()),
        )
        .await;
        if res.is_err() {
            error!("读取 peers 响应超时");
            continue;
        }
        let res = res.unwrap();
        if res.is_err() {
            error!("读取 peers 响应错误");
            continue;
        }

        let (n, _) = res.unwrap();
        buff.resize(n);

        let resp = DHTBase::<GetPeersResp>::from_bencode(buff.as_ref());
        if resp.is_err() {
            error!("get_peers 的响应结果不符合预期\tget_peers: {:?}", resp);
            continue;
        }

        let resp = resp.unwrap();
        info!("resp: {:?}", resp);

        if let Some(r) = resp.r {
            if let Some(nodes) = r.nodes {
                let mut min = min_dist.clone();
                for node in nodes {
                    let dist = node.id.distance(&info_hash);
                    if min_dist >= dist {
                        queue.push_back(node.addr.into());
                        min = min.min(dist);
                    }
                }
                min_dist = min;
            }

            if let Some(mut values) = r.values {
                info!("===================================================");
                info!("peers: {:?}", values);
                info!("===================================================");
                peers.append(&mut values);
            }
        }

        info!("queue len: {}", queue.len());
    }

    info!("peers: {:?}", peers);
}
