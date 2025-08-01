use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Error, Result};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use doro_util::global::Id;

use crate::base_peer::error::PeerExitReason;
use crate::base_peer::{MsgType, Peer, PeerWrapper};
use crate::mapper::torrent::PieceStatus;

#[async_trait]
pub trait ServantContext: Send {
    /// 获取 peer
    fn get_peer(&self) -> &PeerWrapper;
}

#[async_trait]
pub trait Servant: Send + Sync + 'static {
    /// 获取资源的 info hash
    fn info_hash(&self) -> &[u8];

    /// 获取 peer id
    fn peer_id(&self) -> &[u8];

    /// 添加 peer
    async fn add_peer(&self, id: Id, peer: Peer);

    /// 获取 peer
    fn get_peer(&self, id: Id) -> Option<PeerWrapper>;

    /// 返回已完成的 piece 情况
    fn bytefield(&self) -> Arc<Mutex<BytesMut>>;

    /// 返回正在进行中的 piece
    fn underway_bytefield(&self) -> Arc<DashMap<u32, PieceStatus>>;

    /// 获取 servant 回调接口
    fn servant_callback(&self) -> &dyn ServantCallback;

    /// 接收到的数据量
    fn reported_read_size(&self, id: Id, read_size: u64);

    /// 检查 piece 是否接收完成
    fn check_piece_download_finished(&self) -> bool;

    /// 请求分片
    async fn request_piece(&self, id: Id) -> Result<()>;

    /// 发生异常
    async fn happen_exeception(&self, id: Id, error: Error);

    /// 处理心跳包
    async fn handle_heartbeat(&self, id: Id) -> Result<()>;

    /// 响应事件处理
    async fn handle(&self, id: Id, msg_type: MsgType, payload: Bytes) -> Result<()>;

    /// 处理 peer 退出
    async fn peer_exit(&self, id: Id, reason: PeerExitReason);
}

/// servant 的回调接口，用于事件处理的回调通知
#[async_trait]
#[allow(unused_variables)]
pub trait ServantCallback: Send + Sync + 'static {
    /// 接收到了分块数据
    async fn received_block(
        &self, sc: Box<dyn ServantContext>, piece_idx: u32, block_offset: u32, block_size: u32,
    ) -> Result<()>;

    /// 有新的分片可用了
    async fn have_piece_available(
        &self, sc: Box<dyn ServantContext>, piece_idx: u32, block_offset: u32,
    ) -> Result<()>;

    /// 可以进行请求
    async fn request_available(&self, sc: Box<dyn ServantContext>) -> Result<()>;

    /// 上报网络读取量
    fn reported_read_size(&self, sc: Box<dyn ServantContext>, read_size: u64);

    /// 没有分片可以下载了
    fn on_no_pieces_available(&self, sc: Box<dyn ServantContext>) -> Result<()>;

    /// 分块存储成功
    async fn on_store_block_success(
        &self, sc: Box<dyn ServantContext>, piece_idx: u32, block_offset: u32, block_size: u32,
    );

    /// 分块存储失败
    async fn on_store_block_failed(
        &self, sc: Box<dyn ServantContext>, piece_idx: u32, block_offset: u32, block_size: u32,
        error: Error,
    );

    /// 分片校验成功
    async fn on_verify_piece_success(&self, sc: Box<dyn ServantContext>, piece_idx: u32) {}

    /// 分片校验失败
    async fn on_verify_piece_failed(
        &self, sc: Box<dyn ServantContext>, piece_idx: u32, error: Error,
    );

    /// 对端传来他拥有的分片
    async fn owner_bitfield(&self, sc: Box<dyn ServantContext>, bitfield: Arc<RwLock<BytesMut>>) -> Result<()>;

    /// 握手成功
    async fn on_handshake_success(&self, sc: Box<dyn ServantContext>) -> Result<()>;

    /// peer 退出
    async fn on_peer_exit(&self, sc: Box<dyn ServantContext>, reason: PeerExitReason);
}
