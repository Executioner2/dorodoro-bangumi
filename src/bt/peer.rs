//! 端到端通信。
//!
//! 协议格式：
//! `长度（4字节） | 消息类型（1字节） | 数据（长度-5字节）`
//!
//! 长度的需要加上消息类型的 1 字节。
//!
//! 区块与分片的关系：区块是制作种子文件的单位，而分片是对种子文件进行下载的单位。分片可以有下载器动态控制，分片大小 <= 区块大小。

mod error;

use crate::buffer::ByteBuffer;
use crate::bytes::Bytes2Int;
use crate::collection::FixedQueue;
use crate::core::alias::{ReceiverPeer, SenderPeer};
use crate::core::peer_manager::GasketContext;
use crate::core::protocol::{
    BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN,
};
use crate::core::runtime::Runnable;
use crate::peer::error::Error::{
    HandshakeError, ResponseDataIncomplete, TryFromError,
};
use crate::tracker::Host;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::{Bytes, BytesMut};
use error::Result;
use std::cmp::min;
use std::net::SocketAddr;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc::channel;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, error, trace, warn};

/// Peer 通信的消息类型
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
    type Error = error::Error;

    fn try_from(value: u8) -> Result<Self> {
        if value <= 8 {
            Ok(unsafe { std::mem::transmute(value) })
        } else {
            Err(TryFromError)
        }
    }
}

struct PeerStatistics {
    total_bytes_recv: u64,        // 总共接收的字节数
    last_bytes_recv: u64,         // 上次接收的字节数
    last_update_time: u64,        // 上次更新时间
    download_speed: u64,          // 下载速度
    rate_window: FixedQueue<u64>, // 最近 n 次的下载速率
}

impl PeerStatistics {
    fn new() -> Self {
        Self {
            total_bytes_recv: 0,
            last_bytes_recv: 0,
            last_update_time: 0,
            download_speed: 0,
            rate_window: FixedQueue::new(10),
        }
    }
}

#[derive(Eq, PartialEq, Hash)]
enum Status {
    Choke,
    UnChoke,
    Interested,
    NotInterested,
    Wait, // 等待新的分块下载
}

pub struct Peer {
    id: u64,
    peer_id: [u8; 20],
    opposite_peer_id: Option<[u8; 20]>,
    context: GasketContext,
    statistics: PeerStatistics,
    reader: OwnedReadHalf,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    status: Status,
    peer_status: Status,
    peer_bitfield: Option<BytesMut>,
    channel: (SenderPeer, ReceiverPeer),
}

impl Peer {
    pub async fn new(
        id: u64,
        peer_id: [u8; 20],
        host: Host,
        context: GasketContext,
        buff_size: usize,
    ) -> Option<Self> {
        let addr: SocketAddr = host.into();
        let stream = match TcpStream::connect(addr).await {
            Ok(stream) => stream,
            Err(e) => {
                debug!("addr: {:?}\tconnect error: {}", addr, e);
                return None;
            }
        };
        let (reader, writer) = stream.into_split();
        Some(Peer {
            id,
            peer_id,
            opposite_peer_id: None,
            context,
            statistics: PeerStatistics::new(),
            reader,
            writer: Arc::new(Mutex::new(writer)),
            status: Status::Choke,
            peer_status: Status::Choke,
            peer_bitfield: None,
            channel: channel(buff_size),
        })
    }

    /// 握手，然后询问对方有哪些分块是可以下载的
    pub async fn start(mut self) -> Result<(JoinHandle<()>, SenderPeer)> {
        // 握手信息
        let mut bytes =
            Vec::with_capacity(1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN);
        let info_hash = &self.context.torrent.info_hash;
        WriteBytesExt::write_u8(&mut bytes, BIT_TORRENT_PROTOCOL_LEN)?;
        std::io::Write::write(&mut bytes, BIT_TORRENT_PROTOCOL)?;
        WriteBytesExt::write_u64::<BigEndian>(&mut bytes, 0u64)?;
        std::io::Write::write(&mut bytes, info_hash)?;
        std::io::Write::write(&mut bytes, &self.peer_id)?;

        // 发送握手信息
        self.writer.lock().await.write_all(&bytes).await?;

        // 等待握手信息
        self.reader.readable().await?;

        let mut handshake_resp = Vec::with_capacity(bytes.len());
        let size = self.reader.read(&mut handshake_resp).await?;
        if size != bytes.len() {
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
        let bitfield = self.context.bytefield().await;
        let mut bytes = Vec::with_capacity(5 + bitfield.len());
        WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 1 + bitfield.len() as u32)?;
        WriteBytesExt::write_u8(&mut bytes, MsgType::Bitfield as u8)?;
        std::io::Write::write(&mut bytes, &bitfield)?;
        self.writer.lock().await.write_all(&bytes).await?;

        // 告诉对方，对他拥有的资源很感兴趣
        let mut bytes = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut bytes, 1)?;
        WriteBytesExt::write_u8(&mut bytes, MsgType::Interested as u8)?;
        self.writer.lock().await.write_all(&bytes).await?;

        // 开启运行时监听
        let send = self.channel.0.clone();
        let join_handle = tokio::spawn(self.run());

        Ok((join_handle, send))
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

    /// 下载分块
    async fn download_pices(&mut self, piece_index: u32) {
        async fn do_download_pices(writer: Arc<Mutex<OwnedWriteHalf>>, piece_index: u32, sharding_size: u64, piece_length: u64) {
            let mut sharding_offset = 0u64;
            while sharding_offset < piece_length {
                let mut req = Vec::with_capacity(13);
                WriteBytesExt::write_u32::<BigEndian>(&mut req, 13).unwrap();
                WriteBytesExt::write_u8(&mut req, MsgType::Request as u8).unwrap();
                WriteBytesExt::write_u32::<BigEndian>(&mut req, piece_index).unwrap();
                WriteBytesExt::write_u32::<BigEndian>(&mut req, sharding_offset as u32).unwrap();
                WriteBytesExt::write_u32::<BigEndian>(
                    &mut req,
                    min(piece_length - sharding_offset, sharding_size) as u32,
                ).unwrap();

                writer.lock().await.write_all(&req).await.unwrap();
                sharding_offset += sharding_size;
            }
        }
        let sharding_size = 1 << 14; // 分块大小
        let piece_length = self.context.torrent.info.piece_length;
        let resource_length = self.context.torrent.info.length;
        let piece_length = piece_length.min(
            resource_length.saturating_sub(piece_index as u64 * piece_length)
        );
        let writer = self.writer.clone();
        tokio::spawn(do_download_pices(writer, piece_index, sharding_size, piece_length));
    }

    /// 不让我们请求数据了
    async fn handle_choke(&mut self) -> Result<()> {
        self.status = Status::Choke;
        Ok(())
    }

    /// 告知可以交换数据了
    async fn handle_un_choke(&mut self) -> Result<()> {
        self.status = Status::UnChoke;
        Ok(())
    }

    /// 对我们的数据感兴趣
    async fn handle_interested(&mut self) -> Result<()> {
        trace!("对端对我们的数据感兴趣，那我们就允许他下载");
        self.peer_status = Status::UnChoke;
        let mut req = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut req, 1)?;
        WriteBytesExt::write_u8(&mut req, MsgType::UnChoke as u8)?;
        self.writer.lock().await.write_all(&req).await?;
        Ok(())
    }

    /// 对我们的数据不再感兴趣了
    async fn handle_not_interested(&mut self) -> Result<()> {
        trace!("对端对我们的数据不再感兴趣了，那就告诉他你别下载了！");
        self.peer_status = Status::Choke;
        let mut req = Vec::with_capacity(5);
        WriteBytesExt::write_u32::<BigEndian>(&mut req, 1)?;
        WriteBytesExt::write_u8(&mut req, MsgType::Choke as u8)?;
        self.writer.lock().await.write_all(&req).await?;
        Ok(())
    }

    /// 对端告诉我们他有新的分块可以下载
    async fn handle_have(&mut self, bytes: Bytes) -> Result<()> {
        trace!("对端告诉我们他有新的分块可以下载");
        if bytes.len() != 4 {
            return Err(ResponseDataIncomplete);
        }
        let bit = u32::from_be_slice(&bytes[..4]);
        let idx = (bit / 8) as usize;
        let offset = 1 << (bit % 8);
        let mut ub = self.context.underway_bytefield.lock().await;
        if idx >= ub.len() {
            trace!("传入的数据有问题，忽略先");
            return Ok(());
        }
        let new = ub[idx] & offset == 0; // 判断是否是新增的
        ub.as_mut().get_mut(idx).map(|x| *x |= offset);
        drop(ub); // 释放锁

        self.peer_bitfield.as_mut().map(|bytes| bytes[idx] |= offset);
        if new && self.status == Status::Wait {
            // 正在等新的分块可用，刚好这里来了个新的，那么就直接下载，不用告诉 gasket，有下得更快的想来抢，
            // 就让他告诉 gasket，让 gasket 来决定是否分配给其它 peer
            self.download_pices(bit).await;
        }

        Ok(())
    }

    /// 对端告诉我们他有哪些分块可以下载
    async fn handle_bitfield(&mut self, bytes: Bytes) -> Result<()> {
        let torrent = self.context.torrent.clone();
        let bit_len = torrent.info.pieces.len() / 20;
        if bytes.len() != bit_len + 7 / 8 {
            warn!("远端给到的 bitfield 长度和实际的不一致");
            return Ok(());
        }

        self.peer_bitfield = Some(BytesMut::from(bytes));
        for i in 0..=bit_len {
            let idx = i / 8;
            let offset = 1 << (i % 8);
            let mut ub = self.context.underway_bytefield.lock().await;
            let new = ub[idx] & offset == 0;
            ub.as_mut().get_mut(idx).map(|x| *x |= offset);
            drop(ub);

            if new {
                self.download_pices(i as u32).await;
            }
        }

        Ok(())
    }

    /// 向我们请求数据
    async fn handle_request(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端向我们请求数据");
        if self.peer_status != Status::UnChoke {
            return Ok(());
        }
        Ok(())
    }

    /// 对端给我们发来了数据
    async fn handle_piece(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端给我们发来了数据");
        Ok(())
    }

    /// 对端撤回了刚刚请求的那个分块
    async fn handle_cancel(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端撤回了刚刚请求的那个分块");
        Ok(())
    }
}

impl Runnable for Peer {
    async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.context.cancel_token() => {
                    debug!("peer {} cancelled", self.id);
                    break;
                }
                result = BtResp::new(&mut self.reader) => {
                    match result {
                        Some((msg_type, buf)) => {
                            match self.handle(msg_type, buf).await {
                                Ok(_) => {},
                                Err(e) => {
                                    trace!("处理什么鬼东西出现了错误\t{}", e);
                                }
                            }
                        },
                        None => {
                            trace!("peer 接收无法正确处理的响应");
                        }
                    }
                },
                result = self.channel.1.recv() => {
                    match result {
                        Some(_cmd) => {
                            unimplemented!()
                        }
                        None => {
                            unimplemented!()
                        }
                    }
                }
            }
        }
    }
}

enum State {
    Length,   // 等待长度数据
    MsgType, // 等待消息类型
    Context,  // 等待内容数据
    Finished, // Future 已完成
}

/// bt 响应处理
struct BtResp<'a> {
    read: &'a mut OwnedReadHalf,
    state: State,
    size: usize,
    msg_type: Option<MsgType>,
    length: Option<u32>
}

impl<'a> BtResp<'a> {
    fn new(read: &'a mut OwnedReadHalf) -> Self {
        Self {
            read,
            state: State::Length,
            size: 4,
            msg_type: None,
            length: None,
        }
    }

    fn read(&mut self, size: usize, cx: &mut Context<'_>) -> Option<Poll<Bytes>> {
        if size == 0 {
            return Some(Poll::Ready(Bytes::new()));
        }
        let mut buf = ByteBuffer::new(size);
        let mut binding = buf.as_mut();
        let future = self.read.read_exact(&mut binding);
        match pin!(future).poll(cx) {
            Poll::Ready(Ok(0)) => {
                trace!("客户端主动说bye-bye");
                None
            }
            Poll::Ready(Ok(_len)) => Some(Poll::Ready(Bytes::from_owner(buf))),
            Poll::Ready(Err(e)) => {
                warn!("因神秘力量，和客户端失去了联系\t{}", e);
                None
            }
            Poll::Pending => Some(Poll::Pending),
        }
    }
}

impl Future for BtResp<'_> {
    type Output = Option<(MsgType, Bytes)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let bt_resp = unsafe { self.get_unchecked_mut() };
        let buf = match bt_resp.read(bt_resp.size, cx) {
            Some(Poll::Ready(buf)) => buf,
            Some(Poll::Pending) => {
                trace!("protocol 的数据还没准备好");
                return Poll::Pending;
            }
            None => return Poll::Ready(None),
        };

        match bt_resp.state {
            State::Length => {
                bt_resp.size = 1;
                let length = u32::from_be_slice(&buf[..bt_resp.size]);
                if length > 1 {
                    bt_resp.length = Some(length - 1);    
                }
                bt_resp.state = State::Context;
            }
            State::MsgType => {
                if let Ok(msg_type) = MsgType::try_from(buf[0]) {
                    if let Some(length) = bt_resp.length {
                        bt_resp.size = length as usize;
                        bt_resp.msg_type = Some(msg_type)
                    } else {
                        return Poll::Ready(Some((msg_type, Bytes::new())))
                    }
                } else {
                    error ! ("未知的消息类型\tmsg_type value: {}", buf[0]);
                    return Poll::Ready(None)
                }
            }
            State::Context => {
                return Poll::Ready(Some((bt_resp.msg_type.take().unwrap(), buf)))
            }
            State::Finished => {
                return Poll::Ready(None);
            }
        }
        pin!(bt_resp).poll(cx)
    }
}
