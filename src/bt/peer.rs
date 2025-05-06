//! 端到端通信。
//!
//! 协议格式：
//! `长度（4字节） | 消息类型（1字节） | 数据（长度-5字节）`
//!
//! 长度的需要加上消息类型的 1 字节。

pub mod command;
mod error;
mod future;

use crate::bytes::Bytes2Int;
use crate::command::CommandHandler;
use crate::core::protocol::{
    BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN,
};
use crate::core::runtime::Runnable;
use crate::emitter::Emitter;
use crate::emitter::constant::PEER_PREFIX;
use crate::emitter::transfer::TransferPtr;
use crate::peer::command::{Exit, PeerTransfer, PieceCheckoutFailed, PieceWriteFailed};
use crate::peer::error::Error;
use crate::peer::error::Error::{
    BitfieldError, HandshakeError, ResponseDataIncomplete, TryFromError,
};
use crate::peer::future::BtResp;
use crate::peer_manager::gasket::{ExitReason, GasketContext};
use crate::store::Store;
use crate::util;
use ahash::HashMap;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::{Bytes, BytesMut};
use error::Result;
use std::cmp::min;
use std::mem;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{Sender, channel};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, trace, warn};

/// Peer 通信的消息类型
#[derive(Debug)]
pub enum MsgType {
    /// 我现在还不想接收你的请求
    ///
    /// 格式：`length:u32 | 0:u8`
    Choke,

    /// 你现在可以向我发起请求了
    ///
    /// 格式：`length:u32 | 1:u8`
    UnChoke,

    /// 感兴趣，期望可以允许我发起请求
    ///
    /// 格式：`length:u32 | 2:u8`
    Interested,

    /// 我已经对你不感兴趣了，你可以不再理我（choke）\
    /// 告诉对方自己已经下载完他的资源了
    ///
    /// 格式：`length:u32 | 3:u8`
    NotInterested,

    /// 我新增了这个分块，你要不要呀
    ///
    /// 格式：`length:u32 | 4:u8 | piece_index:u32`
    Have,

    /// 你不要告诉别人，我偷偷给你说，我拥有这些分块\
    /// 一般在建立完链接后发送
    ///
    /// 格式：`length:u32 | 5:u8 | bitfield:&[u8]`
    Bitfield,

    /// 请给我这个分块
    ///
    /// 格式：`length:u32 | 6:u8 | piece_index:u32 | block_offset:u32 | block_length:u32`
    Request,

    /// 好的，给你这个分块
    ///
    /// 格式：`length:u32 | 7:u8 | piece_index:u32 | block_offset:u32 | block_data:&[u8]`
    Piece,

    /// 噢撤回，我不需要这个分块了
    ///
    /// 格式：`length:u32 | 8:u8 | piece_index:u32 | block_offset:u32`
    Cancel,
}

impl TryFrom<u8> for MsgType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        if value <= 8 {
            Ok(unsafe { mem::transmute(value) })
        } else {
            Err(TryFromError)
        }
    }
}

#[derive(Eq, PartialEq, Hash)]
enum Status {
    Choke,
    UnChoke,
    _Wait, // 等待新的分块下载
}

pub struct Peer {
    /// 编号
    no: u64,

    /// gasket 共享上下文
    context: GasketContext,

    /// reader，peer actor 启动后会把这个 reader 消费掉
    reader: Option<OwnedReadHalf>,

    /// wrtier
    writer: OwnedWriteHalf,

    /// 当前 peer 的状态
    status: Status,

    /// 对端地址
    addr: Arc<SocketAddr>,

    /// 对端的 peer id
    opposite_peer_id: Option<[u8; 20]>,

    /// 对端的状态
    opposite_peer_status: Status,

    /// 对端拥有的分块
    opposite_peer_bitfield: Option<BytesMut>,

    /// 指令发射器
    emitter: Emitter,

    /// 存储处理
    store: Store,

    /// 成功收到的 piece 的响应次数
    secuessed_piece: u64,

    /// 当前正在下载的分块
    ///
    /// (分块下标 偏移位置 )
    pub(super) downloading_pieces: HashMap<u32, u32>,
}

impl Peer {
    pub async fn new(
        no: u64,
        addr: Arc<SocketAddr>,
        context: GasketContext,
        emitter: Emitter,
        store: Store,
    ) -> Option<Self> {
        let timeout = context.config().peer_connection_timeout();
        let stream = match tokio::time::timeout(timeout, TcpStream::connect(&*addr)).await {
            Ok(Ok(stream)) => stream,
            _ => return None,
        };
        let (reader, writer) = stream.into_split();
        Some(Peer {
            no,
            context,
            reader: Some(reader),
            writer,
            status: Status::Choke,
            addr,
            opposite_peer_id: None,
            opposite_peer_status: Status::Choke,
            opposite_peer_bitfield: None,
            emitter,
            store,
            secuessed_piece: 0,
            downloading_pieces: HashMap::default(),
        })
    }

    /// 握手，然后询问对方有哪些分块是可以下载的
    pub async fn start(&mut self) -> Result<()> {
        // 握手信息
        trace!("发送握手信息 peer_no:{}", self.no);
        let mut bytes =
            Vec::with_capacity(1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN);
        let info_hash = &self.context.torrent().info_hash;
        WriteBytesExt::write_u8(&mut bytes, BIT_TORRENT_PROTOCOL_LEN)?;
        std::io::Write::write(&mut bytes, BIT_TORRENT_PROTOCOL)?;
        WriteBytesExt::write_u64::<BigEndian>(&mut bytes, 0u64)?;
        std::io::Write::write(&mut bytes, info_hash)?;
        std::io::Write::write(&mut bytes, self.context.peer_id())?;

        // 发送握手信息
        self.writer.write_all(&bytes).await?;

        // 等待握手信息
        self.reader.as_ref().unwrap().readable().await?;

        let mut handshake_resp = vec![0u8; bytes.len()];
        let size = self
            .reader
            .as_mut()
            .unwrap()
            .read(&mut handshake_resp)
            .await?;
        if size != bytes.len() {
            error!("响应数据长度于预期不符 [{}]\t[{}]", size, bytes.len());
            return Err(HandshakeError);
        }

        let protocol_len = u8::from_be_bytes([handshake_resp[0]]) as usize;
        let resp_info_hash = &handshake_resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
        let peer_id = &handshake_resp[1 + protocol_len + 8 + 20..];

        if info_hash != resp_info_hash {
            error!("没有在讨论同一个资源文件");
            return Err(HandshakeError);
        }

        self.opposite_peer_id = Some(peer_id.try_into().unwrap());

        // 告诉对方自己拥有的分块
        let bitfield = &*self.context.bytefield().lock().await;
        let mut bytes = Vec::with_capacity(5 + bitfield.len());
        WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 1 + bitfield.len() as u32)?;
        WriteBytesExt::write_u8(&mut bytes, MsgType::Bitfield as u8)?;
        std::io::Write::write(&mut bytes, &bitfield[..])?;
        self.writer.write_all(&bytes).await?;

        // 告诉对方，允许发起请求
        let mut req = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut req, 1)?;
        WriteBytesExt::write_u8(&mut req, MsgType::UnChoke as u8)?;
        self.writer.write_all(&bytes).await?;

        // 告诉对方，对他拥有的资源很感兴趣
        let mut bytes = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 1)?;
        WriteBytesExt::write_u8(&mut bytes, MsgType::Interested as u8)?;
        self.writer.write_all(&bytes).await?;

        Ok(())
    }

    async fn handle(&mut self, msg_type: MsgType, bytes: Bytes) -> Result<()> {
        let res = match msg_type {
            MsgType::Choke => self.handle_choke().await?,
            MsgType::UnChoke => self.handle_un_choke().await?,
            MsgType::Interested => self.handle_interested().await?,
            MsgType::NotInterested => self.handle_not_interested().await?,
            MsgType::Have => self.handle_have(bytes).await?,
            MsgType::Bitfield => self.handle_bitfield(bytes).await?,
            MsgType::Request => self.handle_request(bytes).await?,
            MsgType::Piece => self.handle_piece(bytes).await?,
            MsgType::Cancel => self.handle_cancel(bytes).await?,
        };
        Ok(res)
    }

    pub fn set_downloading_pieces(&mut self, piece_index: u32, block_offset: u32) {
        self.downloading_pieces.insert(piece_index, block_offset);
    }

    /// 顺序请求分块
    async fn request_block(&mut self, piece_index: u32) -> Result<bool> {
        let sharding_size = self.context.config().sharding_size();
        let piece_length = self.get_piece_length(piece_index);
        let offset = self.downloading_pieces.entry(piece_index).or_insert(0);

        let over = *offset >= piece_length;
        if !over {
            debug!(
                "peer_no [{}] 请求下载分块 {} - {}",
                self.no, piece_index, offset
            );
            let mut req = Vec::with_capacity(13);
            WriteBytesExt::write_u32::<BigEndian>(&mut req, 13)?;
            WriteBytesExt::write_u8(&mut req, MsgType::Request as u8)?;
            WriteBytesExt::write_u32::<BigEndian>(&mut req, piece_index)?;
            WriteBytesExt::write_u32::<BigEndian>(&mut req, *offset)?;
            WriteBytesExt::write_u32::<BigEndian>(
                &mut req,
                min(piece_length - *offset, sharding_size),
            )?;
            self.writer.write_all(&req).await?;
        } else {
            self.downloading_pieces.remove(&piece_index);
        }

        Ok(over)
    }

    fn get_piece_length(&self, piece_index: u32) -> u32 {
        let torrent = self.context.torrent();
        let piece_length = torrent.info.piece_length;
        let resource_length = torrent.info.length;
        piece_length.min(resource_length.saturating_sub(piece_index as u64 * piece_length)) as u32
    }

    fn is_can_be_download(&self) -> bool {
        let config = self.context.config();
        self.status == Status::UnChoke
            && self.downloading_pieces.len() < config.con_req_piece_limit()
            && self.secuessed_piece % config.sucessed_recv_piece() as u64 == 0
    }

    /// 尝试寻找可以下载的分块进行下载
    async fn try_find_downloadable_pices(&mut self) -> Result<bool> {
        if !self.is_can_be_download() {
            return Ok(false);
        }

        let torrent = self.context.torrent();
        let bit_len = torrent.info.pieces.len() / 20;

        // 先下载暂停的分块
        if let Some((piece_index, block_offset)) = self.context.apply_download_pasue_piece() {
            debug!("peer_no [{}] 发现新的可下载分块：{}", self.no, piece_index);
            self.downloading_pieces.insert(piece_index, block_offset);
            self.request_block(piece_index).await?;
            return Ok(true);
        }

        for i in 0..bit_len {
            let (idx, offset) = util::bytes::bitmap_offset(i);

            if let Some(true) = self
                .opposite_peer_bitfield
                .as_ref()
                .map(|bytes| bytes[idx] & offset != 0)
            {
                let piece_index = i as u32;
                if let Some(block_offset) = self
                    .context
                    .apply_download_piece(self.no, piece_index)
                    .await
                {
                    debug!("peer_no [{}] 发现新的可下载分块：{}", self.no, i);
                    self.downloading_pieces.insert(piece_index, block_offset);
                    self.request_block(piece_index).await?;
                    return Ok(true);
                }
            }
        }

        // 没有分块可以下载了，告诉 gasket
        if self.downloading_pieces.is_empty() {
            self.context.report_no_downloadable_piece(self.no).await;
        }
        Ok(false)
    }

    /// 不让我们请求数据了
    async fn handle_choke(&mut self) -> Result<()> {
        trace!("peer_no [{}] 对端不让我们请求下载数据", self.no);
        self.status = Status::Choke;
        Ok(())
    }

    /// 告知可以交换数据了
    async fn handle_un_choke(&mut self) -> Result<()> {
        trace!("peer_no [{}] 对端告诉我们可以下载数据", self.no);
        self.status = Status::UnChoke;
        self.try_find_downloadable_pices().await?;
        Ok(())
    }

    /// 对我们的数据感兴趣
    async fn handle_interested(&mut self) -> Result<()> {
        trace!("对端对我们的数据感兴趣，那我们就允许他下载");
        self.opposite_peer_status = Status::UnChoke;
        let mut req = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut req, 1)?;
        WriteBytesExt::write_u8(&mut req, MsgType::UnChoke as u8)?;
        self.writer.write_all(&req).await?;
        Ok(())
    }

    /// 对我们的数据不再感兴趣了
    async fn handle_not_interested(&mut self) -> Result<()> {
        trace!("对端对我们的数据不再感兴趣了，那就告诉他你别下载了！");
        self.opposite_peer_status = Status::Choke;
        let mut req = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut req, 1)?;
        WriteBytesExt::write_u8(&mut req, MsgType::Choke as u8)?;
        self.writer.write_all(&req).await?;
        Ok(())
    }

    /// 对端告诉我们他有新的分块可以下载
    async fn handle_have(&mut self, bytes: Bytes) -> Result<()> {
        trace!("对端告诉我们他有新的分块可以下载");
        if bytes.len() != 4 {
            return Err(ResponseDataIncomplete);
        }
        let bit = u32::from_be_slice(&bytes[..4]);
        let (idx, offset) = util::bytes::bitmap_offset(bit as usize);

        self.opposite_peer_bitfield
            .as_mut()
            .map(|bytes| bytes[idx] |= offset);

        self.try_find_downloadable_pices().await?;

        Ok(())
    }

    /// 对端告诉我们他有哪些分块可以下载
    async fn handle_bitfield(&mut self, bytes: Bytes) -> Result<()> {
        trace!("对端告诉我们他有哪些分块可以下载: {:?}", &bytes[..]);
        let torrent = self.context.torrent();
        let bit_len = (torrent.info.pieces.len() / 20 + 7) >> 3;
        if bytes.len() != bit_len {
            warn!(
                "远端给到的 bitfield 长度和期望的不一致，期望的: {}\t实际的bytes: {:?}",
                bit_len,
                &bytes[..]
            );
            return Err(BitfieldError);
        }

        self.opposite_peer_bitfield = Some(BytesMut::from(bytes));
        self.try_find_downloadable_pices().await?;

        Ok(())
    }

    /// 向我们请求数据
    async fn handle_request(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端向我们请求数据");
        if self.opposite_peer_status != Status::UnChoke {
            return Ok(());
        }
        Ok(())
    }

    /// 对端给我们发来了数据
    async fn handle_piece(&mut self, mut bytes: Bytes) -> Result<()> {
        trace!("peer_no [{}] 收到了对端发来的数据", self.no);
        if bytes.len() < 8 {
            return Err(ResponseDataIncomplete);
        }

        let piece_index = u32::from_be_slice(&bytes[0..4]);
        let block_offset = u32::from_be_slice(&bytes[4..8]);
        let block_data = bytes.split_off(8);
        let block_data_len = block_data.len();
        self.write_file(piece_index, block_offset, block_data).await;

        let sharding_size = self.context.config().sharding_size();
        let offset = self.downloading_pieces.entry(piece_index).or_insert(0);
        *offset += sharding_size;
        self.secuessed_piece += 1;

        if self.request_block(piece_index).await? {
            self.checkout(piece_index); // 校验分块 hash
        }

        // 上报给 gasket
        self.context.reported_download(block_data_len as u64);
        self.try_find_downloadable_pices().await?;

        Ok(())
    }

    /// 对端撤回了刚刚请求的那个分块
    async fn handle_cancel(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端撤回了刚刚请求的那个分块");
        Ok(())
    }

    /// 写入文件
    async fn write_file(&self, piece_index: u32, block_offset: u32, mut block_data: Bytes) {
        let context = self.context.clone();
        let store = self.store.clone();
        let sender = self.emitter.get(&Self::get_transfer_id(self.no)).unwrap();

        // tokio::spawn(async move {
        let torrent = context.torrent();
        let path = context.save_path();

        for block_info in
            torrent.find_file_of_piece_index(path, piece_index, block_offset, block_data.len())
        {
            let data = block_data.split_to(block_info.len);
            if store.write(block_info, data).await.is_err() {
                let cmd = PieceWriteFailed {
                    piece_index,
                    block_offset,
                }
                .into();
                sender.send(cmd).await.unwrap();
            }
        }
        // });
    }

    /// 校验分块
    fn checkout(&self, piece_index: u32) {
        let context = self.context.clone();
        let read_length = self.get_piece_length(piece_index);
        let sender = self.emitter.get(&Self::get_transfer_id(self.no)).unwrap();
        let store = self.store.clone();

        tokio::spawn(async move {
            trace!("开始校验 {} 个分块", piece_index);
            let torrent = context.torrent();
            let path = context.save_path();
            let hash =
                &torrent.info.pieces[piece_index as usize * 20..(piece_index as usize + 1) * 20];
            let list = torrent.find_file_of_piece_index(path, piece_index, 0, read_length as usize);
            match store.checkout(list.clone(), hash).await {
                Ok(false) | Err(_) => {
                    error!("参与计算的文件: {:?}", list);
                    let cmd = PieceCheckoutFailed { piece_index }.into();
                    sender.send(cmd).await.unwrap();
                }
                _ => {
                    trace!("第 {} 个分块校验通过", piece_index);
                    context.reported_piece_finished(piece_index).await;
                }
            }
        });
    }

    pub fn get_transfer_id(no: u64) -> String {
        format!("{}{}", PEER_PREFIX, no)
    }
}

impl Runnable for Peer {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.context.config().channel_buffer());
        let transfer_id = Self::get_transfer_id(self.no);
        self.emitter.register(transfer_id, send.clone());

        trace!("开始监听数据 peer_no:{}\taddr: {}", self.no, self.addr);
        let reason: ExitReason;
        let cancel = CancellationToken::new();
        let recv_handle = tokio::spawn(
            Recv {
                no: self.no,
                reader: self.reader.take(),
                cancel_token: cancel.clone(),
                addr: self.addr.clone(),
                sender: send,
            }
            .run(),
        ); // 让 recv 独立一个异步任务，保证读取不会被任务处理阻塞

        loop {
            tokio::select! {
                _ = self.context.cancel_token() => {
                    debug!("peer {} cancelled", self.no);
                    reason = ExitReason::Normal;
                    break;
                }
                result = recv.recv() => {
                    match result {
                        Some(cmd) => {
                            let cmd: command::Command = cmd.instance();
                            if let command::Command::Exit(exit) = cmd {
                                reason = exit.reason;
                                break;
                            }
                            if let Err(e) = cmd.handle(&mut self).await {
                                error!("处理指令出现错误\t{}", e);
                                reason = ExitReason::Exception;
                                break;
                            }
                        }
                        None => {
                            // 发送通道全部关闭（emitter 已被销毁），buffer 没有消息，peer 退出
                            reason = ExitReason::Normal;
                            break;
                        }
                    }
                }
            }
        }

        cancel.cancel();
        recv_handle.await.unwrap();

        // 归还未下完的分块
        self.context
            .give_back_download_piece(self.no, self.downloading_pieces);
        self.context.peer_exit(self.no, reason).await;
        debug!("peer [{}] 已退出！", self.no);
    }
}

struct Recv {
    no: u64,
    reader: Option<OwnedReadHalf>,
    cancel_token: CancellationToken,
    addr: Arc<SocketAddr>,
    sender: Sender<TransferPtr>,
}

impl Runnable for Recv {
    async fn run(mut self) {
        let mut reader = self.reader.take().unwrap();
        let mut bt_resp = BtResp::new(&mut reader, &self.addr);
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    debug!("recv [{}] 退出", self.no);
                    break;
                }
                result = &mut bt_resp => {
                    match result {
                        Some((msg_type, buf)) => {
                            let buf_len = buf.len() as u64;
                            self.sender.send(PeerTransfer {
                                msg_type,
                                buf,
                                read_size: 5 + buf_len
                            }.into()).await.unwrap()
                        },
                        None => {
                            warn!("断开了链接，终止 {} - {} 的数据监听", self.no, &self.addr);
                            let reason = ExitReason::Exception;
                            self.sender.send(Exit{ reason }.into()).await.unwrap();
                            break;
                        }
                    }
                    bt_resp = BtResp::new(&mut reader, &self.addr);
                }
            }
        }

        debug!("recv [{}] 已退出！", self.no);
    }
}
