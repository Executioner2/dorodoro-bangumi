use anyhow::{Error, Result};
use async_trait::async_trait;
use bytes::Bytes;
use doro_util::global::Id;

use crate::base_peer::error::PeerExitReason;
use crate::base_peer::{MsgType, Peer};
use crate::servant::{Servant, ServantCallback};

/// 默认的 servant 实现。
pub struct DefaultServant {
    callback: Box<dyn ServantCallback>,
}

impl DefaultServant {
    pub fn new(callback: impl ServantCallback) -> Self {
        Self {
            callback: Box::new(callback),
        }
    }
}

#[async_trait]
#[allow(unused_variables)]
impl Servant for DefaultServant {
    fn info_hash(&self) -> &[u8] {
        todo!()
    }

    fn peer_id(&self) -> &[u8] {
        todo!()
    }

    fn add_peer(&self, id: Id, peer: Peer) {
        todo!()
    }

    fn remote_peer(&self, id: Id) {
        todo!()
    }

    fn servant_callback(&self) -> &dyn ServantCallback {
        self.callback.as_ref()
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

    async fn peer_exit(&self, id: Id, reason: PeerExitReason) -> Result<()> {
        todo!()
    }
}
