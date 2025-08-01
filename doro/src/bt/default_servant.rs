use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Error, Result};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use doro_util::option_ext::OptionExt;
use doro_util::sync::MutexExt;
use doro_util::{anyhow_eq, anyhow_ge, bytes_util};
use doro_util::global::Id;
use doro_util::bytes_util::Bytes2Int;
use tracing::{trace, warn};

use crate::base_peer::error::{exception, PeerExitReason};
use crate::base_peer::rate_control::{PacketSend, RateControl};
use crate::base_peer::{MsgType, Peer, PeerWrapper, Status};
use crate::context::Context;
use crate::mapper::torrent::PieceStatus;
use crate::servant::{Servant, ServantCallback, ServantContext};
use crate::store::Store;
use crate::task_manager::PeerId;
use crate::torrent::TorrentArc;

pub struct DefaultServantContext {
    peer: PeerWrapper,
}

impl ServantContext for DefaultServantContext {
    fn get_peer(&self) -> &PeerWrapper {
        &self.peer
    }
}

/// 默认的 servant 实现。
#[derive(Default)]
pub struct DefaultServant {
    /// 种子信息哈希值
    info_hash: [u8; 20],

    /// 种子信息
    torrent: OnceLock<TorrentArc>,

    /// peer id
    peer_id: PeerId,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 回调函数
    callback: OnceLock<Arc<dyn ServantCallback>>,

    /// 保存所有连接的 peer
    peers: Arc<DashMap<Id, Peer>>,

    /// 资源保存路径
    save_path: Arc<PathBuf>,
}

impl DefaultServant {
    pub fn new(
        peer_id: PeerId, info_hash: [u8; 20], save_path: Arc<PathBuf>, 
        bytefield: Arc<Mutex<BytesMut>>, underway_bytefield: Arc<DashMap<u32, PieceStatus>>
    ) -> Self {
        Self {
            peer_id,
            info_hash,
            save_path,
            bytefield,
            underway_bytefield,
            ..Default::default()
        }
    }

    pub fn init(&self, callback: impl ServantCallback, torrent: Option<TorrentArc>) {
        self.callback.set(Arc::new(callback))
            .map_err(|_| anyhow!("callback init failed"))
            .unwrap();
        if let Some(torrent) = torrent {
            self.torrent.set(torrent)
            .map_err(|_| anyhow!("torrent init failed"))
            .unwrap();
        }
    }

    fn callback(&self) -> &dyn ServantCallback {
        if let Some(callback) = self.callback.get() {
            callback.as_ref()
        } else {
            panic!("callback not init")
        }
    }

    fn torrent(&self) -> Option<TorrentArc> {
        self.torrent.get().cloned()
    }

    fn get_servant_context(&self, id: Id) -> Option<Box<dyn ServantContext>> {
        if let Some(peer) = self.peers.get(&id) {
            return Some(self.servant_context(peer.value().clone()));
        }
        None
    }

    fn take_servant_context(&self, id: Id) -> Option<Box<dyn ServantContext>> {
        if let Some((_, peer)) = self.peers.remove(&id) {
            return Some(self.servant_context(peer));
        }
        None
    }

    fn servant_context(&self, peer: Peer) -> Box<dyn ServantContext> {
        Box::new(DefaultServantContext {
            peer: peer.wrapper(),
        })
    }

    #[rustfmt::skip]
    fn apply_download_piece(&self, piece_idx: u32) -> Option<(u32, u32)> {
        let mut ret = None;
        let (idx, offset) = bytes_util::bitmap_offset(piece_idx as usize);
        let mut bytefield = self.bytefield.lock_pe();

        if *bytefield.get_mut(idx)? & offset == 0 {
            self.underway_bytefield
               .entry(piece_idx)
               .and_modify(|ps| {
                    if let PieceStatus::Pause(offset) = ps {
                        ret = Some((piece_idx, *offset));
                        *ps = PieceStatus::Ing(*offset)
                    }
               }).or_insert_with(|| {
                    ret = Some((piece_idx, 0));
                    PieceStatus::Ing(0)
               });
        }

        ret
    }

    #[rustfmt::skip]
    fn try_find_downloadable_peice(&self, torrent: &TorrentArc, peer: &Peer) -> Option<(u32, u32)> {
        if !peer.is_can_be_download() {
            return None;
        }

        // 先拿为请求完成的
        let request_pieces = peer.get_request_pieces();
        if let Some(item) = request_pieces.iter().take(1).next() {
            return Some((*item.key(), *item.value()));
        }
        
        let op_bitfield = peer.op_bitfield();
        for piece_idx in 0..torrent.piece_num() {
            let (idx, offset) = bytes_util::bitmap_offset(piece_idx);

            let have = op_bitfield.lock_pe()
                .get(idx)
                .map(|bit| bit & offset != 0);
            if !matches!(have, Some(true)) {
                continue;
            }

            if let Some((piece_idx, offset)) = self.apply_download_piece(piece_idx as u32) {
                let piece_length = torrent.piece_length(piece_idx);
                peer.reset_request_piece_origin(piece_idx, offset, piece_length);
                return Some((piece_idx, offset));
            }
        }

        None
    }

    /// 检查分片的 hash 值是否正确      
    /// 这个方法不会向上抛出错误，因为这里抛出去，会被 BasePeer 捕捉到，然后关闭 peer 连接。
    /// 我们告诉 ServantCallback 实例就好，让它来决定是否关闭这个 peer 连接。
    async fn checkout_piece_hash(&self, peer: Peer, piece_idx: u32) {
        let torrent = self.torrent().unwrap();
        let piece_length = torrent.piece_length(piece_idx);
        let hash = &torrent.info.pieces[piece_idx as usize * 20..(piece_idx as usize + 1) * 20];
        let sc = self.servant_context(peer.clone());
        let block_infos = torrent.find_file_of_piece_index(
            self.save_path.as_path(), piece_idx, 0, piece_length as usize
        );

        match Store::global().checkout(block_infos, hash).await {
            Ok(false) => {
                self.reset_underway_piece(piece_idx);
                self.callback().on_verify_piece_failed(sc, piece_idx, anyhow!("hash 不一致")).await;
            }
            Err(e) => {
                self.reset_underway_piece(piece_idx);
                self.callback().on_verify_piece_failed(sc, piece_idx, e).await;
            }
            Ok(true) => {
                trace!("[{}] 校验分片 [{piece_idx}] 成功", peer.name());
                self.mark_piece_finish(piece_idx);
                self.callback().on_verify_piece_success(sc, piece_idx).await;
            }
        }
    }

    /// 检查分片的 hash 值是否正确      
    /// 这个方法不会向上抛出错误，因为这里抛出去，会被 BasePeer 捕捉到，然后关闭 peer 连接。
    /// 我们告诉 ServantCallback 实例就好，让它来决定是否关闭这个 peer 连接。
    async fn write_file(&self, peer: Peer, piece_idx: u32, block_offset: u32, mut block_data: Bytes) {
        let torrent = self.torrent().unwrap();
        let piece_length = torrent.piece_length(piece_idx);
        let block_infos = torrent.find_file_of_piece_index(
            self.save_path.as_path(), piece_idx, block_offset, block_data.len()
        );

        for block_info in block_infos {
            let data = block_data.split_to(block_info.len);
            let file_path = block_info.filepath.clone();
            if Store::global().write(block_info, data).await.is_err() {
                peer.reset_request_piece_origin(piece_idx, block_offset, piece_length); // 从这个位置重新请求
                let sc = self.servant_context(peer.clone());
                self.callback().on_store_block_failed(
                    sc, piece_idx, block_offset, block_data.len() as u32,
                    anyhow!("write file failed - [{file_path:?}]")
                ).await;
                return;
            }
        }

        self.update_write_piece_offset(piece_idx, block_offset);
        peer.update_resoponse_piece(piece_idx, block_offset);
        if peer.piece_recv_finish(piece_idx) {
            // 在这里就把 peer 里的此分片状态设为完成（即从集合中删掉这个分片的状态）
            // 这样即便 hash 校验不通过，只需要在 servant 层将该分片重置为 Pause(0)
            // 后续有 peer 申请下载分片时，就会把这个分片给它。
            peer.remove_finish_piece(piece_idx); 
            self.checkout_piece_hash(peer, piece_idx).await;
        }
    }

    /// 更新写入分片的偏移量
    fn update_write_piece_offset(&self, piece_idx: u32, block_offset: u32) {
        if let Some(mut item) = self.underway_bytefield.get_mut(&piece_idx) {
            if let PieceStatus::Ing(offset) = item.value_mut() {
                *offset = block_offset;
            }
        }
    }

    /// 从正在进行集合中，删除已经下载完成且校验通过的分片
    fn remove_finish_piece(&self, piece_idx: u32) {
        self.underway_bytefield.remove(&piece_idx);
    }

    /// 重置正在进行中的分片，一般是 hash 校验失败，需要重新开始
    fn reset_underway_piece(&self, piece_idx: u32) {
        self.underway_bytefield.insert(piece_idx, PieceStatus::Pause(0));
    }

    /// 标记分片为已完成
    fn mark_piece_finish(&self, piece_idx: u32) {
        let (idx, offset) = bytes_util::bitmap_offset(piece_idx as usize);
        self.bytefield.lock_pe().get_mut(idx).map_ext(|bit| *bit |= offset);
        self.remove_finish_piece(piece_idx);
    }
}

#[async_trait]
impl Servant for DefaultServant {
    fn info_hash(&self) -> &[u8] {
        self.info_hash.as_ref()
    }

    fn peer_id(&self) -> &[u8] {
        self.peer_id.value().as_ref()
    }

    async fn add_peer(&self, id: Id, peer: Peer) {
        let dsc = self.servant_context(peer.clone());
        if let Err(e) = self.callback().on_handshake_success(dsc).await {
            let dsc = self.servant_context(peer.clone());
            self.callback().on_peer_exit(dsc, exception(e)).await;
            return;
        }
        self.peers.insert(id, peer);
    }

    fn get_peer(&self, id: Id) -> Option<PeerWrapper> {
        self.peers.get(&id).map(|v| v.value().wrapper())
    }

    // 返回已完成的 piece 情况
    fn bytefield(&self) -> Arc<Mutex<BytesMut>> {
        self.bytefield.clone()
    }

    /// 返回正在进行中的 piece
    fn underway_bytefield(&self) -> Arc<DashMap<u32, PieceStatus>> {
        self.underway_bytefield.clone()
    }

    fn servant_callback(&self) -> &dyn ServantCallback {
        self.callback()
    }

    fn reported_read_size(&self, id: Id, read_size: u64) {
        if let Some(dsc) = self.get_servant_context(id) {
            self.callback().reported_read_size(dsc, read_size);
        }
    }

    /// 检查 piece 是否接收完成
    fn check_piece_download_finished(&self) -> bool {
        let torrent = {
            match self.torrent() {
                Some(torrent) => torrent,
                None => return false
            }
        };

        let mut pn = torrent.piece_num() - 1; // 分片下标是从 0 开始的
        let mut last_v = !0u8 << (7 - (pn & 7));
        let bytefield = self.bytefield.lock_pe();
        for v in bytefield.iter().rev() {
            // 从后往前匹配
            if *v != last_v { return false; }
            pn -= 1;
            last_v = u8::MAX;
        }
        true
    }

    async fn request_piece(&self, id: Id) -> Result<()> {
        let peer = self.peers.get(&id)
           .map(|peer| peer.value().clone())
           .ok_or(anyhow!("peer id [{id}] not found"))?;

        let torrent = self.torrent()
            .ok_or(anyhow!("torrent not init"))?;

        while peer.dashbord().inflight() <= peer.dashbord().cwnd() {
            if let Some((piece_idx, block_offset)) = self.try_find_downloadable_peice(&torrent, &peer) {
                let default_block_size = Context::get_config().block_size();
                let piece_length = torrent.piece_length(piece_idx);
                let block_size = default_block_size.min(piece_length - block_offset);
                peer.request_piece(piece_idx, block_offset, block_size).await?;
                peer.dashbord().send(block_size);
                let new_offsett = default_block_size + block_offset;
                if new_offsett < piece_length {
                    peer.get_request_pieces().insert(piece_idx, new_offsett);
                } else {
                    peer.get_request_pieces().remove(&piece_idx);
                }
            } else {
                let sc = self.servant_context(peer.clone());
                self.callback().on_no_pieces_available(sc)?;
                break;
            }
        }
        Ok(())
    }

    async fn happen_exeception(&self, id: Id, error: Error) {
        self.peer_exit(id, exception(error)).await
    }

    async fn handle_heartbeat(&self, id: Id) -> Result<()> {
        trace!("收到了 [{id}] 的心跳包"); // 收到心跳包，什么也不需要做
        Ok(())
    }

    async fn handle(&self, id: Id, msg_type: MsgType, payload: Bytes) -> Result<()> {
        let peer = {
            match self.peers.get(&id) {
                Some(peer) => peer.value().clone(),
                None => return Ok(()),
            }
        };
        match msg_type {
            MsgType::Choke => self.handle_choke(peer),
            MsgType::UnChoke => self.handle_un_choke(peer).await,
            MsgType::Interested => self.handle_interested(peer).await,
            MsgType::NotInterested => self.handle_not_interested(peer).await,
            MsgType::Have => self.handle_have(peer, payload).await,
            MsgType::Bitfield => self.handle_bitfield(peer, payload).await,
            MsgType::Request => self.handle_request(peer, payload).await,
            MsgType::Piece => self.handle_piece(peer, payload).await,
            MsgType::Cancel => self.handle_cancel(peer, payload),
            _ => Ok(()),
        }
    }

    async fn peer_exit(&self, id: Id, reason: PeerExitReason) {
        if let Some(dsc) = self.take_servant_context(id) {
            dsc.get_peer().shutdown().await;
            self.callback().on_peer_exit(dsc, reason).await;
        }
    }
}

impl DefaultServant {
    /// 不让我们请求数据了
    fn handle_choke(&self, peer: Peer) -> Result<()> {
        trace!("[{}] 对端不让我们请求下载数据", peer.name());
        peer.set_status(Status::Choke);
        Ok(())
    }

    /// 让我门请求数据
    async fn handle_un_choke(&self, peer: Peer) -> Result<()> {
        trace!("[{}] 对端告诉我们可以下载数据", peer.name());
        peer.set_status(Status::UnChoke);
        self.callback().request_available(self.servant_context(peer)).await
    }

    /// 对我们的数据感兴趣
    async fn handle_interested(&self, peer: Peer) -> Result<()> {
        trace!("[{}] 对端对我们的数据感兴趣，那我们就允许他下载", peer.name());
        peer.set_op_status(Status::UnChoke);
        peer.request_un_choke().await
    }

    /// 对我们的数据不再感兴趣了
    async fn handle_not_interested(&self, peer: Peer) -> Result<()> {
        trace!("[{}] 对端对我们的数据不再感兴趣了，那就告诉他你别下载了！", peer.name());
        peer.set_op_status(Status::Choke);
        peer.request_choke().await
    }

    /// 对端告诉我们他有新的分块可以下载
    #[rustfmt::skip]
    async fn handle_have(&self, peer: Peer, payload: Bytes) -> Result<()> {
        trace!("[{}] 对端告诉我们他有新的分块可以下载", peer.name());
        anyhow_eq!(payload.len(), 4, "[{}] 对端的 Have 消息长度不正确", peer.name());
        let bit = u32::from_be_slice(&payload[..4]);
        let (piece_idx, block_offset) = bytes_util::bitmap_offset(bit as usize);
        {
            let bitfield = peer.op_bitfield();
            bitfield.lock_pe().get_mut(piece_idx).map_ext(|bit| *bit |= block_offset);
        }
        let sc = self.servant_context(peer);
        self.callback().have_piece_available(sc, piece_idx as u32, block_offset as u32).await
    }

    /// 对端告诉我们他有哪些分块可以下载
    #[rustfmt::skip]
    async fn handle_bitfield(&self, peer: Peer, payload: Bytes) -> Result<()> {
        trace!("[{}] 对端告诉我们他有哪些分块可以下载: {:?}", peer.name(), &payload[..]);
        let torrent = {
            match self.torrent() {
                Some(torrent) => torrent,
                None => {
                    warn!("由于没有种子信息，无法处理对端的 Bitfield 消息");
                    return Ok(());        
                }
            }
        };
        let bit_len = torrent.bitfield_len();

        anyhow_eq!(payload.len(), bit_len,
            "[{}] 对端的 Bitfield 消息长度不正确。预计长度：{}，实际长度：{}",
            peer.name(), bit_len, payload.len()
        );

        let bitfield = peer.op_bitfield();
        *bitfield.lock_pe() = BytesMut::from(payload);
        let sc = self.servant_context(peer);
        self.callback().owner_bitfield(sc, bitfield).await
    }

    /// 向我们请求数据
    async fn handle_request(&self, peer: Peer, _payload: Bytes) -> Result<()> {
        trace!("[{}] 对端向我们请求数据", peer.name());
        if peer.op_status() != Status::UnChoke {
            return Ok(());
        }
        // todo - 处理请求数据
        Ok(())
    }

    /// 对端给我们发来了数据
    async fn handle_piece(&self, peer: Peer, mut payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对端发来的数据", peer.name());
        if self.torrent().is_none() {
            warn!("由于没有种子信息，无法处理对端的 Piece 消息");
            return Ok(());
        }

        anyhow_ge!(payload.len(), 8, "[{}] 对端的 Piece 消息长度不正确", peer.name());

        let piece_idx = u32::from_be_slice(&payload[0..4]);
        let block_offset = u32::from_be_slice(&payload[4..8]);

        if !peer.is_valid_piece_response(piece_idx) {
            return Ok(());
        }

        let block_data = payload.split_off(8);
        let block_size = block_data.len() as u32;

        let sc = self.servant_context(peer.clone());
        self.callback().received_block(sc, piece_idx, block_offset, block_size).await?;

        // 这里直接持久化，不用开线程，因为处理 handle piece 就是一个线程了，与接收以及发送的线程相互独立
        self.write_file(peer, piece_idx, block_offset, block_data).await;
        Ok(())
    }    

    /// 对端撤回了刚刚请求的那个分块
    fn handle_cancel(&self, peer: Peer, _payload: Bytes) -> Result<()> {
        trace!("[{}] 对端撤回了刚刚请求的那个分块", peer.name());
        // todo - 处理对端撤回分块
        Ok(())
    }
}