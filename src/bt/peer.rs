//! 端到端通信。
//!
//! 协议格式：
//! `长度（4字节） | 消息类型（1字节） | 数据（长度-5字节）`
//!
//! 长度的需要加上消息类型的 1 字节。

pub mod command;
mod error;
pub mod peer_resp;
mod listener;
pub mod rate_control;
pub mod reserved;

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
use crate::peer::error::Error::{BitfieldError, HandshakeError, ResponseDataIncomplete, ResponsePieceError, TryFromError};
use crate::peer::listener::{ReadFuture, WriteFuture};
use crate::peer::rate_control::probe::{Dashbord, Probe};
use crate::peer::rate_control::{PacketSend, RateControl};
use crate::peer_manager::gasket::{ExitReason, GasketContext};
use crate::store::Store;
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
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{channel, Sender};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Instant, Interval};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};
use crate::bt::pe_crypto;
use crate::bt::pe_crypto::CryptoProvide;
use crate::bt::socket::TcpStreamExt;

const MSS: u32 = 17;

/// 每隔 1 分钟发送一次心跳包
const KEEP_ALIVE: Duration = Duration::from_secs(60);

/// Peer 通信的消息类型
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum MsgType {
    
    // ===========================================================================
    // Core 
    // 详情见：[bep_0003](https://www.bittorrent.org/beps/bep_0003.html)
    // ===========================================================================
    
    /// 我现在还不想接收你的请求        
    /// 
    /// 格式：`length:u32 | 0:u8`
    Choke = 0,

    /// 你现在可以向我发起请求了   
    ///     
    /// 格式：`length:u32 | 1:u8`
    UnChoke = 1,

    /// 感兴趣，期望可以允许我发起请求    
    /// 
    /// 格式：`length:u32 | 2:u8`
    Interested = 2,

    /// 我已经对你不感兴趣了，你可以不再理我（choke）告诉对方自己已经下载完他的资源了   
    ///     
    /// 格式：`length:u32 | 3:u8`
    NotInterested = 3,

    /// 我新增了这个分块，你要不要呀      
    /// 
    /// 格式：`length:u32 | 4:u8 | piece_index:u32`
    Have = 4,

    /// 你不要告诉别人，我偷偷给你说，我拥有这些分块。一般在建立完链接后发送         
    ///  
    /// 格式：`length:u32 | 5:u8 | bitfield:&[u8]`
    Bitfield = 5,

    /// 请给我这个分块     
    /// 
    /// 格式：`length:u32 | 6:u8 | piece_index:u32 | block_offset:u32 | block_length:u32`
    Request = 6,

    /// 好的，给你这个分块     
    ///  
    /// 格式：`length:u32 | 7:u8 | piece_index:u32 | block_offset:u32 | block_data:&[u8]`
    Piece = 7,

    /// 噢撤回，我不需要这个分块了    
    ///   
    /// 格式：`length:u32 | 8:u8 | piece_index:u32 | block_offset:u32`
    Cancel = 8,

    // ===========================================================================
    // DHT Extension    
    // 详情见：[bep_0005](https://www.bittorrent.org/beps/bep_0005.html)
    // 握手时，通过设置扩展位（HANDSHAKE_DHT_PROTOCOL）开启
    // ===========================================================================
    
    /// DHT 访问端口     
    ///   
    /// 格式：`length:u32 | 9:u8 | dht_port:u16`
    Port = 9,

    // ===========================================================================
    // Fast Extensions
    // 详情见：[bep_0006](https://www.bittorrent.org/beps/bep_0006.html)
    // 握手时，通过设置扩展位（HANDSHAKE_FAST_EXTENSION）开启
    // ===========================================================================

    /// 建议请求的分片     
    /// 
    /// 格式：`length:u32 | 13:u8 | piece_index:u32`
    Suggest = 13,
    
    /// 拥有所有的分片   
    ///  
    /// 格式：`length:u32 | 14:u8`
    HaveAll = 14,
    
    /// 一个分片都没有
    ///
    /// 格式：`length:u32 | 15:u8`   
    HaveNone = 15,
    
    /// 拒绝请求的分片
    /// 
    /// 格式：`length:u32 | 16:u8 | piece_index:u32 | block_offset:u32 | block_length:u32`
    RejectRequest = 16,
    
    /// 允许快速下载
    /// 
    /// 格式：`length:u32 | 17:u8 | piece_index:u32`
    AllowedFast = 17,

    // ===========================================================================
    // Additional IDs used in deployed clients  
    // 详情见：[bep_0010](https://www.bittorrent.org/beps/bep_0010.html)
    // 握手时，通过设置扩展位（HANDSHAKE_EXTENSION_PROTOCOL）开启
    // ===========================================================================
    
    /// 扩展协议
    /// 
    /// 格式：`length:u32 | 20:u8 | extended_id:u64 | extended_data:bencode`
    LTEPHandshake = 20,

    // ===========================================================================
    // Hash Transfer Protocol  
    // 详情见：[bep_0009](https://www.bittorrent.org/beps/bep_0009.html)
    // ===========================================================================
    
    /// 请求 metadata
    /// 
    /// 格式：`length:u32 | 21:u8 | hash:bencode`
    HashRequest = 21,

    /// 响应 metadata
    /// 
    /// 格式：`length:u32 | 22:u8 | metadata:bencode`
    Hashes = 22,

    /// 拒绝响应 metadata
    /// 
    /// 格式：`length:u32 | 23:u8 | metadata:bencode`
    HashReject = 23,
}

unsafe impl Send for MsgType {}
unsafe impl Sync for MsgType {}

impl TryFrom<u8> for MsgType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value { 
            0..=9 | 13..=17 | 20..=23 => Ok(unsafe { mem::transmute(value) }),
            _ => Err(TryFromError),
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

    /// 分块大小
    block_size: u32,

    /// 分片大小
    piece_length: u32,

    /// 是否完成
    finish: bool,
}

impl Piece {
    fn new(block_offset: u32, piece_length: u32, block_size: u32) -> Self {
        let share = Self::block_offset_idx(piece_length, block_size);
        let len = (share + 7) >> 3;
        let bitmap = vec![0u8; len as usize];
        Self {
            block_offset, // 不需要把 block_offset 之前到填充上 1，之后的判断都是从 block_offset 开始
            bitmap,
            block_size,
            piece_length,
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

        self.finish = self.block_offset >= self.piece_length;
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
    
    /// wrtier
    writer: Writer,

    /// 下载速率仪表盘
    dashbord: Dashbord,
}

impl PeerContext {
    fn new(no: u64, context: GasketContext, dashbord: Dashbord) -> Self {
        Self {
            no,
            gc: context,
            writer: Writer::new(),
            dashbord,
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

    pub fn bytefield(&self) -> &Arc<Mutex<BytesMut>> {
        self.gc.bytefield()
    }

    pub fn recv_bytes(&self) -> u64 {
        self.dashbord.acked_bytes()
    }

    pub fn bw(&self) -> u64 {
        self.dashbord.bw()
    }
}

pub struct Peer {
    /// peer context
    ctx: PeerContext,

    /// 状态
    status: Status,

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
    opposite_peer_bitfield: Arc<Mutex<BytesMut>>,

    /// 收到响应的分块数据，收到的块（block）可能是乱序的
    /// 因此只有在前面的块（block）都收到后，才会更新这个
    /// 分片（piece）的 block offset
    ///
    /// (分块下标 偏移位置)
    response_pieces: FnvHashMap<u32, Piece>,

    /// 请求的分块
    request_pieces: FnvHashMap<u32, u32>,
    
    /// 下载速率仪表盘
    dashbord: Dashbord,

    /// 心跳包间隔
    keep_alive_interval: Interval,
}

impl Peer {
    pub fn new(
        no: u64,
        addr: Arc<SocketAddr>,
        context: GasketContext,
        emitter: Emitter,
        store: Store,
        dashbord: Dashbord,
    ) -> Self {
        let context = PeerContext::new(no, context, dashbord.clone());
        let start = Instant::now() + KEEP_ALIVE;
        let keep_alive_interval = tokio::time::interval_at(start, KEEP_ALIVE);

        Peer {
            ctx: context,
            status: Status::Choke,
            addr,
            opposite_peer_id: None,
            opposite_peer_status: Status::Choke,
            emitter,
            store,
            opposite_peer_bitfield: Arc::new(Mutex::new(BytesMut::new())),
            response_pieces: FnvHashMap::default(),
            request_pieces: FnvHashMap::default(),
            dashbord,
            keep_alive_interval
        }
    }
    
    pub fn dashbord(&self) -> Dashbord {
        self.dashbord.clone()
    }

    fn no(&self) -> u64 {
        self.ctx.no
    }
    
    /// 加密握手
    async fn crypto_handshake(&mut self, stream: TcpStream) -> Result<TcpStreamExt> {
        trace!("加密握手 peer_no: {}", self.no());
        let tse= pe_crypto::init_handshake(stream, &self.ctx.torrent().info_hash, CryptoProvide::Rc4).await?;
        Ok(tse)
    }
    
    /// 握手
    async fn handshake(&mut self, stream: &mut TcpStreamExt) -> Result<()> {
        trace!("发送握手信息 peer_no: {}", self.no());
        let mut bytes =
            Vec::with_capacity(1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN);
        let info_hash = &self.ctx.torrent().info_hash;
        WriteBytesExt::write_u8(&mut bytes, BIT_TORRENT_PROTOCOL_LEN)?;
        std::io::Write::write(&mut bytes, BIT_TORRENT_PROTOCOL)?;
        let ext = reserved::LTEP | reserved::DHT;
        WriteBytesExt::write_u64::<BigEndian>(&mut bytes, ext)?;
        std::io::Write::write(&mut bytes, info_hash)?;
        std::io::Write::write(&mut bytes, self.ctx.peer_id())?;
        stream.write_all(&bytes).await?;

        // 等待握手信息
        stream.readable().await?;

        let mut handshake_resp = vec![0u8; bytes.len()];
        let size = stream.read(&mut handshake_resp).await?;
        if size != bytes.len() {
            error!("响应数据长度与预期不符 [{}]\t[{}]", size, bytes.len());
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
        self.dashbord.cwnd()
    }

    /// 获取流动数据量
    #[inline]
    fn inflight(&self) -> u32 {
        self.dashbord.inflight()
    }

    /// 发送标记
    #[inline]
    fn send_mark(&mut self, packet_size: u32) {
        self.dashbord.send(packet_size);
    }

    /// 请求分片
    async fn request_piece(&mut self) -> Result<()> {
        while self.inflight() <= self.cwnd() {
            if let Some(idx) = self.try_find_downloadable_pices().await? {
                self.request_block(idx).await?;
            } else {
                trace!("没有了，那么等待响应的分片都响应完成");
                break;
            }
        }
        Ok(())
    }

    fn is_can_be_download(&self) -> bool {
        self.status == Status::UnChoke
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

        for piece_index in 0..self.ctx.torrent().piece_num() {
            let (idx, offset) = util::bytes::bitmap_offset(piece_index);
            
            let item = self.opposite_peer_bitfield
                .lock().await.get(idx) 
                .map(|value| value & offset != 0);
            if !matches!(item, Some(true))  {
                continue;
            }

            if let Some((piece_index, block_offset)) = 
                self.ctx.gc.apply_download_piece(self.ctx.no, piece_index as u32).await {
                
                self.insert_response_pieces(piece_index, block_offset);
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

    fn insert_response_pieces(&mut self, piece_index: u32, block_offset: u32) {
        let bs = self.ctx.config().block_size();
        let pl = Self::get_piece_length(&self.ctx.torrent(), piece_index);
        self.response_pieces.insert(piece_index, Piece::new(block_offset, pl, bs));
    }

    fn update_response_pieces(&mut self, piece_index: u32, block_offset: u32) -> Result<(bool, u32)> {
        let piece = self.response_pieces.get_mut(&piece_index)
            .ok_or(ResponsePieceError)?;
        piece.add_finish(block_offset);
        let res = (piece.is_finish(), piece.block_offset);
        if res.0 { self.response_pieces.remove(&piece_index); }
        Ok(res)
    }

    fn get_piece_length(torrent: &TorrentArc, piece_index: u32) -> u32 {
        let piece_length = torrent.info.piece_length;
        let resource_length = torrent.info.length;
        piece_length.min(resource_length.saturating_sub(piece_index as u64 * piece_length)) as u32
    }

    pub fn get_transfer_id(no: u64) -> String {
        format!("{}{}", PEER_PREFIX, no)
    }
    
    /// 释放正在下载的分片
    async fn free_download_piece(&mut self, pieces: &Vec<u32>) {
        for piece_index in pieces {
            self.request_pieces.remove(piece_index);
            if let Some(peer) = self.response_pieces.remove(piece_index) {
                self.ctx.gc.give_back_download_pieces(self.no(), vec![(*piece_index, peer)]);
            }
        }
        if self.response_pieces.is_empty() {
            self.ctx.gc.report_no_downloadable_piece(self.no()).await;
        }
    }

    /// 发送心跳包
    async fn send_heartbeat(&self) -> Result<()> {
        // 一般来说不是很管用，暂时不要发，发了会被中断链接
        // let bytes = vec![0u8];
        // self.ctx.writer.send(bytes).await?;
        Ok(())
    }

    /// 收到心跳包，处理心跳包
    async fn handle_heartbeat(&self) -> Result<()> {
        // 暂时是啥也不做
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
            _ => (),
        };
        Ok(res)
    }

    /// 不让我们请求数据了
    async fn handle_choke(&mut self) -> Result<()> {
        info!("peer_no [{}] 对端不让我们请求下载数据", self.no());
        self.status = Status::Choke;
        Ok(())
    }

    /// 告知可以交换数据了
    async fn handle_un_choke(&mut self) -> Result<()> {
        trace!("peer_no [{}] 对端告诉我们可以下载数据", self.no());
        self.status = Status::UnChoke;
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

        self.opposite_peer_bitfield.
            lock().await.get_mut(idx)
            .map(|bytes| *bytes |= offset);
        self.request_piece().await?;

        Ok(())
    }

    /// 对端告诉我们他有哪些分块可以下载
    async fn handle_bitfield(&mut self, bytes: Bytes) -> Result<()> {
        trace!("对端告诉我们他有哪些分块可以下载: {:?}", &bytes[..]);
        let torrent = self.ctx.torrent();
        let bit_len = torrent.bitfield_len();
        if bytes.len() != bit_len {
            warn!(
                "远端给到的 bitfield 长度和期望的不一致，期望的: {}\t实际的bytes: {:?}",
                bit_len,
                &bytes[..]
            );
            return Err(BitfieldError);
        }

        let bitfield = Arc::new(Mutex::new(BytesMut::from(bytes)));
        self.opposite_peer_bitfield = bitfield.clone();
        self.ctx.gc.reported_bitfield(self.no(), bitfield);
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
        
        // 交给了其他 peer 下载这个 piece。见 free_download_piece
        if !self.response_pieces.contains_key(&piece_index) {
            return Ok(());
        }
        
        let block_offset = u32::from_be_slice(&bytes[4..8]);
        let block_data = bytes.split_off(8);
        let block_data_len = block_data.len();

        self.request_piece().await?;
        let (over, cts_bo) = self.update_response_pieces(piece_index, block_offset)?;

        // fixme - 正式记得删掉下面
        // if over {
        //     self.ctx.gc.reported_piece_finished(self.no(), piece_index).await;
        // }
        
        // fixme - 正式记得取消下面的注释
        self.write_file(piece_index, block_offset, block_data, over);

        // 上报给 gasket
        self.ctx.gc.reported_download(piece_index, cts_bo, block_data_len as u64);

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
        let peer_no = self.no();
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
                Self::checkout(peer_no, context, piece_index, sender, store).await;
            }
        });
    }

    /// 校验分块
    async fn checkout(
        peer_no: u64,
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
                trace!("第 {} 个分块校验通过", piece_index);
                context.reported_piece_finished(peer_no, piece_index).await;
            }
        }
    }

    /// 连接对端 peer
    async fn connection(&self) -> Result<TcpStream> {
        let timeout = self.ctx.config().peer_connection_timeout();
        match tokio::time::timeout(timeout, TcpStream::connect(&*self.addr)).await {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(e)) => {
                error!("连接对端 peer 失败: {:?}", e);
                Err(Error::ConnectionError)
            },
            Err(_) => {
                Err(Error::TimeoutError)
            }
        }
    }

    /// 启动套接字监听
    fn start_listener(
        &mut self,
        stream: TcpStreamExt,
        peer_send: Sender<TransferPtr>,
        cancel: CancellationToken,
    ) -> (JoinHandle<()>, JoinHandle<()>) {
        let (reader, writer) = stream.into_split();
        let (send, recv) = channel(self.ctx.config().channel_buffer());
        let probe = Probe::new(self.dashbord.clone());

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
        let stream = self.connection().await?;
        let mut stream = self.crypto_handshake(stream).await?;

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
            Err(e) => {
                debug!("启动 future token 失败: {}", e);
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
                _ = self.keep_alive_interval.tick() => {
                    if self.send_heartbeat().await.is_err() {
                        // 失败了就失败，这个不是很重要，因为大部分
                        // 端都不会理会心跳包的
                        warn!("发送心跳包失败");
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
            .give_back_download_pieces(self.ctx.no, self.response_pieces.into_iter().collect());
        self.dashbord.clear_ing();
        self.emitter.remove(&Self::get_transfer_id(self.ctx.no));
        self.ctx.gc.peer_exit(self.ctx.no, reason).await;
        debug!("peer [{}] 已退出！", self.ctx.no);
    }
}
