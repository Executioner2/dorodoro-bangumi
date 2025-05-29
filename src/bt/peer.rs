//! 端到端通信。
//!
//! 协议格式：
//! `长度（4字节） | 消息类型（1字节） | 数据（长度-5字节）`
//!
//! 长度的需要加上消息类型的 1 字节。

pub mod command;
mod error;
pub mod future;
mod listener;
mod rate_control;

use crate::bytes::Bytes2Int;
use crate::command::CommandHandler;
use crate::config::Config;
use crate::core::protocol::{
    BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN,
};
use crate::core::runtime::Runnable;
use crate::emitter::Emitter;
use crate::emitter::constant::PEER_PREFIX;
use crate::emitter::transfer::TransferPtr;
use crate::peer::command::{PieceCheckoutFailed, PieceWriteFailed};
use crate::peer::error::Error;
use crate::peer::error::Error::{
    BitfieldError, HandshakeError, ResponseDataIncomplete, TryFromError,
};
use crate::peer::listener::{ReadFuture, WriteFuture};
use crate::peer::rate_control::probe::{Dashbord, Probe};
use crate::peer::rate_control::{PacketSend, RateControl};
use crate::peer_manager::gasket::{ExitReason, GasketContext};
use crate::store::Store;
use crate::timer::CountdownTimer;
use crate::torrent::TorrentArc;
use crate::util;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::{Bytes, BytesMut};
use error::Result;
use fnv::FnvHashMap;
use std::cmp::min;
use std::mem;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Sender, channel};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

const MSS: u32 = 17;

/// Peer 通信的消息类型
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
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

unsafe impl Send for MsgType {}
unsafe impl Sync for MsgType {}

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

#[derive(Debug)]
pub struct Piece {
    /// 完成的偏移，如果重新开启任务，下一次从这个地方开始请求
    block_offset: u32,

    /// 分片位图，完成了的 block 置为 1
    bitmap: Vec<u8>,

    // 位图的实际长度，即去除填充 0 的部分
    // len: usize,
    /// 分块大小
    block_size: u32,

    /// 分片大小
    piece_size: u32,

    /// 是否完成
    finish: bool,
}

impl Piece {
    fn new(piece_size: u32, block_size: u32) -> Self {
        let share = Self::block_offset_idx(piece_size, block_size);
        let len = (share + 7) >> 3;
        let bitmap = vec![0u8; len as usize];
        Self {
            block_offset: 0,
            bitmap,
            // len: ((len * 8) - (share & 7)) as usize,
            block_size,
            piece_size,
            finish: false,
        }
    }
    
    fn block_offset_idx(block_offset: u32, block_size: u32) -> u32 {
        (block_offset + block_size - 1) / block_size
    }

    /// 新增完成的分块
    fn add_finish(&mut self, block_offset: u32) {
        if self.finish {
            return;
        }
        let bo_idx = Self::block_offset_idx(block_offset, self.block_size);
        let (idx, offset) = util::bytes::bitmap_offset(bo_idx as usize);
        self.bitmap.get_mut(idx).map(|val| *val |= offset);

        // 更新连续块偏移
        let bo_idx = Self::block_offset_idx(self.block_offset, self.block_size);
        let (idx, mut offset) = util::bytes::bitmap_offset(bo_idx as usize);
        'first_loop: for i in idx..self.bitmap.len() {
            let mut k = offset;
            while k != 0 {
                if self.bitmap[i] & k == 0 {
                    break 'first_loop;
                }
                self.block_offset += self.block_size;
                k >>= 1;
            }
            offset = 1 << 7
        }

        self.finish = self.block_offset >= self.piece_size;
    }

    pub fn is_finish(&self) -> bool {
        self.finish
    }

    pub fn block_offset(&self) -> u32 {
        self.block_offset
    }
}

#[derive(Clone)]
struct Writer {
    inner: Option<Sender<Vec<u8>>>,
}

impl Writer {
    fn new() -> Self {
        Self { inner: None }
    }

    fn set_sender(&mut self, sender: Sender<Vec<u8>>) {
        self.inner = Some(sender)
    }
}

impl Deref for Writer {
    type Target = Sender<Vec<u8>>;

    fn deref(&self) -> &Self::Target {
        // 安全：因为内部保证在使用 writer 时，
        // inner 已经有值了
        self.inner.as_ref().unwrap()
    }
}

impl DerefMut for Writer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // 安全：因为内部保证在使用 writer 时，
        // inner 已经有值了
        self.inner.as_mut().unwrap()
    }
}

#[derive(Clone)]
pub struct PeerContext {
    /// 编号
    no: u64,

    /// gasket 共享上下文
    gc: GasketContext,

    /// 当前 peer 的状态
    status: Arc<Mutex<Status>>,

    /// wrtier
    writer: Writer,

    /// 下载速率仪表盘
    dashbord: Option<Dashbord>,
}

impl PeerContext {
    fn new(no: u64, context: GasketContext) -> Self {
        Self {
            no,
            gc: context,
            status: Arc::new(Mutex::new(Status::Choke)),
            writer: Writer::new(),
            dashbord: None,
        }
    }

    fn torrent(&self) -> TorrentArc {
        self.gc.torrent()
    }

    fn config(&self) -> Config {
        self.gc.config()
    }

    fn peer_id(&self) -> &[u8; 20] {
        self.gc.peer_id()
    }

    pub fn save_path(&self) -> &PathBuf {
        self.gc.save_path()
    }

    pub fn bytefield(&self) -> &Arc<tokio::sync::Mutex<BytesMut>> {
        self.gc.bytefield()
    }

    pub fn recv_bytes(&self) -> u64 {
        self.dashbord.as_ref().unwrap().acked_bytes()
    }

    pub fn bw(&self) -> u64 {
        self.dashbord.as_ref().unwrap().bw()
    }
}

pub struct Peer {
    /// peer context
    ctx: PeerContext,

    /// 对端地址
    addr: Arc<SocketAddr>,

    /// 对端的 peer id
    opposite_peer_id: Option<[u8; 20]>,

    /// 对端的状态
    opposite_peer_status: Status,

    /// 指令发射器
    emitter: Emitter,

    /// 存储处理
    store: Store,

    /// 对端拥有的分块
    opposite_peer_bitfield: Option<BytesMut>,

    /// 收到响应的分块数据，收到的块（block）可能是乱序的
    /// 因此只有在前面的块（block）都收到后，才会更新这个
    /// 分片（piece）的 block offset
    ///
    /// (分块下标 偏移位置)
    response_pieces: FnvHashMap<u32, Piece>,

    /// 请求的分块
    request_pieces: FnvHashMap<u32, u32>,

    /// 下载速率仪表盘
    dashbord: Option<Dashbord>,

    /// 发送倒计时
    ct: CountdownTimer,
}

impl Peer {
    pub fn new(
        no: u64,
        addr: Arc<SocketAddr>,
        context: GasketContext,
        emitter: Emitter,
        store: Store,
    ) -> Self {
        let context = PeerContext::new(no, context);

        Peer {
            ctx: context,
            addr,
            opposite_peer_id: None,
            opposite_peer_status: Status::Choke,
            emitter,
            store,
            opposite_peer_bitfield: None,
            response_pieces: FnvHashMap::default(),
            request_pieces: FnvHashMap::default(),
            dashbord: None,
            ct: CountdownTimer::new(Duration::from_secs(0)),
        }
    }

    fn no(&self) -> u64 {
        self.ctx.no
    }

    /// 握手
    async fn handshake(&mut self, stream: &mut TcpStream) -> Result<()> {
        trace!("发送握手信息 peer_no: {}", self.no());
        let mut bytes =
            Vec::with_capacity(1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN);
        let info_hash = &self.ctx.torrent().info_hash;
        WriteBytesExt::write_u8(&mut bytes, BIT_TORRENT_PROTOCOL_LEN)?;
        std::io::Write::write(&mut bytes, BIT_TORRENT_PROTOCOL)?;
        WriteBytesExt::write_u64::<BigEndian>(&mut bytes, 0u64)?;
        std::io::Write::write(&mut bytes, info_hash)?;
        std::io::Write::write(&mut bytes, self.ctx.peer_id())?;
        stream.write_all(&bytes).await?;

        // 等待握手信息
        stream.readable().await?;

        let mut handshake_resp = vec![0u8; bytes.len()];
        let size = stream.read(&mut handshake_resp).await?;
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

        Ok(())
    }

    /// 告诉对方，自己拥有的分片
    async fn bitfield(&self) -> Result<()> {
        let bitfield = &*self.ctx.bytefield().lock().await;
        let mut bytes = Vec::with_capacity(5 + bitfield.len());
        WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 1 + bitfield.len() as u32)?;
        WriteBytesExt::write_u8(&mut bytes, MsgType::Bitfield as u8)?;
        std::io::Write::write(&mut bytes, &bitfield[..])?;
        self.ctx.writer.send(bytes).await?;
        Ok(())
    }

    /// 告诉对方，允许发起请求
    #[allow(dead_code)]
    async fn unchoke(&self) -> Result<()> {
        let mut bytes = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 1)?;
        WriteBytesExt::write_u8(&mut bytes, MsgType::UnChoke as u8)?;
        self.ctx.writer.send(bytes).await?;
        Ok(())
    }

    /// 告诉对方，对他拥有的资源很感兴趣
    async fn interested(&self) -> Result<()> {
        let mut bytes = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 1)?;
        WriteBytesExt::write_u8(&mut bytes, MsgType::Interested as u8)?;
        self.ctx.writer.send(bytes).await?;
        Ok(())
    }

    /// 响应事件处理
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

    /// 请求块
    async fn request_block(&mut self, piece_index: u32) -> Result<()> {
        let block_size = self.ctx.config().block_size();
        let piece_length = Self::get_piece_length(&self.ctx.torrent(), piece_index);
        let offset = *self.request_pieces.get(&piece_index).unwrap_or(&0);

        if offset < piece_length {
            trace!(
                "peer_no [{}] 请求下载分块 {} - {}",
                self.ctx.no, piece_index, offset
            );
            let mut req = Vec::with_capacity(13);
            WriteBytesExt::write_u32::<BigEndian>(&mut req, 13)?;
            WriteBytesExt::write_u8(&mut req, MsgType::Request as u8)?;
            WriteBytesExt::write_u32::<BigEndian>(&mut req, piece_index)?;
            WriteBytesExt::write_u32::<BigEndian>(&mut req, offset)?;
            WriteBytesExt::write_u32::<BigEndian>(
                &mut req,
                min(piece_length - offset, block_size),
            )?;
            self.send_mark(MSS);
            self.ctx.writer.send(req).await?;
            self.request_pieces.insert(piece_index, offset + block_size);
        } else {
            self.request_pieces.remove(&piece_index);
        }

        Ok(())
    }

    /// 获取发送窗口
    #[inline]
    fn cwnd(&self) -> u32 {
        self.dashbord.as_ref().unwrap().cwnd()
    }

    /// 获取流动数据量
    #[inline]
    fn inflight(&self) -> u32 {
        self.dashbord.as_ref().unwrap().inflight()
    }

    /// 发送标记
    #[inline]
    fn send_mark(&mut self, packet_size: u32) {
        self.dashbord.as_ref().unwrap().send(packet_size);
    }

    /// 请求分片
    async fn request_piece(&mut self) -> Result<()> {
        while self.inflight() <= self.cwnd() {
            if let Some(idx) = self.try_find_downloadable_pices().await? {
                self.ct.tokio_wait_reamining().await;
                self.request_block(idx).await?;
            } else {
                info!("没有了，那么等待响应的分片都响应完成");
                break;
            }
        }
        Ok(())
    }

    fn is_can_be_download(&self) -> bool {
        match self.ctx.status.lock() {
            Ok(status) => *status == Status::UnChoke,
            Err(e) => **e.get_ref() == Status::UnChoke,
        }
    }

    /// 尝试寻找可以下载的分块进行下载
    async fn try_find_downloadable_pices(&mut self) -> Result<Option<u32>> {
        if !self.is_can_be_download() {
            return Ok(None);
        }

        // 先拿未请求完成的
        if let Some(idx) = self.request_pieces.keys().take(1).next() {
            return Ok(Some(*idx));
        }

        // 下载暂停的分块
        if let Some((idx, offset)) = self.ctx.gc.apply_download_pasue_piece() {
            self.request_pieces.insert(idx, offset);
            return Ok(Some(idx));
        }

        while let Some((piece_index, block_offset)) =
            self.ctx.gc.apply_download_piece(self.ctx.no).await
        {
            let (idx, offset) = util::bytes::bitmap_offset(piece_index as usize);
            if let Some(true) = self
                .opposite_peer_bitfield
                .as_ref()
                .map(|bytes| (bytes[idx] & offset) != 0)
            {
                self.request_pieces.insert(piece_index, block_offset);
                return Ok(Some(piece_index));
            }
        }

        // 没有分块可以下载了，告诉 gasket
        if self.response_pieces.is_empty() {
            self.ctx.gc.report_no_downloadable_piece(self.ctx.no).await;
        }
        Ok(None)
    }

    fn update_response_pieces(&mut self, piece_index: u32, block_offset: u32) -> (bool, u32) {
        let block_size = self.ctx.config().block_size();
        let piece_length = Self::get_piece_length(&self.ctx.torrent(), piece_index);
        let piece = self
            .response_pieces
            .entry(piece_index)
            .or_insert(Piece::new(piece_length, block_size));
        piece.add_finish(block_offset);
        (piece.is_finish(), piece.block_offset)
    }

    fn get_piece_length(torrent: &TorrentArc, piece_index: u32) -> u32 {
        let piece_length = torrent.info.piece_length;
        let resource_length = torrent.info.length;
        piece_length.min(resource_length.saturating_sub(piece_index as u64 * piece_length)) as u32
    }

    pub fn get_transfer_id(no: u64) -> String {
        format!("{}{}", PEER_PREFIX, no)
    }

    /// 不让我们请求数据了
    async fn handle_choke(&mut self) -> Result<()> {
        info!("peer_no [{}] 对端不让我们请求下载数据", self.no());
        match self.ctx.status.lock().as_mut() {
            Ok(status) => **status = Status::Choke,
            Err(e) => **e.get_mut() = Status::Choke,
        }
        Ok(())
    }

    /// 告知可以交换数据了
    async fn handle_un_choke(&mut self) -> Result<()> {
        info!("peer_no [{}] 对端告诉我们可以下载数据", self.no());
        match self.ctx.status.lock().as_mut() {
            Ok(status) => **status = Status::UnChoke,
            Err(e) => **e.get_mut() = Status::UnChoke,
        }
        self.request_piece().await?;
        Ok(())
    }

    /// 对我们的数据感兴趣
    async fn handle_interested(&mut self) -> Result<()> {
        trace!("对端对我们的数据感兴趣，那我们就允许他下载");
        self.opposite_peer_status = Status::UnChoke;
        let mut req = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut req, 1)?;
        WriteBytesExt::write_u8(&mut req, MsgType::UnChoke as u8)?;
        self.ctx.writer.send(req).await?;
        Ok(())
    }

    /// 对我们的数据不再感兴趣了
    async fn handle_not_interested(&mut self) -> Result<()> {
        trace!("对端对我们的数据不再感兴趣了，那就告诉他你别下载了！");
        self.opposite_peer_status = Status::Choke;
        let mut req = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut req, 1)?;
        WriteBytesExt::write_u8(&mut req, MsgType::Choke as u8)?;
        self.ctx.writer.send(req).await?;
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
        self.request_piece().await?;

        Ok(())
    }

    /// 对端告诉我们他有哪些分块可以下载
    async fn handle_bitfield(&mut self, bytes: Bytes) -> Result<()> {
        trace!("对端告诉我们他有哪些分块可以下载: {:?}", &bytes[..]);
        let torrent = self.ctx.torrent();
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
        self.request_piece().await?;

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
        trace!("peer_no [{}] 收到了对端发来的数据", self.ctx.no);
        if bytes.len() < 8 {
            return Err(ResponseDataIncomplete);
        }

        let piece_index = u32::from_be_slice(&bytes[0..4]);
        let block_offset = u32::from_be_slice(&bytes[4..8]);
        let block_data = bytes.split_off(8);
        let block_data_len = block_data.len();

        self.request_piece().await?;
        let (over, cts_bo) = self.update_response_pieces(piece_index, block_offset);

        self.write_file(piece_index, block_offset, block_data, over);

        // 上报给 gasket
        self.ctx.gc.reported_download(piece_index, cts_bo, block_data_len as u64);
        self.try_find_downloadable_pices().await?;

        Ok(())
    }

    /// 对端撤回了刚刚请求的那个分块
    async fn handle_cancel(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端撤回了刚刚请求的那个分块");
        Ok(())
    }

    /// 写入文件
    fn write_file(&self, piece_index: u32, block_offset: u32, mut block_data: Bytes, over: bool) {
        let context = self.ctx.gc.clone();
        let store = self.store.clone();
        let sender = self
            .emitter
            .get(&Self::get_transfer_id(self.ctx.no))
            .unwrap();

        tokio::spawn(async move {
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
                    let _ = sender.send(cmd).await;
                }
            }

            // 校验分块 hash
            if over {
                Self::checkout(context, piece_index, sender, store).await;
            }
        });
    }

    /// 校验分块
    async fn checkout(
        context: GasketContext,
        piece_index: u32,
        sender: Sender<TransferPtr>,
        store: Store,
    ) {
        let read_length = Self::get_piece_length(&context.torrent(), piece_index);

        trace!("开始校验 {} 个分块", piece_index);
        let torrent = context.torrent();
        let path = context.save_path();
        let hash = &torrent.info.pieces[piece_index as usize * 20..(piece_index as usize + 1) * 20];
        let list = torrent.find_file_of_piece_index(path, piece_index, 0, read_length as usize);
        match store.checkout(list.clone(), hash).await {
            Ok(false) | Err(_) => {
                error!("就是false了，参与计算的文件: {:?}", list);
                let cmd = PieceCheckoutFailed { piece_index }.into();
                let _ = sender.send(cmd).await; // 有可能第一次发送错误就关闭了 peer，所以不必理会发送错误
            }
            _ => {
                debug!("第 {} 个分块校验通过", piece_index);
                context.reported_piece_finished(piece_index).await;
            }
        }
    }

    /// 连接对端 peer
    async fn connection(&self) -> Result<TcpStream> {
        let timeout = self.ctx.config().peer_connection_timeout();
        match tokio::time::timeout(timeout, TcpStream::connect(&*self.addr)).await {
            Ok(Ok(stream)) => Ok(stream),
            _ => Err(Error::TimeoutError),
        }
    }

    /// 启动套接字监听
    fn start_listener(
        &mut self,
        stream: TcpStream,
        peer_send: Sender<TransferPtr>,
        cancel: CancellationToken,
    ) -> (JoinHandle<()>, JoinHandle<()>) {
        let (reader, writer) = stream.into_split();
        let (send, recv) = channel(self.ctx.config().channel_buffer());
        let probe = Probe::new();

        self.dashbord = Some(probe.dashbord());

        let write_future_handle = tokio::spawn(
            WriteFuture {
                no: self.ctx.no,
                writer,
                cancel_token: cancel.clone(),
                addr: self.addr.clone(),
                peer_sender: peer_send.clone(),
                recv,
            }
            .run(),
        );

        let read_future_handle = tokio::spawn(
            ReadFuture {
                no: self.ctx.no,
                reader,
                cancel_token: cancel,
                addr: self.addr.clone(),
                peer_sender: peer_send.clone(),
                rc: probe,
            }
            .run(),
        );

        self.ctx.writer.set_sender(send);

        (write_future_handle, read_future_handle)
    }

    /// 启动 peer
    async fn start(
        &mut self,
        send: Sender<TransferPtr>,
    ) -> Result<(CancellationToken, Vec<JoinHandle<()>>)> {
        let mut stream = self.connection().await?;

        self.handshake(&mut stream).await?;
        let cancel = CancellationToken::new();

        let res = self.start_listener(stream, send.clone(), cancel.clone());

        self.bitfield().await?;
        self.interested().await?;

        Ok((cancel, vec![res.0, res.1]))
    }
}

impl Runnable for Peer {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.ctx.config().channel_buffer());
        let transfer_id = Self::get_transfer_id(self.ctx.no);
        self.emitter.register(transfer_id, send.clone());

        let future_token = match self.start(send).await {
            Ok(future_token) => future_token,
            Err(_) => {
                self.ctx
                    .gc
                    .peer_exit(self.ctx.no, ExitReason::Exception)
                    .await;
                return;
            }
        };

        let reason: ExitReason;
        loop {
            tokio::select! {
                _ = self.ctx.gc.cancel_token() => {
                    debug!("peer {} cancelled", self.ctx.no);
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

        future_token.0.cancel();
        for handle in future_token.1 {
            handle.await.unwrap();
        }

        // 归还未下完的分块
        self.ctx
            .gc
            .give_back_download_piece(self.ctx.no, self.response_pieces);
        self.emitter.remove(&Self::get_transfer_id(self.ctx.no));
        self.ctx.gc.peer_exit(self.ctx.no, reason).await;
        debug!("peer [{}] 已退出！", self.ctx.no);
    }
}
