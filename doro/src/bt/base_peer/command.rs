use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use doro_util::command_system_owner;
use doro_util::global::Id;

use crate::base_peer::error::PeerExitReason;
use crate::base_peer::{MsgType, Servant};
use crate::context::AsyncSemaphore;

type ServantRef = (Arc<dyn Servant>, Option<AsyncSemaphore>);

command_system_owner! {
    ctx: ServantRef,
    Command {
        Exit,
        PeerTransfer,
        Heartbeat
    }
}

#[derive(Debug)]
pub struct Exit {
    pub(crate) id: Id,
    pub(crate) reason: PeerExitReason,
}
impl CommandHandler<'_, Result<()>> for Exit {
    type Target = ServantRef;

    /// 什么都不需要做，在 peer 中会处理的
    async fn handle(self, (servant, _ats): Self::Target) -> Result<()> {
        servant.peer_exit(self.id, self.reason).await;
        Ok(())
    }
}

/// peer 网络传输指令
#[derive(Debug)]
pub struct PeerTransfer {
    pub(crate) id: Id,
    pub(crate) msg_type: MsgType,
    pub(crate) buf: Bytes,
    pub(crate) read_size: u64,
}
impl CommandHandler<'_, Result<()>> for PeerTransfer {
    type Target = ServantRef;

    async fn handle(self, (servant, _ats): Self::Target) -> Result<()> {
        servant.reported_read_size(self.id, self.read_size); // 上报给 gasket
        servant.handle(self.id, self.msg_type, self.buf).await
    }
}

/// 心跳包
#[derive(Debug)]
pub struct Heartbeat {
    pub(crate) id: Id,
}
impl CommandHandler<'_, Result<()>> for Heartbeat {
    type Target = ServantRef;

    async fn handle(self, (servant, _ats): Self::Target) -> Result<()> {
        servant.handle_heartbeat(self.id).await
    }
}