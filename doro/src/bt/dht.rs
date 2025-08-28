//! dht 实现

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use bendy::decoding::FromBencode;
use bendy::encoding::ToBencode;
use doro_util::bendy_ext::SocketAddrExt;
use doro_util::global::Id;
use doro_util::sync::MutexExt;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::Receiver;
use tracing::{debug, error, info, trace};

use crate::config::{DHT_EXPECT_PEERS, DHT_FIND_PEERS_INTERVAL, DHT_WAIT_PEER_LIMIT};
use crate::context::Context;
use crate::dht::entity::{DHTBase, GetPeersReq, GetPeersResp, Host, Ping};
use crate::dht::routing::{Node, NodeId, REFRESH_INTERVAL, RoutingTable};
use crate::mapper::dht::DHTMapper;
use crate::task::{HostSource, ReceiveHost};
use crate::udp_server::UdpServer;

pub mod entity;
pub mod routing;

#[cfg(test)]
mod tests;

/// 超时时间
const TIMEOUT: Duration = Duration::from_secs(10);

/// 期望的就近节点数量
const EXPECT_CLOSEST_NODE_NUM: usize = 30;

/// 并发进行的 get_peers 任务数量
const GET_PEERS_CONCURRENCY: usize = 10;

type RoutingTableAM = Arc<Mutex<RoutingTable>>;

type DHTRequestA = Arc<DHTRequest>;

struct DHTRequest {
    /// 请求的节点 ID
    own_id: Arc<NodeId>,

    /// dht 服务器
    udp_server: UdpServer,
}

impl DHTRequest {
    pub fn new(own_id: Arc<NodeId>, udp_server: UdpServer) -> Self {
        Self { own_id, udp_server }
    }

    /// 发送一个 ping 操作
    async fn ping(&self, addr: &SocketAddr) -> Option<DHTBase<'_, Ping<'_>>> {
        debug!("开始尝试 ping");
        let t = self.udp_server.tran_id();
        let ping = DHTBase::<Ping>::request(Ping::new(self.own_id.cow()), "ping".to_string(), t);
        let data = ping.to_bencode().unwrap();
        let response = self.udp_server.request(t, &data, addr).await;
        if response.is_err() {
            debug!(
                "执行 ping，获取 Response Future 错误: {:?}",
                response.unwrap_err()
            );
            return None;
        }
        match tokio::time::timeout(TIMEOUT, response.unwrap()).await {
            Ok((data, _addr)) => {
                let resp = DHTBase::<Ping>::from_bencode(data.as_ref());
                resp.ok()
            }
            Err(e) => {
                debug!("ping request error: {}", e);
                None
            }
        }
    }

    /// 发送一个 get_peers 操作
    async fn get_peers(
        &self, addr: &SocketAddr, info_hash: &NodeId,
    ) -> (Vec<Host>, Vec<SocketAddrExt>) {
        debug!("开始尝试 get_peers");
        let t = self.udp_server.tran_id();
        let get_peers = DHTBase::<GetPeersReq>::request(
            GetPeersReq::new(self.own_id.cow(), info_hash.cow()),
            "get_peers".to_string(),
            t,
        );

        let default = (Vec::default(), Vec::default());
        let data = get_peers.to_bencode().unwrap();
        let response = self.udp_server.request(t, &data, addr).await;
        if response.is_err() {
            debug!(
                "执行 get_peers，获取 Response Future 错误: {:?}",
                response.unwrap_err()
            );
            return default;
        }

        match tokio::time::timeout(TIMEOUT, response.unwrap()).await {
            Ok((data, _addr)) => {
                let resp = DHTBase::<GetPeersResp>::from_bencode(data.as_ref());
                if let Ok(resp) = resp {
                    if let Some(r) = resp.r {
                        return (r.nodes.unwrap_or_default(), r.values.unwrap_or_default());
                    }
                } else {
                    error!("get_peers 解析失败");
                }
            }
            Err(e) => {
                debug!("get_peers request error: {}", e);
            }
        }

        default
    }
}

pub struct DHTInner {
    /// 自己的节点 ID
    own_id: Arc<NodeId>,

    /// dht 请求
    dht_request: DHTRequestA,

    /// 路由表
    routing_table: RoutingTableAM,

    /// 启动节点
    bootstrap_nodes: Vec<String>,

    /// 并行查询 get_peers 任务的信号量
    gpcs: Arc<Semaphore>,
}

static DHT_INSTANCE: OnceLock<DHT> = OnceLock::new();

#[derive(Clone)]
pub struct DHT(Arc<DHTInner>);

impl Deref for DHT {
    type Target = Arc<DHTInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DHT {
    pub fn init(
        udp_server: UdpServer, routing_table: RoutingTable, bootstrap_nodes: Vec<String>,
    ) -> &'static Self {
        let own_id = Arc::new(routing_table.get_own_id().clone());
        let dht_request = DHTRequest::new(own_id.clone(), udp_server);
        let dht = Self(Arc::new(DHTInner {
            own_id,
            dht_request: Arc::new(dht_request),
            routing_table: Arc::new(Mutex::new(routing_table)),
            bootstrap_nodes,
            gpcs: Arc::new(Semaphore::new(GET_PEERS_CONCURRENCY)),
        }));
        DHT_INSTANCE.get_or_init(|| dht)
    }

    pub fn global() -> &'static Self {
        DHT_INSTANCE.get().unwrap()
    }

    async fn domain_resolve(&self, domain: &str) -> Option<SocketAddr> {
        tokio::net::lookup_host(domain).await.ok()?.next()
    }

    pub fn min_dist(&self, info_hash: &NodeId) -> Arc<[u8; 20]> {
        Arc::new(self.own_id.distance(info_hash))
    }

    /// 刷新路由表
    async fn refresh_routing_table(&self) {
        let routing_table = self.routing_table.clone();
        let dht_request = self.dht_request.clone();
        let conn = Context::get_conn().await.unwrap();
        tokio::spawn(Box::pin(async move {
            debug!("开始刷新 dht 路由表");
            loop {
                let mut update_traget = None;
                {
                    let mut routing_table = routing_table.lock_pe();
                    if let Some(bucket) = routing_table.next_refresh_bucket() {
                        trace!(
                            "获取到要刷新的桶: {:?}\tprefix len: {}\t可用的节点: {}",
                            bucket.get_prefix(),
                            bucket.get_prefix_len(),
                            bucket.get_node_len()
                        );
                        bucket.update_lastchange(); // 即便没有节点，也要更新 bucket 的 lastchange
                        let target_id = bucket.random_node();
                        update_traget = routing_table
                            .find_closest_nodes(&target_id, 1)
                            .first()
                            .map(|node| (target_id, node.addr()))
                    }
                }

                if let Some((info_hash, addr)) = update_traget {
                    let (nodes, _) = dht_request.get_peers(&addr, &info_hash).await;
                    trace!(
                        "开始刷新路由表节点 [{info_hash}]\t获取到的节点数量: {}",
                        nodes.len()
                    );
                    let mut routing_table = routing_table.lock_pe();
                    for node in nodes {
                        routing_table.add_node(Node::new(node.id, node.addr.into()));
                    }
                } else {
                    trace!("没有需要刷新的桶");
                    break;
                }
            }
            // 更新路由表
            let routing_table = routing_table.lock_pe();
            debug!("更新 dht 路由表到本地");
            conn.update_routing_table(&routing_table).unwrap();
            let node_num = routing_table.get_node_num();
            debug!("刷新 dht 路由表结束\t当前已知节点数: {}", node_num);
        }));
    }

    /// 开始定时刷新路由表
    pub async fn start_interval_refresh(self) {
        let mut tick = tokio::time::interval(REFRESH_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = Context::global().cancelled() => {
                    break;
                }
                _ = tick.tick() => {
                    self.refresh_routing_table().await;
                }
            }
        }
    }
}

/// 检查队列中的 nodes 是否可用，并加入到路由表中
async fn check_add_node(
    rt: &RoutingTableAM, dr: &DHTRequestA, node_id: Option<NodeId>, addr: &SocketAddr,
) -> bool {
    let ping_resp = dr.ping(addr).await;
    if ping_resp.is_none() || ping_resp.as_ref().unwrap().r.is_none() {
        if let Some(node_id) = node_id {
            rt.lock_pe().mark_node_unresponsive(&node_id)
        }
        debug!("ping 不通[{}]", addr);
        return false;
    }

    // 能 ping 通，那么加入到路由表中
    let ping_resp = ping_resp.unwrap();
    let mut routing_table = rt.lock_pe();
    let node_id = ping_resp.r.unwrap().id.into_owned();
    routing_table.add_node(Node::new(node_id, *addr));
    true // 只要能 ping 通，就认为是可用节点（和有没有成功加入到路由表无关）
}

/// 异步寻找 peers
async fn async_find_peers<T>(
    node_id: Option<NodeId>, addr: SocketAddr, info_hash: Arc<NodeId>, receive_host: Weak<T>,
    min_dist: Arc<[u8; 20]>,
) -> Option<(VecDeque<(Option<NodeId>, SocketAddr)>, [u8; 20], usize)>
where
    T: ReceiveHost + Send + Sync + 'static,
{
    let rt: RoutingTableAM = DHT::global().routing_table.clone();
    let dr: DHTRequestA = DHT::global().dht_request.clone();

    debug!("向 [{addr}] 查询 peers");
    let mut queue = VecDeque::new();

    if !check_add_node(&rt, &dr, node_id, &addr).await {
        return None;
    }

    // 可以连通，那么开始广播查询 peers
    let (nodes, values) = dr.get_peers(&addr, &info_hash).await;
    let mut min = *min_dist.clone();
    for node in nodes {
        let dist = node.id.distance(&info_hash);
        if *min_dist >= dist {
            queue.push_back((Some(node.id), node.addr.into()));
            min = min.min(dist);
        }
    }

    if !values.is_empty() {
        trace!("新增了的peer: {:?}", values);
        let peers = values.iter().map(|p| p.addr).collect();
        if let Some(receive_host) = receive_host.upgrade() {
            receive_host.receive_hosts(peers, HostSource::DHT).await;
        }
    }

    Some((queue, min, values.len()))
}

/// 寻找 peers
#[rustfmt::skip]
pub async fn find_peers<T>(
    info_hash: NodeId,
    receive_host: Weak<T>,
    gasket_id: Id,
    expect_peers: usize,
) where
    T: ReceiveHost + Send + Sync + 'static,
{
    // 从路由表中查询出 N 个最近的节点
    debug!("gasket [{gasket_id}] 委托我们寻找资源 [{info_hash}] 的 peers");
    let this = DHT::global();
    let nodes = {
        let routing_table = this.routing_table.lock_pe();
        routing_table.find_closest_nodes(&info_hash, EXPECT_CLOSEST_NODE_NUM)
    };

    let mut queue = VecDeque::new();
    nodes.into_iter().for_each(|node| {
        let addr = node.addr();
        queue.push_back((Some(node.take_id()), addr))
    });
    let mut used_bootstrap_num = 0; // 使用过的 bootstrap 节点数量
    let mut min_dist = this.own_id.distance(&info_hash);
    let mut count = 0;
    let mut futures = FuturesUnordered::new();
    let info_hash = Arc::new(info_hash);

    'LOOP: loop {
        if queue.is_empty() {
            while used_bootstrap_num < this.bootstrap_nodes.len() {
                let domain = &this.bootstrap_nodes[used_bootstrap_num];
                used_bootstrap_num += 1;
                if let Some(addr) = this.domain_resolve(domain).await {
                    queue.push_back((None, addr));
                    break;
                }
            }
        }

        if this.gpcs.available_permits() == 0 || queue.is_empty() {
            if let Some(Ok(Some((mut nodes, md, find)))) = futures.next().await {
                queue.append(&mut nodes);
                count += find;
                if count >= expect_peers {
                    debug!("通过 dht 找到了足够的 peers");
                    while let Some((node_id, addr)) = queue.pop_front() {
                        check_add_node(&this.routing_table, &this.dht_request, node_id, &addr).await;
                    }
                    if let Some(receive_host) = receive_host.upgrade() {
                        receive_host.find_task_finished().await;
                    }
                    break 'LOOP;
                }
                min_dist = md;
            }
        }

        if let Some((node_id, addr)) = queue.pop_front() {
            let permit = this.gpcs.clone().acquire_owned().await.unwrap();
            let info_hash = info_hash.clone();
            let receive_host = receive_host.clone();
            let md = Arc::new(min_dist);

            futures.push(tokio::spawn(async move {
                let res = async_find_peers(node_id, addr, info_hash, receive_host, md).await;
                drop(permit);
                res
            }));
        } else if futures.is_empty() {
            debug!("没有可用的节点");
            break;
        }
    }

    debug!("find peers 结束");
}

/// dht 定时任务
pub struct DHTTimedTask<T, D> {
    /// 任务 id
    id: Id,

    /// 资源 hash 值
    info_hash: NodeId,

    /// 等待队列
    wait_queue: Arc<Mutex<VecDeque<T>>>,

    /// 发现 peer 后回调
    dispatch: Weak<D>,

    /// 主动扫描信号
    recv: Receiver<()>,
}

impl<T, D> Drop for DHTTimedTask<T, D> {
    fn drop(&mut self) {
        info!("DHTTimedTask [{}] 已 drop", self.id)
    }
}

impl<T, D: ReceiveHost + Send + Sync + 'static> DHTTimedTask<T, D> {
    pub fn new(
        id: Id, info_hash: NodeId, wait_queue: Arc<Mutex<VecDeque<T>>>, dispatch: Weak<D>,
        recv: Receiver<()>,
    ) -> Self {
        Self {
            id,
            info_hash,
            wait_queue,
            dispatch,
            recv,
        }
    }

    async fn find_peers(&self) {
        if self.wait_queue.lock_pe().len() < DHT_WAIT_PEER_LIMIT {
            #[rustfmt::skip]
            find_peers(
                self.info_hash.clone(),
                self.dispatch.clone(),
                self.id,
                DHT_EXPECT_PEERS,
            ).await;
        }
    }

    pub async fn run(mut self) {
        let mut tick = tokio::time::interval(DHT_FIND_PEERS_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    self.find_peers().await;
                }
                ret = self.recv.recv() => {
                    match ret {
                        Some(()) => {
                            tick.reset();
                            self.find_peers().await;
                        }
                        None => {
                            break
                        }
                    }
                }
            }
        }
    }
}
