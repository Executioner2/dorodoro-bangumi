//! DHT 测试

use doro_util::default_logger;
use rand::Rng;
use rand::prelude::IteratorRandom;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tracing::Level;

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

// 定义常量
const K_BUCKET_SIZE: usize = 8; // Kademlia K 值
const REFRESH_INTERVAL: Duration = Duration::from_secs(900); // 桶刷新间隔

// 节点 ID 类型 (160 位)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId([u8; 20]);

impl NodeId {
    /// 生成随机 Node ID
    pub fn random() -> Self {
        let mut bytes = [0u8; 20];
        rand::rng().fill(&mut bytes);
        NodeId(bytes)
    }

    /// 计算与另一个节点的异或距离
    pub fn distance(&self, other: &NodeId) -> [u8; 20] {
        let mut res = [0u8; 20];
        #[allow(clippy::needless_range_loop)]
        for i in 0..20 {
            res[i] = self.0[i] ^ other.0[i];
        }
        res
    }

    /// 获取指定比特位的值 (0 或 1)
    pub fn bit(&self, index: usize) -> u8 {
        let byte_index = index / 8;
        let bit_index = 7 - (index % 8); // 最高位为 bit0
        (self.0[byte_index] >> bit_index) & 1
    }
}

// DHT 节点结构
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub addr: SocketAddr,
    pub last_seen: Instant,
    pub last_responded: bool,
}

impl Node {
    pub fn new(id: NodeId, addr: SocketAddr) -> Self {
        Self {
            id,
            addr,
            last_seen: Instant::now(),
            last_responded: true,
        }
    }

    /// 更新最后可见时间
    pub fn touch(&mut self) {
        self.last_seen = Instant::now();
        self.last_responded = true;
    }
}

// Kademlia 桶结构
#[derive(Debug, Clone)]
struct KBucket {
    prefix: NodeId,        // 桶的前缀
    prefix_len: usize,     // 前缀长度（位）
    nodes: VecDeque<Node>, // 使用双端队列以便于移动节点
    last_accessed: Instant,
    can_split: bool, // 桶是否可以分裂（包含自身节点）
}

impl KBucket {
    fn new(prefix: NodeId, prefix_len: usize, can_split: bool) -> Self {
        Self {
            prefix,
            prefix_len,
            nodes: VecDeque::new(),
            last_accessed: Instant::now(),
            can_split,
        }
    }

    /// 检查节点是否属于此桶
    pub fn contains(&self, node_id: &NodeId) -> bool {
        // 检查前 prefix_len 位是否匹配
        for i in 0..self.prefix_len {
            if self.prefix.bit(i) != node_id.bit(i) {
                return false;
            }
        }
        true
    }

    /// 检查桶是否已满
    fn is_full(&self) -> bool {
        self.nodes.len() >= K_BUCKET_SIZE
    }

    /// 尝试将节点添加到桶中
    fn add_node(&mut self, node: Node) -> (AddResult, Option<Node>) {
        // 检查节点是否已存在
        if let Some(pos) = self.nodes.iter().position(|n| n.id == node.id) {
            // 更新现有节点
            self.nodes[pos] = node;
            self.move_to_end(pos);
            return (AddResult::Updated, None);
        }

        // 如果桶未满，直接添加
        if !self.is_full() {
            self.nodes.push_back(node);
            self.last_accessed = Instant::now();
            return (AddResult::Added, None);
        }

        // 桶已满时检查是否有不响应节点
        if let Some(pos) = self.find_stale_node() {
            self.nodes.remove(pos);
            self.nodes.push_back(node);
            self.last_accessed = Instant::now();
            return (AddResult::ReplacedStale, None);
        }

        (AddResult::BucketFull, Some(node))
    }

    /// 查找可能失效的节点
    fn find_stale_node(&self) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .find(|(_, n)| !n.last_responded && n.last_seen.elapsed() > Duration::from_secs(3600))
            .map(|(i, _)| i)
    }

    /// 将节点移动到桶的末尾（表示最近使用）
    fn move_to_end(&mut self, index: usize) {
        if let Some(node) = self.nodes.remove(index) {
            self.nodes.push_back(node);
        }
    }

    /// 随机选择一个节点用于刷新
    fn random_node(&self) -> Option<&Node> {
        self.nodes.iter().choose(&mut rand::rng())
    }

    /// 分裂桶为两个新桶
    fn split(&self) -> (KBucket, KBucket) {
        let mut prefix0 = self.prefix;
        let mut prefix1 = self.prefix;

        // 设置新前缀的第 prefix_len 位
        let byte_index = self.prefix_len / 8;
        let bit_index = 7 - (self.prefix_len % 8);
        prefix0.0[byte_index] &= !(1 << bit_index); // 设置为0
        prefix1.0[byte_index] |= 1 << bit_index; // 设置为1

        let new_prefix_len = self.prefix_len + 1;

        // 创建新桶 - 只有包含自身节点的桶才能继续分裂
        let can_split0 = self.can_split;
        let can_split1 = self.can_split;

        (
            KBucket::new(prefix0, new_prefix_len, can_split0),
            KBucket::new(prefix1, new_prefix_len, can_split1),
        )
    }
}

// 节点添加结果
#[derive(Debug, PartialEq)]
enum AddResult {
    Added,
    Updated,
    ReplacedStale,
    BucketFull,
}

// DHT 路由表
#[derive(Debug)]
pub struct RoutingTable {
    own_id: NodeId,
    buckets: Vec<KBucket>,
}

impl RoutingTable {
    pub fn new(own_id: NodeId) -> Self {
        // 初始状态：单个桶覆盖整个ID空间
        let bucket = KBucket::new(NodeId([0; 20]), 0, true);

        Self {
            own_id,
            buckets: vec![bucket],
        }
    }

    /// 查找节点所属的桶
    fn find_bucket_for_node(&self, node_id: &NodeId) -> Option<usize> {
        self.buckets.iter().position(|b| b.contains(node_id))
    }

    /// 查找节点所属的桶（或创建新桶）
    fn find_or_create_bucket(&mut self, node_id: &NodeId) -> usize {
        if let Some(index) = self.find_bucket_for_node(node_id) {
            return index;
        }

        // 需要创建新桶（由于分裂）
        // 实际实现中这种情况不应该发生
        panic!("No bucket found for node - routing table inconsistency!");
    }

    /// 添加节点到路由表
    pub fn add_node(&mut self, node: Node) -> bool {
        // 跳过自身节点
        if node.id == self.own_id {
            return false;
        }

        // 找到节点所属桶
        let bucket_index = self.find_or_create_bucket(&node.id);
        let (result, node) = self.buckets[bucket_index].add_node(node);

        // 处理桶满的情况（如果可以分裂）
        if result == AddResult::BucketFull {
            let bucket = &self.buckets[bucket_index];

            if bucket.can_split {
                // 执行桶分裂
                self.split_bucket(bucket_index);

                // 重新尝试添加节点
                let node = node.unwrap();
                let new_bucket_index = self.find_or_create_bucket(&node.id);
                return self.buckets[new_bucket_index].add_node(node).0 != AddResult::BucketFull;
            } else {
                // 对于不包含自身节点的桶，只替换失效节点
                // 这里我们不做特殊处理，因为 add_node 已经尝试过替换失效节点
            }
        }

        result != AddResult::BucketFull
    }

    /// 分裂指定的桶
    fn split_bucket(&mut self, index: usize) {
        let bucket = self.buckets.remove(index);
        let (mut bucket0, mut bucket1) = bucket.split();

        // 重新分配节点
        for node in bucket.nodes {
            if bucket0.contains(&node.id) {
                bucket0.add_node(node);
            } else {
                bucket1.add_node(node);
            }
        }

        // 更新桶的分裂能力（只有包含自身节点的桶才能分裂）
        bucket0.can_split = bucket0.contains(&self.own_id);
        bucket1.can_split = bucket1.contains(&self.own_id);

        // 添加新桶
        self.buckets.push(bucket0);
        self.buckets.push(bucket1);
    }

    /// 查找最近的邻居节点
    pub fn find_closest_nodes(&self, target_id: &NodeId, count: usize) -> Vec<Node> {
        // 收集所有节点
        let mut nodes: Vec<Node> = self
            .buckets
            .iter()
            .flat_map(|bucket| bucket.nodes.iter().cloned())
            .collect();

        // 按距离排序（使用实际的异或距离计算）
        nodes.sort_by_cached_key(|node| node.id.distance(target_id));

        // 取最近的count个节点
        nodes.truncate(count);
        nodes
    }

    /// 刷新路由表（定期调用）
    pub fn refresh(&mut self) {
        for bucket in &mut self.buckets {
            // 只刷新长时间未访问的桶
            if bucket.last_accessed.elapsed() > REFRESH_INTERVAL {
                if let Some(node) = bucket.random_node() {
                    // 在实际应用中，这里会发送PING请求
                    println!(
                        "Refreshing bucket with prefix len {} with node {}",
                        bucket.prefix_len, node.addr
                    );
                }
                bucket.last_accessed = Instant::now();
            }
        }
    }

    /// 标记节点为响应状态
    pub fn mark_node_responded(&mut self, node_id: &NodeId) {
        if let Some(index) = self.find_bucket_for_node(node_id) {
            if let Some(pos) = self.buckets[index]
                .nodes
                .iter()
                .position(|n| n.id == *node_id)
            {
                self.buckets[index].nodes[pos].touch();
                self.buckets[index].move_to_end(pos);
            }
        }
    }

    /// 标记节点为未响应状态
    pub fn mark_node_unresponsive(&mut self, node_id: &NodeId) {
        if let Some(index) = self.find_bucket_for_node(node_id) {
            if let Some(pos) = self.buckets[index]
                .nodes
                .iter()
                .position(|n| n.id == *node_id)
            {
                self.buckets[index].nodes[pos].last_responded = false;
            }
        }
    }

    /// 打印路由表状态（用于调试）
    pub fn print_state(&self) {
        println!("Routing Table ({} buckets):", self.buckets.len());
        for (i, bucket) in self.buckets.iter().enumerate() {
            println!(
                "Bucket {}: prefix_len={}, nodes={}, can_split={}",
                i,
                bucket.prefix_len,
                bucket.nodes.len(),
                bucket.can_split
            );
        }
    }
}
