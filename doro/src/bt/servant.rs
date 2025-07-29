use anyhow::{Error, Result};
use async_trait::async_trait;
use bytes::Bytes;
use doro_util::global::Id;

use crate::base_peer::error::PeerExitReason;
use crate::base_peer::{MsgType, Peer};

#[async_trait]
pub trait Servant: Send + Sync + 'static {
    /// 获取资源的 info hash
    fn info_hash(&self) -> &[u8];

    /// 获取 peer id
    fn peer_id(&self) -> &[u8];

    /// 添加 peer
    fn add_peer(&self, id: Id, peer: Peer);

    /// 移除 peer
    fn remote_peer(&self, id: Id);

    /// 获取 servant 回调接口
    fn servant_callback(&self) -> &dyn ServantCallback;

    /// 接收到的数据量
    fn reported_read_size(&self, id: Id, read_size: u64);

    /// 发生异常
    fn happen_exeception(&self, id: Id, error: Error);

    /// 处理心跳包
    async fn handle_heartbeat(&self, id: Id) -> Result<()>;

    /// 响应事件处理
    async fn handle(&self, id: Id, msg_type: MsgType, payload: Bytes) -> Result<()>;

    /// 处理 peer 退出
    async fn peer_exit(&self, id: Id, reason: PeerExitReason) -> Result<()>;
}

/// servant 的回调接口，用于事件处理的回调通知
#[async_trait]
#[allow(unused_variables)]
pub trait ServantCallback: Send + Sync + 'static {
    /// 分块存储成功
    async fn on_store_block_success(
        &self,
        id: Id,
        piece_idx: u32,
        block_offset: u32,
        block_size: u32,
    ) {
    }

    /// 分块存储失败
    async fn on_store_block_failed(
        &self,
        id: Id,
        piece_idx: u32,
        block_offset: u32,
        block_size: u32,
        error: Error,
    ) {
    }

    /// 分片校验成功
    async fn on_verify_piece_success(&self, id: Id, piece_idx: u32) {}

    /// 分片校验失败
    async fn on_verify_piece_failed(&self, id: Id, piece_idx: u32, error: Error) {}

    /// 握手成功
    async fn on_handshake_success(&self, id: Id) {}

    /// peer 退出
    async fn on_peer_exit(&self, id: Id, reason: PeerExitReason) {}
}
