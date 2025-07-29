//! 进行通用公共处理的 base peer，不进行具体的业务逻辑处理。
//! 例如，负责接收 piece 数据，但是不会再进行后续操作。由
//! `实现了 `Peer` trait 的具体子类来处理具体的业务逻辑。
//!
//! 协议格式：
//! `长度（4字节） | 消息类型（1字节） | 数据（长度-5字节）`
//!
//! 长度的需要加上消息类型的 1 字节。

use crate::base_peer::listener::{ReadFuture, WriteFuture};
use crate::base_peer::rate_control::probe::{Dashbord, Probe};
use crate::bt::pe_crypto::{self, CryptoProvide};
use crate::bt::socket::TcpStreamWrapper;
use crate::command::CommandHandler;
use crate::config::CHANNEL_BUFFER;
use crate::context::Context;
use crate::emitter::transfer::TransferPtr;
use crate::protocol::{BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN};
use crate::servant::Servant;
use crate::task_manager::PeerId;
use anyhow::{Error, Result, anyhow};
use doro_util::anyhow_eq;
use doro_util::bytes_util::WriteBytesBigEndian;
use doro_util::global::Id;
use doro_util::sync::MutexExt;
use std::mem;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::task::JoinSet;
use tracing::trace;

pub mod command;
pub mod error;
pub mod listener;
pub mod peer_resp;
pub mod rate_control;
pub mod reserved;

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

impl TryFrom<u8> for MsgType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0..=9 | 13..=17 | 20..=23 => Ok(unsafe { mem::transmute::<u8, MsgType>(value) }),
            _ => Err(anyhow!("Invalid message type: {}", value)),
        }
    }
}

#[derive(Clone)]
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

pub struct BasePeerInner {
    /// 编号
    id: Id,

    /// 通信地址
    addr: SocketAddr,

    /// 具体的业务逻辑处理
    servant: Arc<dyn Servant>,

    /// 异步任务句柄
    handles: Mutex<JoinSet<()>>,

    /// 下载速率仪表盘
    dashbord: Dashbord,

    /// wrtier
    writer: Writer,

    /// 对端的 peer id
    opposite_peer_id: Mutex<Option<PeerId>>,

    /// 读取到数据时，通过此通道传输
    channel: (Sender<TransferPtr>, Mutex<Option<Receiver<TransferPtr>>>),
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
    pub fn new(id: Id, addr: SocketAddr, servant: Arc<impl Servant>, dashbord: Dashbord) -> Self {
        let buf_limit = Context::global().get_config().buf_limit();
        let (tx, rx) = channel(buf_limit);
        Self(Arc::new(BasePeerInner {
            id,
            addr,
            servant,
            handles: Mutex::new(JoinSet::new()),
            dashbord,
            writer: Writer::new(),
            opposite_peer_id: Mutex::new(None),
            channel: (tx, Mutex::new(Some(rx))),
        }))
    }

    /// 连接对端 peer
    async fn connection(&self) -> Result<TcpStream> {
        let timeout = Context::global().get_config().peer_connection_timeout();
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
        pe_crypto::init_handshake(
            socket, 
            self.servant.info_hash(), 
            CryptoProvide::Rc4
        ).await
    }

    /// 协议握手
    #[rustfmt::skip]
    async fn protocol_handshake(&self, socket: &mut TcpStreamWrapper) -> Result<()> {
        trace!("发送握手信息 peer_no: {}", self.id);
        let msg_size = 1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN;
        let mut bytes = Vec::with_capacity(msg_size);
        let info_hash = self.servant.info_hash();
        bytes.write_u8(BIT_TORRENT_PROTOCOL_LEN)?;
        bytes.write_bytes(BIT_TORRENT_PROTOCOL)?;
        bytes.write_u64(reserved::LTEP | reserved::DHT)?; // 扩展位 
        bytes.write_bytes(info_hash)?;
        bytes.write_bytes(self.servant.peer_id())?;
        socket.write_all(&bytes).await?;

        // 等待握手信息
        socket.readable().await?;

        let mut handshake_resp = vec![0u8; bytes.len()];
        let size = socket.read(&mut handshake_resp).await?;
        anyhow_eq!(size, bytes.len(), "握手响应数据长度与预期不符 [{}]\t[{}]", size, bytes.len());

        let protocol_len = u8::from_be_bytes([handshake_resp[0]]) as usize;
        let resp_info_hash = &handshake_resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
        let peer_id = &handshake_resp[1 + protocol_len + 8 + 20..];

        anyhow_eq!(info_hash, resp_info_hash, "对端的 info_hash 与本地不符");

        *self.opposite_peer_id.lock_pe() = Some(PeerId::new(peer_id.try_into()?));

        Ok(())
    }

    /// 启动套接字监听
    fn start_listener(&self, socket: TcpStreamWrapper) {
        let (reader, writer) = socket.into_split();
        let (send, recv) = channel(CHANNEL_BUFFER);
        let probe = Probe::new(self.dashbord.clone());

        // 异步写入
        self.handles.lock_pe().spawn(Box::pin(
            WriteFuture {
                id: self.id,
                writer,
                peer_sender: self.channel.0.clone(),
                recv,
            }
            .run(),
        ));

        // 异步读取
        self.handles.lock_pe().spawn(Box::pin(
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

    /// 持续监听收到的数据，并进行分发
    async fn persist_listen(self) {
        let mut rx = self.channel.1.lock_pe().take().unwrap();
        while let Some(cmd) = rx.recv().await {
            let cmd: command::Command = cmd.instance();
            if let Err(e) = cmd.handle(&self.servant).await {
                self.servant.happen_exeception(self.id, e);
                break;
            }
        }
    }

    pub async fn async_run(self) -> Result<()> {
        let socket = self.connection().await?;
        let mut socket = self.crypto_handshake(socket).await?;
        self.protocol_handshake(&mut socket).await?;
        self.servant
            .servant_callback()
            .on_handshake_success(self.id)
            .await;
        self.start_listener(socket);

        // self.bitfield().await?;
        // self.interested().await?;

        self.handles
            .lock_pe()
            .spawn(Box::pin(self.clone().persist_listen()));

        let peer = Peer {
            id: self.id,
            writer: self.writer.clone(),
        };
        self.servant.add_peer(self.id, peer);

        Ok(())
    }
}

/// 暴露给外部的 Peer 通信 socket 封装
#[allow(dead_code)]
pub struct Peer {
    id: Id,
    writer: Writer,
}

impl Peer {
    pub async fn send(&self, data: Vec<u8>) -> Result<()> {
        self.writer
            .send(data)
            .await
            .map_err(|error| anyhow!("发送数据失败\t{error}"))
    }
}
