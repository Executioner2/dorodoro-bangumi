use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use anyhow::{Error, Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use doro_util::global::Id;

use crate::base_peer::error::{exception, PeerExitReason};
use crate::base_peer::{MsgType, Peer};
use crate::servant::{Servant, ServantCallback, ServantContext};
use crate::task_manager::PeerId;

pub struct DefaultServantContext {
    peer: Peer,
}

impl ServantContext for DefaultServantContext {
    fn get_peer(&self) -> &Peer {
        &self.peer
    }
}

/// 默认的 servant 实现。
#[derive(Default)]
pub struct DefaultServant {
    /// 种子信息哈希值
    info_hash: [u8; 20],

    /// peer id
    peer_id: PeerId,

    /// 回调函数
    callback: OnceLock<Arc<dyn ServantCallback>>,

    /// 保存所有连接的 peer
    peers: Arc<DashMap<Id, Peer>>,
}

impl DefaultServant {
    pub fn new(peer_id: PeerId, info_hash: [u8; 20]) -> Self {
        Self {
            peer_id,
            info_hash,
            ..Default::default()
        }
    }

    pub fn init(&self, callback: impl ServantCallback) {
        self.callback.set(Arc::new(callback))
            .map_err(|_| anyhow!("callback init failed"))
            .unwrap();
    }

    fn callback(&self) -> &dyn ServantCallback {
        if let Some(callback) = self.callback.get() {
            callback.as_ref()
        } else {
            panic!("callback not init")
        }
    }

    fn gen_servant_context(&self, id: Id) -> Option<DefaultServantContext> {
        if let Some(peer) = self.peers.get(&id) {
            return Some(self.servant_context(peer.value().clone()));
        }
        None
    }

    fn take_servant_context(&self, id: Id) -> Option<DefaultServantContext> {
        if let Some((_, peer)) = self.peers.remove(&id) {
            return Some(self.servant_context(peer));
        }
        None
    }

    fn servant_context(&self, peer: Peer) -> DefaultServantContext {
        DefaultServantContext {
            peer,
        }
    }
}

#[async_trait]
#[allow(unused_variables)]
impl Servant for DefaultServant {
    fn info_hash(&self) -> &[u8] {
        self.info_hash.as_ref()
    }

    fn peer_id(&self) -> &[u8] {
        self.peer_id.value().as_ref()
    }

    async fn add_peer(&self, id: Id, peer: Peer) {
        if let Some(dsc) = self.gen_servant_context(id) {
            if let Err(e) = self.callback().on_handshake_success(&dsc).await {
                self.callback().on_peer_exit(&dsc, exception(e)).await;
                return;
            }
        }
        self.peers.insert(id, peer);
    }

    fn get_peer(&self, id: Id) -> Option<Peer> {
        self.peers.get(&id).map(|v| v.value().clone())
    }

    async fn remove_peer(&self, id: Id) {
        if let Some(dsc) = self.take_servant_context(id) {
            dsc.get_peer().shutdown().await;
            self.callback().on_peer_exit(&dsc, PeerExitReason::ClientExit).await;
        }
    }

    fn servant_callback(&self) -> &dyn ServantCallback {
        self.callback()
    }

    fn reported_read_size(&self, id: Id, read_size: u64) {
        todo!()
    }

    fn happen_exeception(&self, id: Id, error: Error) {
        todo!()
    }

    async fn handle_heartbeat(&self, id: Id) -> Result<()> {
        todo!()
    }

    async fn handle(&self, id: Id, msg_type: MsgType, payload: Bytes) -> Result<()> {
        todo!()
    }

    async fn peer_exit(&self, id: Id, reason: PeerExitReason) {
        todo!()
    }

    fn shutdown(self) -> Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>> {
        // 无须特意去调用 shutdown，因为默认的实现中，peer 没有实例持有后，会自动 drop 掉。
        // 而在 peer 中的 JoinSet 也会自动被 drop 掉，JoinSet 的 drop 会终止掉所有任务。
        Box::pin(async move {
            self.peers.clear();
        })
    }
}
