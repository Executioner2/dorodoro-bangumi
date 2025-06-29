//! dht 实现

use crate::command::CommandHandler;
use crate::context::Context;
use crate::dht::command::Command;
use crate::dht::entity::{DHTBase, GetPeersReq, GetPeersResp, Host, Ping};
use crate::emitter::Emitter;
use crate::emitter::constant::DHT_PREFIX;
use crate::peer_manager::gasket;
use crate::peer_manager::gasket::command::PeerSource;
use crate::runtime::Runnable;
use crate::udp_server::UdpServer;
use alloc::borrow::Cow;
use bendy::decoding::FromBencode;
use bendy::encoding::ToBencode;
use bendy::value::Value;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};
use crate::bendy_ext::SocketAddrExt;

mod command;
pub mod entity;

const TIMEOUT: Duration = Duration::from_secs(10);
const DISCOVER_MIN: usize = 100;

fn distancemetric(a: &[u8], b: &[u8]) -> [u8; 20] {
    if a.len() != 20 || b.len() != 20 {
        error!("长度不等\ta len: {}\tb len: {}", a.len(), b.len());
        return [0; 20]
    }
    let mut res = [0u8; 20];
    for i in 0..20 {
        res[i] = a[i] ^ b[i];
    }
    res
}

pub struct DHT {
    /// id 标识。同 gasket id 一致
    id: u64,

    /// 节点 ID
    node_id: Arc<[u8; 20]>,

    /// torrent 信息哈希值
    info_hash: Arc<[u8; 20]>,

    /// 全局上下文
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
        node_id: Arc<[u8; 20]>,
        info_hash: Arc<[u8; 20]>,
        emitter: Emitter,
        ctx: Context,
        cancel_token: CancellationToken,
        udp_server: UdpServer,
    ) -> Self {
        Self {
            id,
            node_id,
            info_hash,
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
            Ping::new(Value::Bytes(Cow::from(&*self.node_id))),
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
                Value::Bytes(Cow::from(&*self.node_id)),
                Value::Bytes(Cow::from(&*self.info_hash)),
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
    async fn spread(&self, addr: SocketAddr) {
        debug!("收到一个可以扩散的ip: {}", addr);
        let mut queue = VecDeque::new();
        queue.push_back(addr);
        let mut peers = vec![];

        let mut min_dist = distancemetric(&*self.node_id, &*self.info_hash); // todo - 这个要改

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
                let dist = distancemetric(&node.id, &*self.info_hash);
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
            let cmd = gasket::command::DiscoverPeerAddr {
                peers: peers.iter().map(|p| p.addr.into()).collect(),
                source: PeerSource::DHT,
            };
            let _ = self
                .emitter
                .send(&gasket::get_transfer_id(self.id), cmd.into())
                .await;
        }
    }

    /// 开始扫描 peers
    async fn peers_scan(&self) {
        debug!("开始扫描 peers");
    }

    fn shutdown(self) {
        // 从 emiiter 中移除自己
        self.emitter.remove(&get_transfer_id(self.id));
    }
}

#[inline]
pub fn get_transfer_id(id: u64) -> String {
    format!("{}{}", DHT_PREFIX, id)
}

impl Runnable for DHT {
    async fn run(mut self) {
        debug!("dht started");
        let (send, mut recv) = channel(self.ctx.get_config().channel_buffer());
        let transfer_id = get_transfer_id(self.id);
        self.emitter.register(transfer_id, send.clone());

        // 15 分钟扫描一次
        let interval = Duration::from_secs(15 * 60);
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now(), interval);

        // fixme - 测试用
        // let _ = self
        //     .emitter
        //     .send(
        //         &get_transfer_id(self.id),
        //         command::Spread {
        //             addr: SocketAddr::from_str("192.168.2.242:3115").unwrap(),
        //         }
        //         .into(),
        //     )
        //     .await;

        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                },
                cmd = recv.recv() => {
                    match cmd {
                        Some(cmd) => {
                            let cmd: Command = cmd.instance();
                            if let Err(e) = cmd.handle(&mut self).await {
                                error!("处理指令出现错误\t{}", e);
                                break;
                            }
                        }
                        None => {
                            error!("dht {} 接收到空命令！", get_transfer_id(self.id));
                            break;
                        }
                    }
                }
                _ = tick.tick() => {
                    self.peers_scan().await;
                }
            }
        }

        self.shutdown();
        debug!("dht stopped");
    }
}
