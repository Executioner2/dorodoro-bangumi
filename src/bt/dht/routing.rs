#[cfg(test)]
mod tests;

use crate::{datetime, util};
use alloc::borrow::Cow;
use anyhow::{Error, anyhow};
use bincode::{Decode, Encode};
use core::fmt::Formatter;
use rand::{Rng, RngCore};
use sha1::{Digest, Sha1};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::net::SocketAddr;
use std::ops::{Range, RangeFrom};
use std::time::Duration;
use bendy::decoding::{FromBencode, Object};
use bendy::encoding::{SingleItemEncoder, ToBencode};

/// Kademlia K 值
const K_BUCKET_SIZE: usize = 8;

/// 桶刷新间隔
// pub const REFRESH_INTERVAL: Duration = Duration::from_secs(900);
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Eq, PartialEq, Encode, Decode, Default)]
pub struct NodeId([u8; 20]);

impl NodeId {
    pub fn new(id: [u8; 20]) -> Self {
        Self(id)
    }

    pub fn random() -> Self {
        let mut bytes = [0u8; 20];
        rand::rng().fill_bytes(&mut bytes);
        let mut hasher = Sha1::new();
        hasher.update(&bytes);
        NodeId(hasher.finalize().into())
    }

    pub fn distance(&self, other: &Self) -> [u8; 20] {
        let mut res = [0u8; 20];
        for i in 0..20 {
            res[i] = self[i] ^ other[i];
        }
        res
    }

    pub fn cow(&self) -> Cow<Self> {
        Cow::Borrowed(self)
    }

    pub fn bit(&self, index: usize) -> u8 {
        let (index, offset) = util::bytes_util::bitmap_offset(index);
        self[index] & offset
    }
}

impl TryFrom<&[u8]> for NodeId {
    type Error = Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != 20 {
            return Err(anyhow!("Invalid node ID length: {}", value.len()));
        }
        Ok(Self::new(value.try_into()?))
    }
}

impl From<[u8; 20]> for NodeId {
    fn from(value: [u8; 20]) -> Self {
        Self::new(value)
    }
}

impl std::ops::Index<usize> for NodeId {
    type Output = u8;
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl std::ops::IndexMut<usize> for NodeId {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

impl std::ops::Index<Range<usize>> for NodeId {
    type Output = [u8];

    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.0[range]
    }
}

impl std::ops::IndexMut<Range<usize>> for NodeId {
    fn index_mut(&mut self, range: Range<usize>) -> &mut Self::Output {
        &mut self.0[range]
    }
}

impl std::ops::Index<RangeFrom<usize>> for NodeId {
    type Output = [u8];

    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        &self.0[range]
    }
}

impl std::ops::IndexMut<RangeFrom<usize>> for NodeId {
    fn index_mut(&mut self, range: RangeFrom<usize>) -> &mut Self::Output {
        &mut self.0[range]
    }
}

impl std::fmt::Debug for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeId({})", hex::encode(&self.0))
    }
}

impl std::str::FromStr for NodeId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|e| anyhow!("Invalid hex string: {}", e))?;
        if bytes.len() == 20 {
            Ok(Self(bytes.try_into().unwrap()))
        } else {
            Err(anyhow!("Invalid node ID length: {}", bytes.len()))
        }
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

impl FromBencode for NodeId {
    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error>
    where
        Self: Sized
    {
        let bytes = object.try_into_bytes()?;
        match NodeId::try_from(bytes){
            Ok(node_id) => Ok(node_id),
            Err(_) => Err(bendy::decoding::Error::unexpected_token(
                "expected 20-byte node ID",
                format!("Invalid id length: {}", bytes.len())
            )),
        }
    }
}

impl ToBencode for NodeId {
    const MAX_DEPTH: usize = 10;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), bendy::encoding::Error> {
        encoder.emit_bytes(&self.0)
    }
}

#[derive(Eq, PartialEq, Clone, Decode, Encode, Debug)]
pub struct Node {
    id: NodeId,
    addr: SocketAddr,
    last_seen: Duration,
    responsed: bool,
}

impl Node {
    pub fn new(id: NodeId, addr: SocketAddr) -> Self {
        Self {
            id,
            addr,
            last_seen: now_secs(),
            responsed: true, // 创建 node 的时候，这个 node 是响应了的
        }
    }

    pub fn touch(&mut self) {
        self.last_seen = now_secs();
        self.responsed = true;
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    
    pub fn take_id(self) -> NodeId {
        self.id
    }
}

#[derive(Eq, PartialEq)]
enum AddResult {
    Added,
    Updated,
    ReplacedStale,
    BucketFull(Node),
}

impl AddResult {
    fn is_success(&self) -> bool {
        match self {
            AddResult::BucketFull(_) => false,
            _ => true,
        }
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct Bucket {
    prefix: NodeId,
    prefix_len: usize,
    nodes: VecDeque<Node>,
    lastchange: Duration,
    can_split: bool,
}

impl Bucket {
    pub fn new(prefix: NodeId, prefix_len: usize, can_split: bool) -> Self {
        Self {
            prefix,
            prefix_len,
            nodes: VecDeque::new(),
            lastchange: now_secs(),
            can_split,
        }
    }
    
    pub fn get_prefix(&self) -> &NodeId {
        &self.prefix
    }
    
    pub fn get_prefix_len(&self) -> usize {
        self.prefix_len
    }
    
    pub fn update_lastchange(&mut self) {
        self.lastchange = now_secs();
    }

    pub fn contains(&self, node_id: &NodeId) -> bool {
        for i in 0..self.prefix_len {
            if self.prefix.bit(i) != node_id.bit(i) {
                return false;
            }
        }
        true
    }

    fn add_node(&mut self, node: Node) -> AddResult {
        if let Some(pos) = self.nodes.iter().position(|n| n.id == node.id) {
            self.nodes[pos] = node;
            self.move_to_end(pos);
            self.lastchange = now_secs();
            return AddResult::Updated;
        }

        if !self.is_full() {
            self.nodes.push_back(node);
            self.lastchange = now_secs();
            return AddResult::Added;
        }

        if let Some(pos) = self.pop_stale_node() {
            self.nodes.remove(pos);
            self.nodes.push_back(node);
            self.lastchange = now_secs();
            return AddResult::ReplacedStale;
        }

        AddResult::BucketFull(node)
    }

    pub fn random_node(&self) -> Option<(NodeId, SocketAddr)> {
        self.nodes.iter().last().map(|node| {
            let mut info_hash = self.prefix.clone();
            let (index, offset) = util::bytes_util::bitmap_offset(self.prefix_len);
            let mut rng = rand::rng();
            if index + 1 < 20 {
                rng.fill_bytes(&mut info_hash[index + 1..]);
            }
            info_hash[index] =
                (info_hash[index] & u8::MAX - (offset - 1)) | rng.random_range(..offset);
            (info_hash, node.addr)
        })
    }

    fn split(&self) -> (Bucket, Bucket) {
        let mut prefix0 = self.prefix.clone();
        let mut prefix1 = self.prefix.clone();

        let new_prefix_len = self.prefix_len + 1;
        let (index, offset) = util::bytes_util::bitmap_offset(self.prefix_len);
        prefix0[index] &= !offset; // 设置为0
        prefix1[index] |= offset; // 设置为1

        (
            Bucket::new(prefix0, new_prefix_len, self.can_split),
            Bucket::new(prefix1, new_prefix_len, self.can_split),
        )
    }

    fn move_to_end(&mut self, pos: usize) {
        if let Some(node) = self.nodes.remove(pos) {
            self.nodes.push_back(node);
        }
    }

    fn pop_stale_node(&self) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .find(|(_, n)| {
                !n.responsed && now_secs().saturating_sub(n.last_seen) > REFRESH_INTERVAL
            })
            .map(|(i, _)| i)
    }

    fn is_full(&self) -> bool {
        self.nodes.len() >= K_BUCKET_SIZE
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct RoutingTable {
    own_id: NodeId,
    buckets: Vec<Bucket>,
}

impl RoutingTable {
    pub fn new(own_id: NodeId) -> Self {
        let bucket = Bucket::new(NodeId([0; 20]), 0, true);
        Self {
            own_id,
            buckets: vec![bucket],
        }
    }

    fn find_bucket_for_node(&self, node_id: &NodeId) -> Option<usize> {
        self.buckets.iter().position(|b| b.contains(node_id))
    }

    fn find_or_create_bucket(&self, node_id: &NodeId) -> usize {
        if let Some(index) = self.find_bucket_for_node(node_id) {
            return index;
        }

        panic!("No bucket found for node - routing table inconsistency!");
    }

    fn split_bucket(&mut self, index: usize) {
        let (mut bucket0, mut bucket1) = self.buckets[index].split();
        let bucket = self.buckets.remove(index);

        // 重新分配节点
        for node in bucket.nodes {
            if bucket0.contains(&node.id) {
                bucket0.add_node(node);
            } else {
                bucket1.add_node(node);
            }
        }

        bucket0.can_split = bucket0.contains(&self.own_id);
        bucket1.can_split = bucket1.contains(&self.own_id);

        self.buckets.push(bucket0);
        self.buckets.push(bucket1);
    }

    pub fn add_node(&mut self, node: Node) -> bool {
        // 跳过自身节点
        if node.id == self.own_id {
            return false;
        }

        let mut bucket_index = self.find_or_create_bucket(&node.id);
        let result = self.buckets[bucket_index].add_node(node);
        let mut is_success = result.is_success();

        if let AddResult::BucketFull(node) = result {
            // 分裂桶
            let bucket = &self.buckets[bucket_index];
            if bucket.can_split {
                self.split_bucket(bucket_index);
                bucket_index = self.find_or_create_bucket(&node.id);
                is_success = self.buckets[bucket_index].add_node(node).is_success();
            } else {
                // 这里什么都不做，桶满了，但是不能分裂。我们尝试过更新
                // 节点和替换节点，但是此node都不满足更新节点和替换节点
                // 的条件。
            }
        }

        if is_success {
            // 更新桶的 lastchange 时间
            self.buckets[bucket_index].update_lastchange();
        }

        is_success
    }

    pub fn find_closest_nodes(&self, target_id: &NodeId, count: usize) -> Vec<Node> {
        if count == 0 {
            return Vec::new();
        }

        // 使用最大堆来维护最近的 count 个节点
        let mut heap = BinaryHeap::with_capacity(count);

        for bucket in &self.buckets {
            for node in &bucket.nodes {
                let distance = node.id.distance(target_id);
                heap.push(NodeWithDistance { node, distance });
                if heap.len() > count {
                    heap.pop();
                }
            }
        }

        heap.into_sorted_vec()
            .iter()
            .map(|nwd| nwd.node.clone())
            .collect()
    }

    pub fn next_refresh_bucket(&mut self) -> Option<&mut Bucket> {
        self.buckets
            .iter_mut()
            .find(|bucket| now_secs().saturating_sub(bucket.lastchange) > REFRESH_INTERVAL)
            
    }

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

    pub fn mark_node_unresponsive(&mut self, node_id: &NodeId) {
        if let Some(index) = self.find_bucket_for_node(node_id) {
            if let Some(pos) = self.buckets[index]
                .nodes
                .iter()
                .position(|n| n.id == *node_id)
            {
                self.buckets[index].nodes[pos].responsed = false;
            }
        }
    }
    
    pub fn get_own_id(&self) -> &NodeId {
        &self.own_id
    }
    
    pub fn get_node_num(&self) -> usize {
        self.buckets.iter().map(|b| b.nodes.len()).sum()
    }
}

/// 包装结构，用于节点和距离的比较
#[derive(Eq, PartialEq)]
struct NodeWithDistance<'a> {
    node: &'a Node,
    distance: [u8; 20],
}

impl<'a> Ord for NodeWithDistance<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        // 比较距离（使用 XOR 距离的字典序比较）
        self.distance
            .cmp(&other.distance)
            // 如果距离相等，使用节点 ID 作为次要键
            .then_with(|| self.node.id.0.cmp(&other.node.id.0))
    }
}

impl<'a> PartialOrd for NodeWithDistance<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn now_secs() -> Duration {
    Duration::from_secs(datetime::now_secs())
}
