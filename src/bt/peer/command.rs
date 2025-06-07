use crate::bt::peer::error::Result;
use crate::command_system;
use crate::core::command::CommandHandler;
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::peer::error::Error::{PieceCheckoutError, PieceWriteError};
use crate::peer::{MsgType, Peer};
use crate::peer_manager::gasket::ExitReason;
use bytes::Bytes;
use tracing::debug;

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
    pub reason: ExitReason,
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
    pub(crate) piece_index: u32,
}
impl<'a> CommandHandler<'a, Result<()>> for PieceCheckoutFailed {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        // 清空所有正在进行以及校验失败了的分块
        ctx.response_pieces
            .iter_mut()
            .for_each(|(_, value)| value.block_offset = 0);
        // 下载完最后一个 block 后，piece_index 会被删除，因此重新设置为 0
        ctx.insert_response_pieces(self.piece_index, 0);
        Err(PieceCheckoutError(self.piece_index))
    }
}

/// 分块写入失败
#[derive(Debug)]
pub struct PieceWriteFailed {
    pub(crate) piece_index: u32,
    pub(crate) block_offset: u32,
}
impl<'a> CommandHandler<'a, Result<()>> for PieceWriteFailed {
    type Target = &'a mut Peer;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.insert_response_pieces(self.piece_index, self.block_offset);
        Err(PieceWriteError(self.piece_index, self.block_offset))
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
    pub(crate) peer_no: u64,
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