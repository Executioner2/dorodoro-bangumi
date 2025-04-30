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
use crate::collection::FixedQueue;
use crate::command::CommandHandler;
use crate::core::protocol::{
    BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN,
};
use crate::core::runtime::Runnable;
use crate::emitter::Emitter;
use crate::emitter::constant::PEER_PREFIX;
use crate::peer::error::Error;
use crate::peer::error::Error::{
    BitfieldError, HandshakeError, PieceCheckoutError, ResponseDataIncomplete, TryFromError,
};
use crate::peer::future::BtResp;
use crate::peer_manager::gasket::{ExitReason, GasketContext};
use crate::store::Store;
use crate::{datetime, util};
use ahash::HashMap;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::{Bytes, BytesMut};
use error::Result;
use std::cmp::min;
use std::mem;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::channel;
use tracing::{debug, error, info, trace, warn};

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
    _Wait, // 等待新的分块下载
}

pub struct Peer {
    /// 编号
    no: u64,

    /// gasket 共享上下文
    context: GasketContext,

    /// peer 统计信息
    statistics: PeerStatistics,

    /// reader，peer actor 启动后会把这个 reader 消费掉
    reader: Option<OwnedReadHalf>,

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

    /// 当前正在下载的分块
    ///
    /// (分块下标 偏移位置 )
    downloading_pieces: HashMap<u32, u32>,

    /// 成功收到的 piece 的响应次数
    secuessed_piece: u64,

    /// 指令发射器
    emitter: Emitter,

    /// 存储处理
    store: Store,
}

impl Peer {
    pub async fn new(
        no: u64,
        addr: SocketAddr,
        context: GasketContext,
        emitter: Emitter,
        store: Store,
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
            reader: Some(reader),
            writer,
            status: Status::Choke,
            addr,
            opposite_peer_id: None,
            opposite_peer_status: Status::Choke,
            opposite_peer_bitfield: None,
            downloading_pieces: HashMap::default(),
            secuessed_piece: 0,
            emitter,
            store,
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
        let size = self.reader.as_mut().unwrap().read(&mut handshake_resp).await?;
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
        let bitfield = self.context.bytefield();
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
        self.statistics.last_request_time = datetime::now_millis();
        let offset = self.downloading_pieces.entry(piece_index).or_insert(0);

        let over = *offset >= piece_length;
        if !over {
            trace!(
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
            *offset += sharding_size;
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
    pub async fn try_find_downloadable_pices(&mut self) -> Result<bool> {
        if !self.is_can_be_download() {
            return Ok(false);
        }

        let torrent = self.context.torrent();
        let bit_len = torrent.info.pieces.len() / 20;

        for i in 0..bit_len {
            let (idx, offset) = util::bytes::bitmap_offset(i);

            if let Some(true) = self
                .opposite_peer_bitfield
                .as_ref()
                .map(|bytes| bytes[idx] & offset != 0)
            {
                let piece_index = i as u32;
                if let Some(block_offset) = self.context.apply_download_piece(self.no, piece_index)
                {
                    trace!("peer_no [{}] 发现新的可下载分块：{}", self.no, i);
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
        // 下载到本地
        if bytes.len() < 8 {
            return Err(ResponseDataIncomplete);
        }
        let now_time = datetime::now_millis();
        let piece_index = u32::from_be_slice(&bytes[0..4]);
        let block_offset = u32::from_be_slice(&bytes[4..8]);
        let block_data = bytes.split_off(8);
        let block_data_len = block_data.len();
        self.write_file(piece_index, block_offset, block_data)
            .await?;

        self.secuessed_piece += 1;

        let mut over = false;
        if self.request_block(piece_index).await? {
            // 校验分块 hash
            if !self.checkout(piece_index).await? {
                error!("分块校验不通过，中断链接");
                return Err(PieceCheckoutError(piece_index));
            } else {
                info!("第 {} 个分块校验通过", piece_index);
            }
            over = true;
        }

        // 上报给 gasket
        let take_time = 1.max(now_time.saturating_sub(self.statistics.last_request_time)); // 耗时
        let rate = ((block_data_len as u128) * 1000 / take_time) as u64; // 速率（乘以 1000 毫秒，方便计算出字节/秒）
        self.statistics.last_bytes_recv = block_data_len as u64;
        self.statistics.total_bytes_recv += block_data_len as u64;
        self.statistics.download_speed = rate;
        self.statistics.rate_window.push(rate);
        self.context.report_statistics(
            self.no,
            piece_index,
            block_offset,
            block_data_len as u64,
            self.statistics.avg_rate(),
            rate,
            over,
        );

        self.try_find_downloadable_pices().await?;

        Ok(())
    }

    /// 对端撤回了刚刚请求的那个分块
    async fn handle_cancel(&mut self, _bytes: Bytes) -> Result<()> {
        trace!("对端撤回了刚刚请求的那个分块");
        Ok(())
    }

    /// 校验分块
    async fn checkout(&self, piece_index: u32) -> Result<bool> {
        let read_length = self.get_piece_length(piece_index);
        let torrent = self.context.torrent();
        let path = self.context.download_path();
        let hash = &torrent.info.pieces[piece_index as usize * 20..(piece_index as usize + 1) * 20];
        let list = torrent.find_file_of_piece_index(path, piece_index, 0, read_length as usize);
        let res = self.store.checkout(list, hash).await?;
        Ok(res)
    }

    /// 写入文件
    async fn write_file(
        &self,
        piece_index: u32,
        block_offset: u32,
        mut block_data: Bytes,
    ) -> Result<()> {
        let torrent = &self.context.torrent();
        let path = self.context.download_path();

        for block_info in
            torrent.find_file_of_piece_index(path, piece_index, block_offset, block_data.len())
        {
            let data = block_data.split_to(block_info.len);
            self.store.write(block_info, data).await?;
        }

        Ok(())
    }

    pub fn get_transfer_id(no: u64) -> String {
        format!("{}{}", PEER_PREFIX, no)
    }
}

impl Runnable for Peer {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.context.config().channel_buffer());
        let transfer_id = Self::get_transfer_id(self.no);
        self.emitter.register(transfer_id, send);

        trace!("开始监听数据 peer_no:{}\taddr: {}", self.no, self.addr);
        let reason: ExitReason;
        let mut reader = self.reader.take().unwrap();
        let mut bt_resp = BtResp::new(&mut reader, self.addr);
        loop {
            tokio::select! {
                _ = self.context.cancel_token() => {
                    debug!("peer {} cancelled", self.no);
                    reason = ExitReason::Normal;
                    break;
                }
                result = &mut bt_resp => {
                    match result {
                        Some((msg_type, buf)) => {
                            match self.handle(msg_type, buf).await {
                                Ok(_) => {},
                                Err(e) => {
                                    error!("处理响应数据时出现错误\t{}", e);
                                    reason = ExitReason::Exception;
                                    break;
                                }
                            }
                        },
                        None => {
                            warn!("断开了链接，终止 {} - {} 的数据监听", self.no, self.addr);
                            reason = ExitReason::Exception;
                            break;
                        }
                    }
                    bt_resp = BtResp::new(&mut reader, self.addr);
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
                            unimplemented!()
                        }
                    }
                }
            }
        }

        // 归还未下完的分块
        self.context
            .give_back_download_piece(self.no, self.downloading_pieces);
        self.context.peer_exit(self.no, reason).await;
    }
}
