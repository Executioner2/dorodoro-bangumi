use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use doro_util::command_system_ref;
use doro_util::global::Id;

use crate::base_peer::error::PeerExitReason;
use crate::base_peer::{MsgType, Servant};

type ServantRef = Arc<dyn Servant>;

command_system_ref! {
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
impl<'a> CommandHandler<'a, Result<()>> for Exit {
    type Target = &'a ServantRef;

    /// 什么都不需要做，在 peer 中会处理的
    async fn handle(self, servant: Self::Target) -> Result<()> {
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
impl<'a> CommandHandler<'a, Result<()>> for PeerTransfer {
    type Target = &'a ServantRef;

    async fn handle(self, servant: Self::Target) -> Result<()> {
        servant.reported_read_size(self.id, self.read_size); // 上报给 gasket
        servant.handle(self.id, self.msg_type, self.buf).await
    }
}

/// 心跳包
#[derive(Debug)]
pub struct Heartbeat {
    pub(crate) id: Id,
}
impl<'a> CommandHandler<'a, Result<()>> for Heartbeat {
    type Target = &'a ServantRef;

    async fn handle(self, servant: Self::Target) -> Result<()> {
        servant.handle_heartbeat(self.id).await
    }
}