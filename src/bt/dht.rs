//! dht 实现

use crate::command::CommandHandler;
use crate::context::Context;
use crate::dht::command::Command;
use crate::dht::entity::{DHTBase, GetPeersReq, GetPeersResp, Host, Ping};
use crate::emitter::Emitter;
use crate::emitter::constant::DHT_PREFIX;
use crate::peer_manager::gasket;
use crate::peer_manager::gasket::command::PeerSource;
use crate::runtime::{CommandHandleResult, CustomTaskResult, Runnable};
use crate::udp_server::UdpServer;
use bendy::decoding::FromBencode;
use bendy::encoding::ToBencode;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use futures::stream::FuturesUnordered;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::{debug, error, trace};
use crate::bendy_ext::SocketAddrExt;
use crate::dht::node_id::NodeId;
use crate::emitter::transfer::TransferPtr;
use crate::peer_manager::gasket::Gasket;
use anyhow::Result;

mod command;
pub mod entity;
pub mod node_id;

#[cfg(test)]
mod tests;

const TIMEOUT: Duration = Duration::from_secs(10);
const DISCOVER_MIN: usize = 100;

#[derive(Clone)]
pub struct DHT {
    /// id 标识。同 gasket id 一致
    id: u64,

    /// 节点 ID
    node_id: Arc<NodeId>,

    /// torrent 信息哈希值
    info_hash: Arc<NodeId>,

    /// 全局上下文
    #[allow(dead_code)]
    ctx: Context,

    /// 命令发射器。注册到全局 emitter 中
    emitter: Emitter,

    /// 退出标记
    cancel_token: CancellationToken,

    /// dht 服务器
    udp_server: UdpServer,
}

impl DHT {
    pub fn new(
        id: u64,
        info_hash: [u8; 20],
        emitter: Emitter,
        ctx: Context,
        cancel_token: CancellationToken,
        udp_server: UdpServer,
    ) -> Self {
        let node_id = ctx.get_node_id();
        Self {
            id,
            node_id,
            info_hash: Arc::new(NodeId::from(info_hash)),
            emitter,
            ctx,
            cancel_token,
            udp_server,
        }
    }

    /// 发送一个 ping 操作
    async fn ping(&self, addr: SocketAddr) -> bool {
        debug!("开始尝试 ping");
        let t = self.udp_server.tran_id();
        let ping = DHTBase::<Ping>::request(
            Ping::new(self.node_id.to_value_bytes()),
            "ping".to_string(),
            t,
        );
        let data = ping.to_bencode().unwrap();
        let response = self.udp_server.request(t, &data, addr).await;
        if response.is_err() {
            debug!(
                "执行 ping，获取 Response Future 错误: {:?}",
                response.unwrap_err()
            );
            return false;
        }
        match tokio::time::timeout(TIMEOUT, response.unwrap()).await {
            Ok((data, _addr)) => {
                let resp = DHTBase::<Ping>::from_bencode(data.as_ref());
                resp.is_ok()
            }
            Err(e) => {
                error!("ping request error: {}", e);
                false
            }
        }
    }

    async fn get_peers(&self, addr: SocketAddr) -> (Vec<Host>, Vec<SocketAddrExt>) {
        debug!("开始尝试 get_peers");
        let t = self.udp_server.tran_id();
        let get_peers = DHTBase::<GetPeersReq>::request(
            GetPeersReq::new(
                self.node_id.to_value_bytes(),
                self.info_hash.to_value_bytes(),
            ),
            "get_peers".to_string(),
            t,
        );

        let default = (Vec::default(), Vec::default());
        let data = get_peers.to_bencode().unwrap();
        let response = self
            .udp_server
            .request(t, &data, addr)
            .await;
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
            },
            Err(e) => {
                error!("get_peers request error: {}", e);
            }
        }

        default
    }

    /// 有一个支持 DHT 的节点，那么根据它发现更多的 peers 和 nodes
    async fn spread(&self, addr: SocketAddr) -> Result<()> {
        debug!("收到一个可以扩散的ip: {}", addr);
        let mut queue = VecDeque::new();
        queue.push_back(addr);
        let mut peers = vec![];
        let mut min_dist = self.node_id.distance(&*self.info_hash);

        while !queue.is_empty() && peers.len() < DISCOVER_MIN {
            let addr = queue.pop_front().unwrap();

            if !self.ping(addr).await {
                debug!("ping 不通[{}]", addr);
                continue;
            }

            // 可以连通，那么开始广播查询 peers
            let (nodes, mut values) = self.get_peers(addr).await;
            let mut min = min_dist.clone();
            for node in nodes {
                let dist = node.id.distance(&*self.info_hash);
                if min_dist >= dist {
                    queue.push_back(node.addr.into());
                    min = min.min(dist);
                }
            }
            min_dist = min;

            peers.append(&mut values);
        }

        // 告诉 gasket 有新增的 peers
        if !peers.is_empty() {
            trace!("新增了的peer: {:?}", peers);
            let cmd = gasket::command::DiscoverPeerAddr {
                peers: peers.iter().map(|p| p.addr.into()).collect(),
                source: PeerSource::DHT,
            };
            let _ = self
                .emitter
                .send(&Gasket::get_transfer_id(self.id), cmd.into())
                .await;
        }
        
        Ok(())
    }

    /// 开始扫描 peers
    async fn peers_scan(&self) {
        debug!("开始扫描 peers");
    }
    
    /// 定时扫描
    fn register_interval_scan(&self) -> Pin<Box<dyn Future<Output=CustomTaskResult> + Send + 'static>> {
        let id = Self::get_transfer_id(self.id);
        let emitter = self.emitter.clone();
        Box::pin(async move {
            loop {
                let _ = emitter.send(&id, command::PeersScan.into()).await;
                tokio::time::sleep(Duration::from_secs(15 * 60)).await;
            }
        })
    }
}

impl Runnable for DHT {
    fn emitter(&self) -> &Emitter {
        &self.emitter
    }

    fn get_transfer_id<T: ToString>(suffix: T) -> String {
        format!("{}{}", DHT_PREFIX, suffix.to_string())
    }

    fn get_suffix(&self) -> String {
        self.id.to_string()
    }

    fn register_lt_future(&mut self) -> FuturesUnordered<Pin<Box<dyn Future<Output=CustomTaskResult> + Send + 'static>>> {
        // 注册一个定时任务的 future 就行了
        let futs = FuturesUnordered::new();
        futs.push(self.register_interval_scan());  
        futs
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel_token.cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }
}