//! 进行通用公共处理的 base peer，不进行具体的业务逻辑处理。
//! 例如，负责接收 piece 数据，但是不会再进行后续操作。由
//! `实现了 `Peer` trait 的具体子类来处理具体的业务逻辑。
//!
//! 协议格式：
//! `长度（4字节） | 消息类型（1字节） | 数据（长度-5字节）`
//!
//! 长度的需要加上消息类型的 1 字节。

use std::collections::VecDeque;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::mem;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

#[cfg(target_has_atomic = "64")]
use std::sync::atomic::AtomicU64;
#[cfg(not(target_has_atomic = "64"))]
use portable_atomic::AtomicU64;

use anyhow::{anyhow, Error, Result};
use async_trait::async_trait;
use bendy::decoding::FromBencode;
use bendy::encoding::ToBencode;
use bytes::{Bytes, BytesMut};
use dashmap::{DashMap, DashSet};
use doro_util::option_ext::OptionExt;
use doro_util::{anyhow_eq, bytes_util};
use doro_util::bytes_util::{WriteBytesBigEndian, Bytes2Int};
use doro_util::global::Id;
use doro_util::sync::{MutexExt, ReadLockExt, WriteLockExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::task::JoinSet;
use tracing::{debug, info, trace, error};

use crate::base_peer::listener::{ReadFuture, WriteFuture};
use crate::base_peer::rate_control::probe::{Dashboard, Probe};
use crate::base_peer::rate_control::RateControl;
use crate::bt::pe_crypto::{self, CryptoProvide};
use crate::bt::socket::TcpStreamWrapper;
use crate::command::CommandHandler;
use crate::config::CHANNEL_BUFFER;
use crate::context::{AsyncSemaphore, Context};
use crate::default_servant::extend_data::{HandshakeData, Metadata};
use crate::dht;
use crate::dht::DHT;
use crate::emitter::transfer::TransferPtr;
use crate::protocol::{BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN};
use crate::servant::Servant;
use crate::task::magnet::Magnet;
use crate::task::HostSource;
use crate::task_manager::PeerId;
use crate::torrent::{Info, Torrent, TorrentArc};

pub mod command;
pub mod error;
pub mod listener;
pub mod peer_resp;
pub mod rate_control;
pub mod reserved;

/// 尝试获取 servant 实例，如果没有，则直接 return Ok(())
macro_rules! try_get_servant_or_err {
    ($self: expr) => {
        match $self.servant.upgrade() {
            Some(servant) => servant,
            None => {
                return Err(anyhow!("servant weak ref released"));
            }
        }
    };
}

/// 尝试获取 servant 实例，如果没有，这直接 break
macro_rules! try_get_servant_or_break {
    ($self: expr) => {
        match $self.servant.upgrade() {
            Some(servant) => servant,
            None => {
                trace!("servant weak ref released");
                break;
            }
        }
    };
}

/// metadata 信息的分块大小
pub const METADATA_PIECE_SIZE: usize = 16384;

/// 本端的扩展 id 定义
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum ExtendedId {
    /// 扩展协议握手
    Handshake = 0,

    /// Peer Exchange       
    /// peer 交换
    UtPex = 1,

    /// 元数据交换          
    /// 通过磁链链接获取种子元数据信息
    UtMetadata = 2,

    /// 上传模式（超级种子模式）        
    /// 表示客户端处于仅上传状态
    UploadOnly = 3,

    /// µTorrent 的 NAT 打洞消息    
    /// 建立直接 P2P 连接
    UtHolepunch = 4,

    /// 评论扩展
    UtComment = 6,
}

impl ExtendedId {
    pub const fn get_value(&self) -> u8 {
        *self as u8
    }

    pub fn get_name(&self) -> &'static str {
        match self {
            ExtendedId::Handshake => "handshake",
            ExtendedId::UtPex => "ut_pex",
            ExtendedId::UtMetadata => "ut_metadata",
            ExtendedId::UploadOnly => "upload_only",
            ExtendedId::UtHolepunch => "ut_holepunch",
            ExtendedId::UtComment => "ut_comment",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "handshake" => Some(ExtendedId::Handshake),
            "ut_pex" => Some(ExtendedId::UtPex),
            "ut_metadata" => Some(ExtendedId::UtMetadata),
            "upload_only" => Some(ExtendedId::UploadOnly),
            "ut_holepunch" => Some(ExtendedId::UtHolepunch),
            "ut_comment" => Some(ExtendedId::UtComment),
            _ => None,
        }
    }
}

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
    ExtensionProtocol = 20,

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

impl TryFrom<u8> for MsgType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0..=9 | 13..=17 | 20..=23 => Ok(unsafe { mem::transmute::<u8, MsgType>(value) }),
            _ => Err(anyhow!("Invalid message type: {}", value)),
        }
    }
}

#[derive(Clone, Debug)]
struct Writer {
    inner: OnceLock<Sender<Vec<u8>>>,
}

impl Writer {
    fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    fn set_sender(&self, sender: Sender<Vec<u8>>) {
        // 安全：因为内部保证在设置 sender 时，
        // inner 是空的 OnceLock
        self.inner.set(sender).unwrap()
    }
}

impl Deref for Writer {
    type Target = Sender<Vec<u8>>;

    fn deref(&self) -> &Self::Target {
        // 安全：因为内部保证在使用 writer 时，
        // inner 已经有值了
        self.inner.get().unwrap()
    }
}

impl DerefMut for Writer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // 安全：因为内部保证在使用 writer 时，
        // inner 已经有值了
        self.inner.get_mut().unwrap()
    }
}


/// peer 启动结果
/// T 为回传值
pub enum PeerStartResult<T> {
    /// 启动成功
    Success(T),

    /// 启动失败
    Failed(T, Error),
}

pub struct BasePeerInner {
    /// 编号
    id: Id,

    /// 通信地址
    addr: SocketAddr,

    /// 具体的业务逻辑处理
    servant: Weak<dyn Servant>,

    /// 异步任务句柄
    handles: Mutex<Option<JoinSet<()>>>,

    /// 下载速率仪表盘
    dashboard: Dashboard,

    /// writer
    writer: Writer,

    /// 对端的 peer id
    opposite_peer_id: Mutex<Option<PeerId>>,

    /// 对端的 reserved 扩展位
    opposite_reserved: AtomicU64,

    /// 读取到数据时，通过此通道传输
    channel: (Sender<TransferPtr>, Mutex<Option<Receiver<TransferPtr>>>),
}

impl Drop for BasePeerInner {
    fn drop(&mut self) {
        trace!("base peer [{}] 已 drop", self.addr);
    }
}

impl Deref for BasePeer {
    type Target = Arc<BasePeerInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 基础的 Peer
#[derive(Clone)]
pub struct BasePeer(Arc<BasePeerInner>);

impl BasePeer {
    pub fn new(id: Id, addr: SocketAddr, servant: Weak<impl Servant>, dashboard: Dashboard) -> Self {
        let buf_limit = Context::get_config().buf_limit();
        let (tx, rx) = channel(buf_limit);
        Self(Arc::new(BasePeerInner {
            id,
            addr,
            servant,
            handles: Mutex::new(Some(JoinSet::new())),
            dashboard,
            writer: Writer::new(),
            opposite_peer_id: Mutex::new(None),
            opposite_reserved: AtomicU64::new(0),
            channel: (tx, Mutex::new(Some(rx))),
        }))
    }

    /// 连接对端 peer
    async fn connection(&self) -> Result<TcpStream> {
        let timeout = Context::get_config().peer_connection_timeout();
        match tokio::time::timeout(timeout, TcpStream::connect(self.addr)).await {
            Ok(Ok(socket)) => Ok(socket),
            Ok(Err(e)) => Err(anyhow!("连接对端 peer 失败\n{}", e)),
            Err(_) => Err(anyhow!("连接对端 peer 超时")),
        }
    }

    /// 加密握手
    #[rustfmt::skip]
    async fn crypto_handshake(&self, socket: TcpStream) -> Result<TcpStreamWrapper> {
        trace!("加密握手 peer_no: {}", self.id);
        let servant = try_get_servant_or_err!(self);
        let info_hash = servant.info_hash();
        pe_crypto::init_handshake(
            socket,
            info_hash,
            CryptoProvide::Rc4
        ).await
    }

    /// 协议握手
    #[rustfmt::skip]
    async fn protocol_handshake(&self, socket: &mut TcpStreamWrapper) -> Result<()> {
        trace!("发送握手信息 peer_no: {}", self.id);
        let servant = try_get_servant_or_err!(self);
        let msg_size = 1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN;
        let mut bytes = Vec::with_capacity(msg_size);
        let info_hash = servant.info_hash();
        bytes.write_u8(BIT_TORRENT_PROTOCOL_LEN)?;
        bytes.write_bytes(BIT_TORRENT_PROTOCOL)?;
        bytes.write_u64(reserved::LTEP | reserved::DHT)?; // 扩展位 
        bytes.write_bytes(info_hash)?;
        bytes.write_bytes(servant.peer_id())?;
        socket.write_all(&bytes).await?;

        // 等待握手信息
        socket.readable().await?;

        let mut handshake_resp = vec![0u8; bytes.len()];
        let size = socket.read(&mut handshake_resp).await?;
        anyhow_eq!(size, bytes.len(), "握手响应数据长度与预期不符 [{}]\t[{}]", size, bytes.len());

        let protocol_len = u8::from_be_bytes([handshake_resp[0]]) as usize;
        let reserved = u64::from_be_slice(&handshake_resp[1 + protocol_len..1 + protocol_len + 8]);
        let resp_info_hash = &handshake_resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
        let peer_id = &handshake_resp[1 + protocol_len + 8 + 20..];

        anyhow_eq!(info_hash, resp_info_hash, "对端的 info_hash 与本地不符");

        // 支持 ltep，就往 dht 路由表中添加
        if reserved & reserved::LTEP != 0 {
            let rt = DHT::global().routing_table.clone();
            let dr = DHT::global().dht_request.clone();
            dht::check_add_node(&rt, &dr, None, &self.addr).await;
        }

        *self.opposite_peer_id.lock_pe() = Some(PeerId::new(peer_id.try_into()?));
        self.opposite_reserved.store(reserved, Ordering::Relaxed);

        Ok(())
    }

    /// 启动套接字监听
    #[rustfmt::skip]
    fn start_listener(&self, socket: TcpStreamWrapper) {
        let (reader, writer) = socket.into_split();
        let (send, recv) = channel(CHANNEL_BUFFER);
        let probe = Probe::new(self.dashboard.clone());

        // 异步写入
        self.handles.lock_pe().as_mut()
            .unwrap().spawn(Box::pin(
                WriteFuture {
                    id: self.id,
                    writer,
                    addr: self.addr,
                    peer_sender: self.channel.0.clone(),
                    recv,
                }
                .run(),
            ));

        // 异步读取
        self.handles.lock_pe().as_mut()
            .unwrap().spawn(Box::pin(
                ReadFuture {
                    id: self.id,
                    reader,
                    addr: self.addr,
                    peer_sender: self.channel.0.clone(),
                    rc: probe,
                }
                .run(),
            ));

        self.writer.set_sender(send);
    }

    /// 获取一个异步任务信号量，如果异步任务池用完了，那么则返回 none，示意同步执行这个请求    
    ///  
    /// 这个不用处理 task_pool.len() < async_task_limit() 和 task_pool.spawn() 非原子性的问题，
    /// 因为这两个操作只会在监听线程中以同步的方式使用。
    fn take_async_semaphore(&self, task_pool: & JoinSet<Result<()>>) -> Option<AsyncSemaphore> {
        // 首先不可超出单个 peer 最大异步任务数的限制
        if task_pool.len() < Context::get_config().async_task_limit() {
            Context::take_async_task_semaphore()
        } else {
            None
        }
    }

    /// 持续监听收到的数据，并进行分发
    async fn persist_listen(self) {
        let mut rx = self.channel.1.lock_pe().take().unwrap();
        let mut task_pool = JoinSet::new(); // 任务池
        loop {
            tokio::select! {
                cmd = rx.recv() => {
                    if let Some(cmd) = cmd {
                        let cmd: command::Command = cmd.instance();
                        let ts = self.take_async_semaphore(&task_pool);
                        let servant = try_get_servant_or_break!(self);
                        if ts.is_none() {
                            if let Err(e) = cmd.handle((servant.clone(), ts)).await {
                                servant.happen_exeception(self.id, e).await;
                                break;
                            }
                        } else {
                            task_pool.spawn(Box::pin(cmd.handle((servant, ts))));
                        }
                    } else {
                        trace!("peer channel closed");
                        break;
                    }
                }
                ret = task_pool.join_next(), if !task_pool.is_empty() => {
                    let servant = try_get_servant_or_break!(self);
                    match ret {
                        Some(Ok(Err(e))) => {
                            servant.happen_exeception(self.id, e).await;
                            break;    
                        },
                        Some(Err(e)) => {
                            servant.happen_exeception(self.id, e.into()).await;
                            break;    
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub async fn async_run<T>(self, r: T) -> PeerStartResult<T> {
        match self.do_async_run().await {
            Ok(()) => PeerStartResult::Success(r),
            Err(e) => PeerStartResult::Failed(r, e),
        }
    }

    #[rustfmt::skip]
    async fn do_async_run(self) -> Result<()> {
        let socket = self.connection().await?;
        let mut socket = self.crypto_handshake(socket).await?;
        self.protocol_handshake(&mut socket).await?;
        self.start_listener(socket);
        
        // 这里很重要，将 handles 拿出来，所有权转移给 Peer 实例，然后 Peer 的所有权
        // 转移给 Servant，保证 Servant 被销毁时，Servant 管理下的 Peer 资源也能被
        // 正确回收
        let mut handles = self.handles.lock_pe().take().unwrap();
        handles.spawn(Box::pin(self.clone().persist_listen()));
        
        let peer = Peer::new(
            self.id, self.addr,
            self.writer.clone(),
            self.dashboard.clone(),
            handles, self.opposite_reserved.load(Ordering::Relaxed)
        );
        let servant = try_get_servant_or_err!(self);
        servant.add_peer(self.id, peer).await;

        Ok(())
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
pub enum Status {
    /// 不可下载
    Choke,

    /// 可以下载
    UnChoke,

    /// 等待新的分块下载
    _Wait,
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
        block_offset.div_ceil(block_size)
    }

    /// 新增完成的分块
    fn add_finish(&mut self, block_offset: u32) {
        if self.finish {
            return;
        }
        let bo_idx = Self::block_offset_idx(block_offset, self.block_size);
        let (idx, offset) = bytes_util::bitmap_offset(bo_idx);
        self.bitmap.get_mut(idx).map_ext(|val| *val |= offset);

        // 更新连续块偏移
        let bo_idx = Self::block_offset_idx(self.block_offset, self.block_size);
        let (idx, mut offset) = bytes_util::bitmap_offset(bo_idx);
        'first_loop: for value in self.bitmap[idx..].iter() {
            let mut k = offset;
            while k != 0 {
                if *value & k == 0 {
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

#[derive(Debug)]
pub struct PeerInner {
    /// 编号
    id: Id,

    /// 是否正在运行
    runnable: AtomicBool,

    /// 通信地址
    addr: SocketAddr,

    /// 写入通道
    writer: Writer,

    /// 请求的 piece 分片       
    /// 记录的是上次请求的偏移量
    request_pieces: DashMap<u32, u32>,

    /// 响应的 piece 分片       
    /// 记录的是已经接收到的偏移数据
    response_pieces: DashMap<u32, Piece>,

    /// 状态
    status: RwLock<Status>,

    /// 对面 peer 的状态
    op_status: RwLock<Status>,

    /// 对面 peer 的 bitfield
    op_bitfield: Arc<RwLock<BytesMut>>,

    /// 对端的扩展位
    op_reserved: u64,

    /// 是否接收到了对方的 bitfield
    recv_bitfield: AtomicBool,

    /// 速度仪表盘
    dashboard: Dashboard,

    /// 异步任务句柄
    handles: Mutex<Option<JoinSet<()>>>,

    /// 扩展 id 映射
    extend_id_map: DashMap<u8, u8>,

    /// 扩展协议握手数据
    extend_handshake_data: OnceLock<HandshakeData>,

    /// metadata 数据
    metadata: Mutex<Option<Vec<u8>>>,

    /// metadata 位图
    metadata_bitfield: Mutex<Option<Vec<u8>>>,

    /// metadata 分片索引
    metadata_piece_idx: AtomicUsize,

    /// 同步锁
    sync_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Drop for PeerInner {
    fn drop(&mut self) {
        trace!("peer {} 已关闭", self.addr);
    }
}

impl Deref for Peer {
    type Target = Arc<PeerInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 暴露给外部的 Peer 通信 socket 封装
#[derive(Clone, Debug)]
pub struct Peer(Arc<PeerInner>);

impl Peer {
    fn new(id: Id, addr: SocketAddr, writer: Writer, dashboard: Dashboard, handles: JoinSet<()>, op_reserved: u64) -> Self {
        let extend_id_map = DashMap::new();
        extend_id_map.insert(0, ExtendedId::Handshake.get_value()); // 扩展协议的握手 id 固定为 0

        Peer(Arc::new(PeerInner {
            id,
            runnable: AtomicBool::new(true),
            addr,
            writer,
            request_pieces: DashMap::new(),
            response_pieces: DashMap::new(),
            status: RwLock::new(Status::Choke),
            op_status: RwLock::new(Status::Choke),
            op_bitfield: Arc::new(RwLock::new(BytesMut::new())),
            op_reserved,
            recv_bitfield: AtomicBool::new(false),
            dashboard,
            handles: Mutex::new(Some(handles)),
            extend_id_map,
            extend_handshake_data: OnceLock::new(),
            metadata: Mutex::new(None),
            metadata_bitfield: Mutex::new(None),
            metadata_piece_idx: AtomicUsize::new(0),
            sync_lock: Arc::new(tokio::sync::Mutex::new(()))
        }))
    }

    pub fn name(&self) -> String {
        format!("{} - {}", self.addr, self.id)
    }

    pub fn wrapper(&self) -> PeerWrapper {
        PeerWrapper {
            inner: self.clone(),
            _not_sync: PhantomData
        }
    }

    pub async fn send(&self, data: Vec<u8>) -> Result<()> {
        self.writer
            .send(data)
            .await
            .map_err(|error| anyhow!("发送数据失败\t{error}"))
    }

    pub fn get_id(&self) -> Id {
        self.id
    }

    pub fn get_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn get_request_pieces(&self) -> &DashMap<u32, u32> {
        &self.request_pieces
    }

    pub fn get_response_pieces(&self) -> &DashMap<u32, Piece> {
        &self.response_pieces
    }

    pub fn is_runnable(&self) -> bool {
        self.runnable.load(Ordering::Relaxed)
    }

    fn check_runnable(&self) -> Result<()> {
        anyhow_eq!(self.is_runnable(), true, "peer 已关闭");
        Ok(())
    }

    pub fn get_sync_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.sync_lock.clone()
    }

    pub fn is_can_be_download(&self) -> bool {
        *self.status.read_pe() == Status::UnChoke && self.recv_bitfield.load(Ordering::Relaxed)
    }

    pub fn dashboard(&self) -> &Dashboard {
        &self.dashboard
    }

    pub fn is_can_be_request_metadata(&self) -> bool {
        self.is_ltep() && self.extend_handshake_data.get()
            .is_some_and(|data| data.m.contains_key(ExtendedId::UtMetadata.get_name()))
    }

    /// 对端是否支持 LTEP（扩展协议）
    pub fn is_ltep(&self) -> bool {
        self.op_reserved & reserved::LTEP != 0
    }

    /// 扩展 id 转换
    pub fn extend_id_transform(&self, id: u8) -> Option<u8> {
        self.extend_id_map.get(&id).map(|v| *v.value())
    }

    /// 添加扩展 id 映射
    pub fn add_extend_id(&self, source_id: u8, target_id: ExtendedId) {
        self.extend_id_map.insert(source_id, target_id.get_value());
    }

    /// 设置扩展协议握手数据
    pub fn set_extend_handshake_data(&self, data: HandshakeData) -> Result<()> {
        self.extend_handshake_data.set(data)
            .map_err(|_| anyhow!("扩展协议握手数据已存在"))
    }

    /// 获取 metadata   
    /// 调用此方法应该确保 metadata 已经初始化
    pub fn parse_torrent_from_metadata(&self, magnet: Arc<Magnet>) -> Result<TorrentArc> {
        let metadata = self.metadata.lock_pe();
        if metadata.is_none() {
            return Err(anyhow!("metadata 未初始化"));
        }

        let metadata = metadata.as_ref().unwrap();
        let info = Info::from_bencode(metadata.as_slice())
            .map_err(|e| anyhow!("解析 torrent 元数据失败\t{e}"))?;
        Ok(TorrentArc::new(Torrent::new(info, magnet.info_hash, magnet.trackers.clone())))
    }

    /// 从扩展协议握手数据中获取种子元数据大小
    pub fn get_metadata_size(&self) -> Option<u32> {
        self.extend_handshake_data.get()?.metadata_size
    }

    /// 检查对端是否支持指定的扩展协议
    pub fn check_extend_support(&self, extend: ExtendedId) -> bool {
        self.extend_handshake_data.get()
            .is_some_and(|data| data.m.contains_key(extend.get_name()))
    }

    /// 初始化 metadata
    pub fn init_metadata_piece(&self, size: usize) {
        let mut metadata = self.metadata.lock_pe();
        if metadata.is_none() {
            *metadata = Some(vec![0u8; size]);
        }

        let mut metadata_bitfield = self.metadata_bitfield.lock_pe();
        if metadata_bitfield.is_none() {
            let bit_len = size.div_ceil(METADATA_PIECE_SIZE);
            let len = (bit_len + 7) >> 3;
            *metadata_bitfield = Some(vec![0u8; len]);
        }
    }

    /// 设置 metadata 的分片数据
    pub fn set_metadata_piece(&self, piece_idx: u32, data: Bytes) -> Result<()> {
        {
            let mut metadata_lock = self.metadata.lock_pe();
            let metadata = metadata_lock.as_mut().unwrap();
            let index = piece_idx as usize * METADATA_PIECE_SIZE;
            if index + data.len() > metadata.len() {
                return Err(anyhow!("metadata 分片 [{piece_idx}] 数据异常，长度 [{}] 超出范围", data.len()));
            }
            metadata[index..index+data.len()].copy_from_slice(&data);
        }
        
        {
            let mut metadata_bitfield_lock = self.metadata_bitfield.lock_pe();
            let metadata_bitfield = metadata_bitfield_lock.as_mut().unwrap();
            let (idx, offset) = bytes_util::bitmap_offset(piece_idx);
            metadata_bitfield[idx] |= offset;

            let mut metadata_piece_idx = self.metadata_piece_idx.load(Ordering::Acquire);
            let (idx, mut offset) = bytes_util::bitmap_offset(metadata_piece_idx as u32);
            'first_loop: for value in metadata_bitfield[idx..].iter() {
                let mut k = offset;
                while k != 0 {
                    if *value & k == 0 {
                        break 'first_loop;
                    }
                    metadata_piece_idx += 1;
                    k >>= 1;
                }
                offset = 1 << 7
            }

            self.metadata_piece_idx.store(metadata_piece_idx, Ordering::Release);
        }
        Ok(())
    }

    pub fn is_metadata_complete(&self) -> bool {
        let total_size = { self.metadata.lock_pe().as_ref().unwrap().len() };
        let total_piece_size = total_size.div_ceil(METADATA_PIECE_SIZE);
        self.metadata_piece_idx.load(Ordering::Relaxed) >= total_piece_size
    }

    pub fn get_metadata_piece_idx(&self) -> u32 {
        self.metadata_piece_idx.load(Ordering::Relaxed) as u32
    }

    pub fn status(&self) -> Status {
        *self.status.read_pe()
    }

    /// 设置状态
    pub fn set_status(&self, status: Status) {
        *self.status.write_pe() = status;
    }

    pub fn op_status(&self) -> Status {
        *self.op_status.read_pe()
    }

    /// 设置对端 peer 的状态    
    pub fn set_op_status(&self, status: Status) {
        *self.op_status.write_pe() = status;
    }

    pub fn op_bitfield(&self) -> Arc<RwLock<BytesMut>> {
        self.op_bitfield.clone()
    }

    pub fn set_op_bitfield(&self, bitfield: BytesMut) {
        *self.op_bitfield.write_pe() = bitfield;
        self.recv_bitfield.store(true, Ordering::Relaxed);
    }

    /// 是否还有传输数据（指等待响应的数据，或者对方已经 UnChoke，但是我们还没有收到对方的 bitfield）    
    /// ture 表示还有数据，false 表示没有数据
    pub fn has_transfer_data(&self) -> bool {
        (!self.response_pieces.is_empty() && self.dashboard.inflight() > 0) || (
            self.status() == Status::UnChoke &&
            !self.recv_bitfield.load(Ordering::Relaxed)
        )
    }

    /// 是否可以发送数据，即 inflight 数据小于 cwnd
    pub fn has_send_window_space(&self) -> bool {
        self.dashboard.inflight() <= self.dashboard.cwnd()
    }

    /// 重置分片请求的起始位置
    pub fn reset_request_piece_origin(&self, piece_idx: u32, block_offset: u32, piece_length: u32) {
        if let Some(origin_offset) = self.request_pieces.get(&piece_idx) {
            // 避免先写入错误时，高偏移值替换掉还没请求的低偏移值。
            // 例如：先请求了 0 号偏移数据，再请求 1 号偏移数据，接着
            // 收到的 0 号 和 1 号 都发生了写入错误，先把请求记录重置为
            // 了 0 号偏移，但是还没重新发起 0 号请求，就接着重置为了 1 号，
            // 导致一直缺失 0 号数据。
            if *origin_offset < block_offset {
                return;
            }
        }
        self.insert_response_pieces(piece_idx, block_offset, piece_length);
        self.request_pieces.insert(piece_idx, block_offset);
    }

    pub fn insert_response_pieces(&self, piece_idx: u32, block_offset: u32, piece_length: u32) {
        let bs = Context::get_config().block_size();
        self.response_pieces.insert(
            piece_idx, 
            Piece::new(block_offset, piece_length, bs)
        );
    }

    /// 检查是否是有效的 piece 响应    
    /// 为什么需要检查？因为有可能当前 peer 请求了某个 piece，但是还没来得及处理这个 piece 的响应，
    /// 调度者就把这个 piece 的请求分配给了另一个 peer 来处理。为了保证 piece 的完整性，会尽量让
    /// 同一个 peer 来处理一个 piece。因此，这里将检验，这个 piece 是不是还是当前 peer 负责处理
    /// 的有效 piece。
    pub fn is_valid_piece_response(&self, piece_idx: u32) -> bool {
        self.response_pieces.contains_key(&piece_idx)
    }

    /// 分片是否接受完了
    pub fn piece_recv_finish(&self, piece_idx: u32) -> bool {
        self.response_pieces.get(&piece_idx)
            .is_some_and(|piece| piece.is_finish())
    }

    /// 收到新的响应分片后，更新响应分片记录
    pub fn update_response_piece(&self, piece_idx: u32, block_offset: u32) {
        self.response_pieces.get_mut(&piece_idx)
            .map_ext(|mut piece| piece.add_finish(block_offset))
    }

    /// 获取分片中，连续 block 的最后一个
    pub fn get_seq_last_piece_block_offset(&self, piece_idx: u32) -> Option<u32> {
        self.response_pieces.get(&piece_idx).map(|p| p.block_offset())
    }

    /// 检查分片是否下载完了
    pub fn remove_finish_piece(&self, piece_idx: u32) {
        self.response_pieces.remove(&piece_idx);
    }

    /// 发送自己拥有的分片
    pub async fn request_bitfield(&self, bitfield: Arc<Mutex<BytesMut>>) -> Result<()> {
        self.check_runnable()?;
        let bytes = {
            let bitfield = &*bitfield.lock_pe();
            let mut bytes = Vec::with_capacity(5 + bitfield.len());
            bytes.write_u32(1 + bitfield.len() as u32)?;
            bytes.write_u8(MsgType::Bitfield as u8)?;
            bytes.write_bytes(bitfield)?;
            bytes
        };

        self.send(bytes).await
    }

    /// 发送感兴趣的消息
    pub async fn request_interested(&self) -> Result<()> {
        self.check_runnable()?;
        let mut bytes = Vec::with_capacity(5);
        bytes.write_u32(1)?;
        bytes.write_u8(MsgType::Interested as u8)?;
        self.send(bytes).await
    }

    /// 发送允许请求数据的消息
    pub async fn request_un_choke(&self) -> Result<()> {
        self.check_runnable()?;
        let mut bytes = Vec::with_capacity(5);
        bytes.write_u32(1)?;
        bytes.write_u8(MsgType::UnChoke as u8)?;
        self.send(bytes).await
    }

    /// 发送不允许请求数据的消息
    pub async fn request_choke(&self) -> Result<()> {
        self.check_runnable()?;
        let mut bytes = Vec::with_capacity(5);
        bytes.write_u32(1)?;
        bytes.write_u8(MsgType::Choke as u8)?;
        self.send(bytes).await
    }

    /// 请求分片
    pub async fn request_piece(&self, piece_idx: u32, block_offset: u32, block_size: u32) -> Result<()> {
        self.check_runnable()?;
        let mut bytes = Vec::with_capacity(17);
        bytes.write_u32(13)?;
        bytes.write_u8(MsgType::Request as u8)?;
        bytes.write_u32(piece_idx)?;
        bytes.write_u32(block_offset)?;
        bytes.write_u32(block_size)?;
        self.send(bytes).await
    }

    /// 请求扩展协议握手数据
    pub async fn request_extend_handshake(&self) -> Result<()> {
        self.check_runnable()?;
        if !self.is_ltep() {
            return Err(anyhow!("对端不支持扩展协议，无法请求扩展协议握手数据"));
        }

        let mut handshake = HandshakeData::new();
        handshake.m.insert(ExtendedId::UtMetadata.get_name().to_string(), ExtendedId::UtMetadata.get_value());
        let handshake = handshake.to_bencode()
            .map_err(|e| anyhow!("扩展协议握手数据编码失败\t{e}"))?;

        let mut bytes = Vec::with_capacity(6 + handshake.len());
        bytes.write_u32(2 + handshake.len() as u32)?;
        bytes.write_u8(MsgType::ExtensionProtocol as u8)?;
        bytes.write_u8(ExtendedId::Handshake.get_value())?;
        bytes.write_bytes(&handshake)?;
        self.send(bytes).await
    }

    /// 请求 metadata 分片
    pub async fn request_metadata_piece(&self, piece: u32) -> Result<()> {
        self.check_runnable()?;
        if !self.is_can_be_request_metadata() {
            return Err(anyhow!("对端不支持 ut_metadata 扩展协议，无法请求 metadata 分片"));
        }

        let metadata = Metadata::request(piece).to_bencode()
            .map_err(|e| anyhow!("metadata 请求数据编码失败\t{e}"))?;
        let msg_type = self.extend_handshake_data
            .get().unwrap()
            .m.get(ExtendedId::UtMetadata.get_name())
            .unwrap();

        let mut bytes = Vec::with_capacity(6 + metadata.len());
        bytes.write_u32(2 + metadata.len() as u32)?;
        bytes.write_u8(MsgType::ExtensionProtocol as u8)?;
        bytes.write_u8(*msg_type)?;
        bytes.write_bytes(&metadata)?;
        self.send(bytes).await
    }

    /// 关闭此 peer
    pub async fn shutdown(&self) {
        if self.runnable.swap(false, Ordering::Relaxed) {
            let handles = {
                match self.handles.lock_pe().take() {
                    Some(handles) => handles,
                    None => return,
                }
            };
            drop(handles)
        }
    }
}

/// Peer 的包装，用于将 Peer 的可共享信息对外放开。例如 Peer 中的 handles 是绝对不能 Sync 的。
/// 所以，我们对外共享 Peer，只能共享 PeerWrapper，PeerWrapper 包装了可以 Sync 的 Peer，但是
/// 我们指定 PeerWrapper 不可 Sync，保证使用者无法再次让 PeerWrapper 在多线程中被共享。以避免
/// 内存泄漏
#[derive(Debug)]
pub struct PeerWrapper {
    /// 内部还是指向 Peer
    inner: Peer,

    /// 保证 PeerWrapper 不会被 Sync，但可以被 Send
    _not_sync: PhantomData<Mutex<()>> 
}

impl Deref for PeerWrapper {
    type Target = Peer;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// peer info 扩展
pub trait PeerInfoExt: Send + Debug {
    /// 获取 peer id
    fn get_id(&self) -> Id;

    /// 获取 peer 地址
    fn get_addr(&self) -> SocketAddr;

    /// 获取 dashboard
    fn get_dashboard(&self) -> Dashboard;

    /// 获取 peer 名称
    fn get_name(&self) -> String;

    /// 获取 peer 来源
    fn get_source(&self) -> HostSource;

    /// 获取 peer 链接限制
    fn get_peer_conn_limit(&self) -> usize;

    /// 是否来自等待队列
    fn is_from_waited(&self) -> bool;
}

#[async_trait]
pub trait PeerLaunchCallback {
    /// 当 peer 启动失败时调用
    async fn on_peer_start_failed(&self, peer_info: Box<dyn PeerInfoExt>);

    /// 当 peer 启动成功时调用
    async fn on_peer_start_success(&self, peer_id: Id);

    /// 发生不可逆的错误
    async fn on_panic(&self, error: Error);
}

/// peer 启动器
pub struct PeerLaunch<T, S, C> {
    /// peer 启动任务池
    join_set: JoinSet<PeerStartResult<(T, Option<AsyncSemaphore>)>>,

    /// peer 事件处理
    servant: Weak<S>,

    /// 等待队列
    wait_queue: Arc<Mutex<VecDeque<T>>>,

    /// 不可启动的 host 集合
    unstart_host: Arc<DashSet<SocketAddr>>,

    /// peer 启动事件回调
    callback: Weak<C>,

    /// peer 管理集合
    peers: Arc<DashMap<Id, T>>,

    /// 消费通道
    channel: (Sender<T>, Receiver<T>)
}

impl<T, S, C> Drop for PeerLaunch<T, S, C> {
    fn drop(&mut self) {
        info!("PeerLaunch 已 drop");
    }
}

/// 当等待队列长度是 peer_limit 的 n 倍时，触发快速启动
const FAST_START_CRITICAL_VALUE: usize = 3;

impl<T, S, C> PeerLaunch<T, S, C>
where 
    T: PeerInfoExt + Send + 'static,
    S: Servant,
    C: PeerLaunchCallback,
{
    pub fn new(servant: Weak<S>, peers: Arc<DashMap<Id, T>>, 
        wait_queue: Arc<Mutex<VecDeque<T>>>, unstart_host: Arc<DashSet<SocketAddr>>,
        callback: Weak<C>, channel: (Sender<T>, Receiver<T>)
    ) -> Self {
        Self {
            join_set: JoinSet::new(),
            servant,
            callback,
            peers,
            channel,
            wait_queue,
            unstart_host,
        }
    }

    pub fn get_sender(&self) -> Sender<T> {
        self.channel.0.clone()
    }

    fn get_async_start_limit(&self) -> usize {
        let config = Context::get_config();
        config.async_peer_start_limit().max(config.torrent_peer_conn_limit().saturating_sub(self.peers.len()))
    }

    fn take_async_semaphore(&self, fast_start: bool) -> Option<AsyncSemaphore> {
        if fast_start || self.join_set.len() < self.get_async_start_limit() {
            Context::take_async_peer_start_semaphore()
        } else {
            None
        }
    }

    async fn handle_peer_start_result(&self, ret: PeerStartResult<(T, Option<AsyncSemaphore>)>) {
        match ret {
            PeerStartResult::Success((peer_info, _)) => {
                let id = peer_info.get_id();
                self.peers.insert(peer_info.get_id(), peer_info);
                if let Some(callback) = self.callback.upgrade() {
                    callback.on_peer_start_success(id).await;
                }
            }
            PeerStartResult::Failed((peer_info, _), e) => {
                debug!("peer [{}] 启动失败: {}", peer_info.get_name(), e);
                self.unstart_host.remove(&peer_info.get_addr());
                if let Some(callback) = self.callback.upgrade() {
                    callback.on_peer_start_failed(Box::new(peer_info)).await;
                }
            }
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                peer_info = self.channel.1.recv() => {
                    match peer_info {
                        Some(peer_info) => {
                            debug!("启动 [{}]，启动队列有[{}]个正在启动", peer_info.get_name(), self.join_set.len());
                            if !peer_info.is_from_waited() && self.unstart_host.contains(&peer_info.get_addr()) {
                                debug!("[{}] 已在启动队列中，或者被禁用", peer_info.get_name());
                                continue;
                            }

                            self.unstart_host.insert(peer_info.get_addr());
                            let limit = peer_info.get_peer_conn_limit();
                            let running_num = self.peers.len() + self.join_set.len();
                            let wait_num = self.wait_queue.lock_pe().len();
                            let fast_start = running_num * FAST_START_CRITICAL_VALUE < wait_num;
                            if !fast_start && running_num >= limit {
                                debug!("达到启动上限，把 [{}] 加入等待队列", peer_info.get_name());
                                self.wait_queue.lock_pe().push_back(peer_info);
                                continue;
                            }

                            let bp = BasePeer::new(
                                peer_info.get_id(), peer_info.get_addr(), 
                                self.servant.clone(), peer_info.get_dashboard()
                            );
                            let ts = self.take_async_semaphore(fast_start);
                            if ts.is_none() {
                                self.handle_peer_start_result(bp.async_run((peer_info, ts)).await).await;
                            } else {
                                self.join_set.spawn(bp.async_run((peer_info, ts)));
                            }
                        }
                        None => {
                            break;
                        }
                    }
                } 
                ret = self.join_set.join_next(), if !self.join_set.is_empty() => {
                    match ret {
                        Some(Ok(ret)) => {
                            self.handle_peer_start_result(ret).await;
                        }
                        Some(Err(e)) => {
                            // 不可逆的错误，无法恢复，当作 panic 处理
                            if let Some(callback) = self.callback.upgrade() {
                                callback.on_panic(anyhow!("peer 启动器的任务出错: {e}")).await;
                            } else {
                                error!("peer 启动器的任务出错: {e}");    
                            }
                        }
                        None => {
                            // 不会走到这个分支来的
                            info!("peer 启动器的任务集合为空，没有 peer 在启动中");
                        }
                    }
                }
            }
        }
    }
}
