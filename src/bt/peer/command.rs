use crate::bt::peer::error::Result;
use crate::command_system;
use crate::core::command::CommandHandler;
use crate::emitter::transfer::{CommandEnum, TransferPtr};
use crate::peer::error::Error::{PieceCheckoutError, PieceWriteError};
use crate::peer::{MsgType, Peer};
use crate::peer_manager::gasket::ExitReason;
use bytes::Bytes;

command_system! {
    ctx: Peer,
    Command {
        Exit,
        PeerTransfer,
        PieceCheckoutFailed,
        PieceWriteFailed,
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
        ctx.update_response_pieces(self.piece_index, 0);
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
        ctx.update_response_pieces(self.piece_index, self.block_offset);
        Err(PieceWriteError(self.piece_index, self.block_offset))
    }
}
