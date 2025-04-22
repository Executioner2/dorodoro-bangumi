//! 端到端通信。
//!
//! 协议格式：
//! `长度（4字节） | 消息类型（1字节） | 数据（长度-5字节）`
//!
//! 长度的需要加上消息类型的 1 字节。
//!
//! 区块与分片的关系：区块是制作种子文件的单位，而分片是对种子文件进行下载的单位。分片可以有下载器动态控制，分片大小 <= 区块大小。

pub mod command;
mod error;

use crate::buffer::ByteBuffer;
use crate::bytes::Bytes2Int;
use crate::collection::FixedQueue;
use crate::command::CommandHandler;
use crate::core::protocol::{
    BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN,
};
use crate::core::runtime::Runnable;
use crate::datetime;
use crate::emitter::Emitter;
use crate::emitter::constant::PEER_PREFIX;
use crate::peer::error::Error::{HandshakeError, ResponseDataIncomplete, TryFromError};
use crate::peer_manager::gasket::{ExitReason, GasketContext};
use byteorder::{BigEndian, WriteBytesExt};
use bytes::{Bytes, BytesMut};
use error::Result;
use std::cmp::min;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom};
use std::mem;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::channel;
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
    type Error = error::Error;

    fn try_from(value: u8) -> Result<Self> {
        if value <= 8 {
            Ok(unsafe { mem::transmute(value) })
        } else {
            Err(TryFromError)
        }
    }
}

struct PeerStatistics {
    total_bytes_recv: u64,        // 总共接收的字节数
    last_bytes_recv: u64,         // 上次接收的字节数
    last_request_time: u128,      // 上次请求时间
    download_speed: u64,          // 下载速度
    rate_window: FixedQueue<u64>, // 最近 n 次的下载速率
}

impl PeerStatistics {
    fn new() -> Self {
        Self {
            total_bytes_recv: 0,
            last_bytes_recv: 0,
            last_request_time: datetime::now_millis(), // 初始化为当前时间，方便处理一直没有下载的情况
            download_speed: 0,
            rate_window: FixedQueue::new(10),
        }
    }

    /// 计算出平均速率，多少字节/秒
    fn avg_rate(&self) -> u64 {
        self.rate_window.iter().sum::<u64>() / self.rate_window.len() as u64
    }
}

#[derive(Eq, PartialEq, Hash)]
enum Status {
    Choke,
    UnChoke,
    Wait, // 等待新的分块下载
}

pub struct Peer {
    /// 编号
    no: u64,

    /// gasket 共享上下文
    context: GasketContext,

    /// peer 统计信息
    statistics: PeerStatistics,

    /// reader
    reader: OwnedReadHalf,

    /// wrtier
    writer: OwnedWriteHalf,

    /// 当前 peer 的状态
    status: Status,

    /// 对端地址
    addr: SocketAddr,

    /// 对端的 peer id
    opposite_peer_id: Option<[u8; 20]>,

    /// 对端的状态
    opposite_peer_status: Status,

    /// 对端拥有的分块
    opposite_peer_bitfield: Option<BytesMut>,

    /// 当前下载分块的偏移量
    sharding_offset: u64, // 分块偏移量

    /// 当前正在下载的分块
    downloading_piece: Option<u32>,

    /// 指令发射器
    emitter: Emitter,
}

impl Peer {
    pub async fn new(
        no: u64,
        addr: SocketAddr,
        context: GasketContext,
        emitter: Emitter,
    ) -> Option<Self> {
        let stream = match TcpStream::connect(addr).await {
            Ok(stream) => stream,
            Err(_) => return None,
        };
        let (reader, writer) = stream.into_split();
        Some(Peer {
            no,
            context,
            statistics: PeerStatistics::new(),
            reader,
            writer,
            status: Status::Choke,
            addr,
            opposite_peer_id: None,
            opposite_peer_status: Status::Choke,
            opposite_peer_bitfield: None,
            sharding_offset: 0,
            downloading_piece: None,
            emitter,
        })
    }

    /// 握手，然后询问对方有哪些分块是可以下载的
    pub async fn start(&mut self) -> Result<()> {
        // 握手信息
        trace!("发送握手信息 peer_id:{}", self.no);
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
        self.reader.readable().await?;

        let mut handshake_resp = vec![0u8; bytes.len()];
        let size = self.reader.read(&mut handshake_resp).await?;
        // info!("peer_id {} 握手响应长度: {}", self.id, size);
        if size != bytes.len() {
            // error!("握手失败，响应数据长度和发送的不一致，实际响应长度: {}\t具体内容: {:?}", size, handshake_resp);
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

    /// 请求下载分块
    async fn request_block(&mut self, piece_index: u32) -> Result<bool> {
        let sharding_size = self.context.config().sharding_size();
        let torrent = self.context.torrent();
        let piece_length = torrent.info.piece_length;
        let resource_length = torrent.info.length;
        let piece_length =
            piece_length.min(resource_length.saturating_sub(piece_index as u64 * piece_length));
        self.statistics.last_request_time = datetime::now_millis();
        let over = self.sharding_offset >= piece_length;
        if !over {
            trace!(
                "peer_id [{}] 请求下载分块 {} - {}",
                self.no, piece_index, self.sharding_offset
            );
            let mut req = Vec::with_capacity(13);
            WriteBytesExt::write_u32::<BigEndian>(&mut req, 13)?;
            WriteBytesExt::write_u8(&mut req, MsgType::Request as u8)?;
            WriteBytesExt::write_u32::<BigEndian>(&mut req, piece_index)?;
            WriteBytesExt::write_u32::<BigEndian>(&mut req, self.sharding_offset as u32)?;
            WriteBytesExt::write_u32::<BigEndian>(
                &mut req,
                min(piece_length - self.sharding_offset, sharding_size) as u32,
            )?;

            self.writer.write_all(&req).await?;
            self.sharding_offset += sharding_size;
        } else {
            self.sharding_offset = 0;
        }
        Ok(over)
    }

    /// 尝试寻找可以下载的分块进行下载
    pub async fn try_find_downloadable_pices(&mut self) -> Result<bool> {
        let torrent = self.context.torrent();
        let bit_len = torrent.info.pieces.len() / 20;

        for i in 0..bit_len {
            let idx = i / 8;
            let offset = 1 << (i % 8);

            if let Some(true) = self
                .opposite_peer_bitfield
                .as_ref()
                .map(|bytes| bytes[idx] & offset != 0)
            {
                if self
                    .context
                    .apply_download_piece(self.no, idx, offset)
                    .await
                {
                    trace!("peer_id [{}] 发现新的可下载分块：{}", self.no, i);
                    self.downloading_piece = Some(i as u32);
                    self.request_block(i as u32).await?;
                    return Ok(true);
                }
            }
        }

        // 没有分块可以下载了，告诉 gasket
        self.context.report_no_downloadable_piece(self.no).await;
        self.downloading_piece = None;
        Ok(false)
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
        let idx = (bit / 8) as usize;
        let offset = 1 << (bit % 8);

        if self
            .context
            .apply_download_piece(self.no, idx, offset)
            .await
        {
            self.opposite_peer_bitfield
                .as_mut()
                .map(|bytes| bytes[idx] |= offset);
            if self.status == Status::Wait {
                // 正在等新的分块可用，刚好这里来了个新的，那么就直接下载，不用告诉 gasket，有下得更快的想来抢，
                // 就让他告诉 gasket，让 gasket 来决定是否分配给其它 peer
                self.request_block(bit).await?;
            }
        }

        Ok(())
    }

    /// 对端告诉我们他有哪些分块可以下载
    async fn handle_bitfield(&mut self, bytes: Bytes) -> Result<()> {
        trace!("对端告诉我们他有哪些分块可以下载: {:?}", &bytes[..]);
        let torrent = self.context.torrent();
        let bit_len = torrent.info.pieces.len() / 20;
        if bytes.len() != (bit_len + 7) / 8 {
            warn!(
                "远端给到的 bitfield 长度和期望的不一致，期望的: {}\t实际的bytes: {:?}",
                (bit_len + 7) / 8,
                &bytes[..]
            );
            return Ok(());
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
    async fn handle_piece(&mut self, bytes: Bytes) -> Result<()> {
        trace!("peer_id [{}] 收到了对端发来的数据", self.no);
        // 下载到本地
        if bytes.len() < 8 {
            return Err(ResponseDataIncomplete);
        }
        let now_time = datetime::now_millis();
        let piece_index = u32::from_be_slice(&bytes[0..4]);
        let block_offset = u32::from_be_slice(&bytes[4..8]);
        let block_data = &bytes[8..];
        let torrent = self.context.torrent();
        let piece_length = torrent.info.piece_length;
        let file_path = format!("{}{}", self.context.download_path(), torrent.info.name);
        Self::write_file(
            piece_index,
            block_offset,
            block_data,
            file_path,
            piece_length,
        )?;

        // 判断这个分块是否下载完了
        let over = self.request_block(piece_index).await?;

        // 上报给 gasket
        let take_time = 1.max(now_time.saturating_sub(self.statistics.last_request_time)); // 耗时
        let rate = ((block_data.len() as u128) * 1000 / take_time) as u64; // 速率（乘以 1000 毫秒，方便计算出字节/秒）
        self.statistics.last_bytes_recv = block_data.len() as u64;
        self.statistics.total_bytes_recv += block_data.len() as u64;
        self.statistics.download_speed = rate;
        self.statistics.rate_window.push(rate);
        self.context
            .report_statistics(
                self.no,
                piece_index,
                block_offset,
                block_data.len() as u64,
                self.statistics.avg_rate(),
                rate,
                over,
            )
            .await;

        if over {
            // 这个分块下载完了，寻找新的分块下载
            self.try_find_downloadable_pices().await?;
        }

        Ok(())
    }

    /// 对端撤回了刚刚请求的那个分块
    async fn handle_cancel(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端撤回了刚刚请求的那个分块");
        Ok(())
    }

    /// 写入文件
    fn write_file(
        piece_index: u32,
        block_offset: u32,
        block_data: &[u8],
        file: String,
        piece_length: u64,
    ) -> Result<()> {
        let mut file = OpenOptions::new().write(true).create(true).open(file)?;
        trace!(
            "写入文件\t{} - {} - {}偏移量: {}",
            piece_index,
            piece_length,
            block_offset,
            piece_index as u64 * piece_length + block_offset as u64
        );
        file.seek(SeekFrom::Start(
            piece_index as u64 * piece_length + block_offset as u64,
        ))?;
        std::io::prelude::Write::write_all(&mut file, block_data)?;
        std::io::prelude::Write::flush(&mut file)?;
        Ok(())
    }

    fn get_transfer_id(&self) -> String {
        format!("{}{}", PEER_PREFIX, self.no)
    }
}

impl Runnable for Peer {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.context.config().channel_buffer());
        let transfer_id = self.get_transfer_id();
        self.emitter.register(transfer_id, send).await.unwrap();

        trace!("开始监听数据 peer_id:{}\taddr: {}", self.no, self.addr);
        loop {
            tokio::select! {
                _ = self.context.cancel_token() => {
                    debug!("peer {} cancelled", self.no);
                    break;
                }
                result = BtResp::new(&mut self.reader, self.addr) => {
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
                            // trace!("peer 接收无法正确处理的响应");
                            error!("断开了链接，终止 {} - {} 的数据监听", self.no, self.addr);
                            break;
                        }
                    }
                },
                result = recv.recv() => {
                    match result {
                        Some(cmd) => {
                            let cmd: command::Command = cmd.instance();
                            cmd.handle(()).await; // todo - context
                        }
                        None => {
                            unimplemented!()
                        }
                    }
                }
            }
        }

        // 归还未下完的分块
        if let Some(bit) = self.downloading_piece {
            self.context.give_back_download_piece(self.no, bit).await;
        }
        self.context.peer_exit(self.no, ExitReason::Normal).await;
    }
}

enum State {
    Length,   // 等待长度数据
    MsgType,  // 等待消息类型
    Content,  // 等待内容数据
    Finished, // Future 已完成
}

/// bt 响应处理
struct BtResp<'a> {
    read: &'a mut OwnedReadHalf,
    state: State,
    msg_type: Option<MsgType>,
    length: Option<u32>,
    addr: SocketAddr,
    buf: ByteBuffer,
    read_count: usize,
}

impl<'a> BtResp<'a> {
    fn new(read: &'a mut OwnedReadHalf, addr: SocketAddr) -> Self {
        Self {
            read,
            state: State::Length,
            msg_type: None,
            length: None,
            addr,
            buf: ByteBuffer::new(4),
            read_count: 0,
        }
    }

    fn read(&mut self, cx: &mut Context<'_>, addr: SocketAddr) -> Option<Poll<Bytes>> {
        if self.buf.len() == 0 {
            return Some(Poll::Ready(Bytes::new()));
        }

        loop {
            let mut read_buf = ReadBuf::new(&mut self.buf[self.read_count..]);
            match Pin::new(&mut self.read).poll_read(cx, &mut read_buf) {
                Poll::Ready(Ok(())) => {
                    let filled = read_buf.filled().len();
                    if filled == 0 {
                        trace!("客户端主动说bye-bye");
                        return None;
                    }
                    self.read_count += filled;
                    if self.read_count >= self.buf.capacity() {
                        let mut buf = ByteBuffer::new(0);
                        mem::swap(&mut self.buf, &mut buf);
                        self.read_count = 0;
                        return Some(Poll::Ready(Bytes::from_owner(buf)));
                    }
                }
                Poll::Ready(Err(e)) => {
                    error!("因神秘力量，和客户端失去了联系\t{}，addr: {}", e, addr);
                    return None;
                }
                Poll::Pending => return Some(Poll::Pending),
            }
        }
    }
}

impl Future for BtResp<'_> {
    type Output = Option<(MsgType, Bytes)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let bt_resp = unsafe { self.get_unchecked_mut() };
        let buf = match bt_resp.read(cx, bt_resp.addr) {
            Some(Poll::Ready(buf)) => buf,
            Some(Poll::Pending) => {
                trace!("protocol 的数据还没准备好");
                return Poll::Pending;
            }
            None => return Poll::Ready(None),
        };

        match bt_resp.state {
            State::Length => {
                let length = u32::from_be_slice(&buf[..4]);
                if length > 1 {
                    bt_resp.length = Some(length - 1);
                }
                bt_resp.buf = ByteBuffer::new(1);
                bt_resp.state = State::MsgType;
            }
            State::MsgType => {
                if let Ok(msg_type) = MsgType::try_from(buf[0]) {
                    trace!("取得消息类型: {:?}", msg_type);
                    if let Some(length) = bt_resp.length {
                        bt_resp.buf = ByteBuffer::new(length as usize);
                        bt_resp.msg_type = Some(msg_type);
                        bt_resp.state = State::Content;
                    } else {
                        return Poll::Ready(Some((msg_type, Bytes::new())));
                    }
                } else {
                    error!("未知的消息类型\tmsg_type value: {}", buf[0]);
                    return Poll::Ready(None);
                }
            }
            State::Content => {
                bt_resp.state = State::Finished;
                return Poll::Ready(Some((bt_resp.msg_type.take().unwrap(), buf)));
            }
            State::Finished => {
                return Poll::Ready(None);
            }
        }
        pin!(bt_resp).poll(cx)
    }
}
