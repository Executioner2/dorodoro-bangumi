use crate::peer::{MsgType, Peer};
use bytes::Bytes;
use tracing::debug;
use anyhow::{anyhow, Result};
use doro_util::command_system;
use doro_util::global::Id;
use crate::task_handler::gasket::error::PeerExitReason;

command_system! {
    ctx: Peer,
    Command {
        Exit,
        PeerTransfer,
        Heartbeat,
        PieceCheckoutFailed,
        PieceWriteFailed,
        TryRequestPiece,
        FreePiece,
    }
}

#[derive(Debug)]
pub struct Exit {
    pub reason: PeerExitReason,
}
impl<'a> CommandHandler<'a, Result<()>> for Exit {
    type Target = &'a mut Peer;

    /// 什么都不需要做，在 peer 中会处理的
    async fn handle(self, _ctx: Self::Target) -> Result<()> {
        unimplemented!()
    }
}

/// peer 网络传输指令
#[derive(Debug)]
pub struct PeerTransfer {
    pub(crate) msg_type: MsgType,
    pub(crate) buf: Bytes,
    pub(crate) read_size: u64,
}
impl<'a> CommandHandler<'a, Result<()>> for PeerTransfer {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.ctx.gc.reported_read_size(ctx.ctx.no, self.read_size); // 上报给 gasket
        ctx.handle(self.msg_type, self.buf).await
    }
}

/// 心跳包
#[derive(Debug)]
pub struct Heartbeat;
impl<'a> CommandHandler<'a, Result<()>> for Heartbeat {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.handle_heartbeat().await
    }
}

/// 分块校验失败
#[derive(Debug)]
pub struct PieceCheckoutFailed {
    pub(crate) piece_idx: u32,
}
impl<'a> CommandHandler<'a, Result<()>> for PieceCheckoutFailed {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        // 重置所有进行中的分片
        let pieces = ctx.response_pieces.keys().map(|key| *key).collect::<Vec<_>>();
        for pi in pieces {
            ctx.reset_request_piece_origin(pi, 0);
        }
        // 下载完最后一个 block 后，piece_index 会被删除，因此重新设置为 0
        ctx.reset_request_piece_origin(self.piece_idx, 0);
        Err(anyhow!("响应分块错误，piece_index: {}", self.piece_idx))
    }
}

/// 分块写入失败
#[derive(Debug)]
pub struct PieceWriteFailed {
    pub(crate) piece_idx: u32,
    pub(crate) block_offset: u32,
}
impl<'a> CommandHandler<'a, Result<()>> for PieceWriteFailed {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.reset_request_piece_origin(self.piece_idx, self.block_offset);
        Err(anyhow!("piece_index: {}, block_offset: {} 写入失败", self.piece_idx, self.block_offset))
    }
}

/// 尝试寻找可请求的分块
#[derive(Debug)]
pub struct TryRequestPiece;
impl<'a> CommandHandler<'a, Result<()>> for TryRequestPiece {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.request_piece().await
    }
}

/// 释放分块
#[derive(Debug)]
pub struct FreePiece {
    pub(crate) peer_no: Id,
    pub(crate) pieces: Vec<u32>,
}
impl<'a> CommandHandler<'a, Result<()>> for FreePiece {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.free_download_piece(&self.pieces).await;
        ctx.ctx.gc.assign_peer_handle(self.peer_no).await;
        debug!("将 {} 的分块给 {}\n给过去的分块有: {:?}", ctx.addr, self.peer_no, self.pieces);
        Ok(())
    }
}