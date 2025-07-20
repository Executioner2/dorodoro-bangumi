use crate::dht::DHT;
use crate::dht::routing::NodeId;
use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use doro_util::command_system;

command_system! {
    ctx: DHT,
    Command {
        Spread,
        FindPeers,
        RefreshRoutingTable,
    }
}

/// 发现了一个支持 DHT 的节点，向其发送 DHT 更新
#[derive(Debug)]
pub struct Spread {
    pub node_id: Option<NodeId>,
    pub addr: SocketAddr,
    pub info_hash: NodeId,
    pub resp_tx: Sender<TransferPtr>,
}
impl<'a> CommandHandler<'a, Result<()>> for Spread {
    type Target = &'a mut DHT;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        let min_dist = ctx.own_id.distance(&self.info_hash);
        DHT::async_find_peers(
            ctx.routing_table.clone(),
            ctx.dht_request.clone(),
            self.node_id,
            self.addr,
            Arc::new(self.info_hash),
            self.resp_tx,
            Arc::new(min_dist),
        )
        .await;
        Ok(())
    }
}

/// 从 DHT 网络中查询可用的 peers
#[derive(Debug)]
pub struct FindPeers {
    pub info_hash: NodeId,
    pub resp_tx: Sender<TransferPtr>,
    pub gasket_id: u64,
    pub expect_peers: usize,
}
impl<'a> CommandHandler<'a, Result<()>> for FindPeers {
    type Target = &'a mut DHT;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.find_peers(
            self.info_hash,
            self.resp_tx,
            self.gasket_id,
            self.expect_peers,
        )
        .await;
        Ok(())
    }
}

/// 扫描 DHT 网络中的节点，更新路由表
#[derive(Debug)]
pub struct RefreshRoutingTable;
impl<'a> CommandHandler<'a, Result<()>> for RefreshRoutingTable {
    type Target = &'a mut DHT;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.refresh_routing_table().await;
        Ok(())
    }
}
