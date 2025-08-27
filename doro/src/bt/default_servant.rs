use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use anyhow::{anyhow, Error, Result};
use async_trait::async_trait;
use bendy::decoding::FromBencode;
use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use doro_util::option_ext::OptionExt;
use doro_util::sync::{MutexExt, ReadLockExt, WriteLockExt};
use doro_util::{anyhow_eq, anyhow_ge, anyhow_le, bytes_util};
use doro_util::global::Id;
use doro_util::bytes_util::Bytes2Int;
use tracing::{debug, info, trace, warn};

use crate::base_peer::error::{exception, PeerExitReason};
use crate::base_peer::rate_control::{PacketSend, RateControl};
use crate::base_peer::{ExtendedId, MsgType, Peer, PeerWrapper, Status};
use crate::context::Context;
use crate::default_servant::extend_data::{HandshakeData, Metadata, MetadataType};
use crate::mapper::torrent::PieceStatus;
use crate::servant::{Servant, ServantCallback, ServantContext};
use crate::store::Store;
use crate::task_manager::PeerId;
use crate::torrent::TorrentArc;

pub mod coordinator;
pub mod extend_data;

/// 尝试获取 torrent 信息，如果没有，则打印警告信息并返回 None
macro_rules! try_get_torrent {
    ($self: expr, $warn_msg: literal) => {
        match $self.torrent.as_ref() {
            Some(torrent) => torrent,
            None => {
                warn!($warn_msg);
                return Ok(());
            }
        }
    };
    ($self: expr) => {
        match $self.torrent.as_ref() {
            Some(torrent) => torrent,
            None => {
                return false;
            }
        }
    };
}

/// 尝试获取 callback 实例，如果没有，则直接 return Ok(())
macro_rules! try_get_callback {
    ($self: expr) => {
        match $self.callback() {
            Some(callback) => callback,
            None => {
                return Ok(());
            }
        }
    };
}

pub struct DefaultServantContext {
    peer: PeerWrapper,
}

impl ServantContext for DefaultServantContext {
    fn get_peer(&self) -> &PeerWrapper {
        &self.peer
    }
}

#[derive(Default)]
pub struct DefaultServantBuilder {
    // 种子信息哈希值
    info_hash: [u8; 20],

    /// peer id
    peer_id: PeerId,

    /// 种子信息
    torrent: Option<TorrentArc>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 回调函数
    callback: OnceLock<Weak<dyn ServantCallback>>,

    /// 资源保存路径
    save_path: Arc<PathBuf>,

    /// piece 下载是否完成
    piece_finished: bool,
}

impl DefaultServantBuilder {
    pub fn new(info_hash: [u8; 20], peer_id: PeerId) -> Self {
        Self {
            info_hash,
            peer_id,
            bytefield: Arc::new(Mutex::new(BytesMut::new())),
            underway_bytefield: Arc::new(DashMap::new()),
            save_path: Arc::new(Context::get_config().default_download_dir()),
            piece_finished: false,
            ..Default::default()
        }
    }

    pub fn build(self) -> DefaultServant {
        DefaultServant {
            info_hash: self.info_hash,
            torrent: self.torrent,
            peer_id: self.peer_id,
            bytefield: self.bytefield,
            underway_bytefield: self.underway_bytefield,
            callback: self.callback,
            peers: Arc::new(DashMap::new()),
            save_path: self.save_path,
            piece_finished: Arc::new(AtomicBool::new(self.piece_finished)),
        }
    }

    pub fn arc_build(self) -> Arc<DefaultServant> {
        Arc::new(self.build())
    } 

    pub fn set_torrent(mut self, torrent: TorrentArc) -> Self {
        self.torrent = Some(torrent);
        self
    }

    pub fn set_bytefield(mut self, bytefield: Arc<Mutex<BytesMut>>) -> Self {
        self.bytefield = bytefield;
        self
    }

    pub fn set_underway_bytefield(mut self, underway_bytefield: Arc<DashMap<u32, PieceStatus>>) -> Self {
        self.underway_bytefield = underway_bytefield;
        self
    }

    pub fn set_callback(self, callback: Weak<impl ServantCallback>) -> Self {
        self.callback.set(callback).unwrap();
        self
    }

    pub fn set_save_path(mut self, save_path: Arc<PathBuf>) -> Self {
        self.save_path = save_path;
        self
    }

    pub fn set_piece_finished(mut self, piece_finished: bool) -> Self {
        self.piece_finished = piece_finished;
        self
    }
}

/// 默认的 servant 实现。
pub struct DefaultServant {
    /// 种子信息哈希值
    info_hash: [u8; 20],

    /// 种子信息
    torrent: Option<TorrentArc>,

    /// peer id
    peer_id: PeerId,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 回调函数
    callback: OnceLock<Weak<dyn ServantCallback>>,

    /// 保存所有连接的 peer
    peers: Arc<DashMap<Id, Peer>>,

    /// 资源保存路径
    save_path: Arc<PathBuf>,

    /// piece 下载是否完成
    piece_finished: Arc<AtomicBool>,
}

impl Drop for DefaultServant {
    fn drop(&mut self) {
        info!("DefaultServant 已 drop");
    }
}

impl DefaultServant {
    pub fn set_callback(&self, callback: Weak<impl ServantCallback>) {
        self.callback.set(callback)
            .map_err(|_| anyhow!("callback init failed"))
            .unwrap();
    }

    fn callback(&self) -> Option<Arc<dyn ServantCallback>> {
        if let Some(callback) = self.callback.get() {
            callback.upgrade()
        } else {
            panic!("callback not init")
        }
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
        let (idx, offset) = bytes_util::bitmap_offset(piece_idx);
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
        let default_block_size = Context::get_config().block_size();

        // 先拿未请求完成的
        let request_pieces = peer.get_request_pieces();
        let mut ret = None;
        for mut item in request_pieces.iter_mut() {
            let piece_length = torrent.piece_length(*item.key());
            if *item.value() < piece_length {
                ret = Some((*item.key(), *item.value()));
                *item.value_mut() += default_block_size;
                break;
            }
        }
        
        if ret.is_some() {
            let key = ret.unwrap().0;
            let piece_length = torrent.piece_length(key);
            request_pieces.remove_if(&key, |_, v| *v >= piece_length);
            return ret;
        }

        let op_bitfield = peer.op_bitfield();
        for piece_idx in 0..torrent.piece_num() {
            let (idx, offset) = bytes_util::bitmap_offset(piece_idx as u32);

            let have = op_bitfield.read_pe()
                .get(idx)
                .map(|bit| bit & offset != 0);
            if !matches!(have, Some(true)) {
                continue;
            }

            if let Some((piece_idx, offset)) = self.apply_download_piece(piece_idx as u32) {
                let piece_length = torrent.piece_length(piece_idx);
                peer.reset_request_piece_origin(piece_idx, offset, piece_length);
                peer.get_request_pieces().get_mut(&piece_idx).map_ext(|mut v| *v += default_block_size);
                return Some((piece_idx, offset));
            }
        }

        None
    }

    /// 检查分片的 hash 值是否正确      
    /// 这个方法不会向上抛出错误，因为这里抛出去，会被 BasePeer 捕捉到，然后关闭 peer 连接。
    /// 我们告诉 ServantCallback 实例就好，让它来决定是否关闭这个 peer 连接。
    async fn checkout_piece_hash(&self, peer: Peer, piece_idx: u32, callback: Arc<dyn ServantCallback>) {
        let torrent = self.torrent.as_ref().unwrap();
        let piece_length = torrent.piece_length(piece_idx);
        let hash = &torrent.info.pieces[piece_idx as usize * 20..(piece_idx as usize + 1) * 20];
        let sc = self.servant_context(peer.clone());
        let block_infos = torrent.find_file_of_piece_index(
            self.save_path.as_path(), piece_idx, 0, piece_length as usize
        );

        match Store::global().checkout(block_infos, hash).await {
            Ok(false) => {
                self.reset_underway_piece(piece_idx);
                callback.on_verify_piece_failed(sc, piece_idx, anyhow!("hash 不一致")).await;
            }
            Err(e) => {
                self.reset_underway_piece(piece_idx);
                callback.on_verify_piece_failed(sc, piece_idx, e).await;
            }
            Ok(true) => {
                trace!("[{}] 校验分片 [{piece_idx}] 成功", peer.name());
                self.mark_piece_finish(piece_idx);
                callback.on_verify_piece_success(sc, piece_idx).await;
            }
        }
    }

    /// 检查分片的 hash 值是否正确      
    /// 这个方法不会向上抛出错误，因为这里抛出去，会被 BasePeer 捕捉到，然后关闭 peer 连接。
    /// 我们告诉 ServantCallback 实例就好，让它来决定是否关闭这个 peer 连接。
    async fn write_file(&self, peer: Peer, piece_idx: u32, block_offset: u32, mut block_data: Bytes, callback: Arc<dyn ServantCallback>) {
        let real_offset = match peer.get_seq_last_piece_block_offset(piece_idx) {
            Some(real_offset) => real_offset,
            None => {
                warn!("[{}] 找不到连续的 offset 位置 [{piece_idx}] - [{block_offset}]", peer.name());
                return;
            }
        };

        let torrent = self.torrent.as_ref().unwrap();
        let piece_length = torrent.piece_length(piece_idx);
        let block_infos = torrent.find_file_of_piece_index(
            self.save_path.as_path(), piece_idx, block_offset, block_data.len()
        );

        let block_size = block_data.len();
        for block_info in block_infos {
            let data = block_data.split_to(block_info.len);
            let file_path = block_info.filepath.clone();
            if Store::global().write(block_info, data).await.is_err() {
                peer.reset_request_piece_origin(piece_idx, block_offset, piece_length); // 写入失败，从这个位置重新请求
                let sc = self.servant_context(peer.clone());
                callback.on_store_block_failed(
                    sc, piece_idx, block_offset, block_data.len() as u32,
                    anyhow!("write file failed - [{file_path:?}]")
                ).await;
                return;
            }
        }

        self.update_write_piece_offset(piece_idx, real_offset);
        
        // 注意，一定要在获取 real_offset 之后更新，否则会出现【线程1】走到了下面的 piece_recv.finish，
        // 原本还差【线程2】的数据，但是【线程2】在【线程1】之前更新了，然后【线程1】刚好又把这个完成的 piece
        // 给删掉了，就会导致【线程2】无法再拿到 real_offset。
        peer.update_resoponse_piece(piece_idx, block_offset); 
        callback.on_store_block_success(
            self.servant_context(peer.clone()), piece_idx, block_offset, block_size as u32
        ).await;
        if peer.piece_recv_finish(piece_idx) {
            // 在这里就把 peer 里的此分片状态设为完成（即从集合中删掉这个分片的状态）
            // 这样即便 hash 校验不通过，只需要在 servant 层将该分片重置为 Pause(0)
            // 后续有 peer 申请下载分片时，就会把这个分片给它。
            peer.remove_finish_piece(piece_idx);
            self.checkout_piece_hash(peer, piece_idx, callback).await;
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
        let (idx, offset) = bytes_util::bitmap_offset(piece_idx);
        self.bytefield.lock_pe().get_mut(idx).map_ext(|bit| *bit |= offset);
        self.remove_finish_piece(piece_idx);
    }

    /// 获取正在进行中或者暂停的分片的 idx
    fn get_piece_status_idx(&self, piece_idx: u32) -> Option<u32> {
        if let Some(status) = self.underway_bytefield.get(&piece_idx) {
            return match status.value() {
                PieceStatus::Ing(offset) => Some(*offset),
                PieceStatus::Pause(offset) => Some(*offset),
            }
        }
        None
    }

    /// 设置正在进行中的分片的状态为 Pause
    fn set_piece_status_pause(&self, piece_idx: u32) {
        if let Some(mut status) = self.underway_bytefield.get_mut(&piece_idx) {
            let i = match status.value() {
                PieceStatus::Ing(offset) => offset,
                PieceStatus::Pause(offset) => offset,
            };
            *status.value_mut() = PieceStatus::Pause(*i);
        }
    }

    /// 尝试抢夺慢速的 peer 的 piece 下载任务
    async fn try_loot_slow_peer(&self, peer: Peer, torrent: &TorrentArc) -> Result<()> {
        if self.check_piece_download_finished() {
            let dsc = self.servant_context(peer);
            try_get_callback!(self).on_piece_download_finished(dsc).await;
            return Ok(());
        }

        if self.try_free_slow_piece(&peer, torrent).await {
            // 这里能够释放掉慢速 piece，那么说明是成功将其分配给了当前 peer 的
            // 那么接下来再次请求 piece，应该不会再走到这里，所以最多递归两层才
            // 是符合预期的。
            self.request_piece(peer.get_id()).await
        } else {
            // let mut peers = self.peers
            // .iter().filter(|item| *item.key() != peer.get_id())
            // .map(|item| (item.value().name(), item.dashbord().bw(), item.get_response_pieces().len()))
            // .collect::<Vec<_>>();
            // peers.sort_unstable_by_key(|peer| peer.1);
            // let println = {
            //     let mut str = String::new();
            //     peers.iter().for_each(|peer| {
            //         let rate = doro_util::net::rate_formatting(peer.1);
            //         str.push_str(&format!("{} - [{}]: {:.2}{}\n", peer.0, peer.2, rate.0, rate.1));
            //     });
            //     str
            // };
            // let re_bit = {
            //     let mut str = String::new();
            //     if peers.is_empty() {
            //         let bytefield = self.bytefield.lock_pe();
            //         for i in 0..torrent.piece_num() {
            //             let (idx, offset) = bytes_util::bitmap_offset(i as u32);
            //             let have = bytefield.get(idx).map(|bit| bit & offset != 0).unwrap_or(false);
            //             if !have {
            //                 let ub = self.underway_bytefield.get(&(i as u32));
            //                 str.push_str(&format!("{i} - 未完成，ub 状态: {:?}\n", ub.map(|ub| ub.value().clone())));
            //             }
            //         }
            //     }
            //     str
            // };
            // let rate = doro_util::net::rate_formatting(peer.dashbord().bw());
            // info!("[{}] 尝试抢夺慢速 peer 的 piece 下载任务失败\tresponse_pieces: [{}]bw: [{:.2}{}]\npeers: [{println}]\n还没下载完的 piece: [{re_bit}]",
            // peer.name(), peer.get_response_pieces().len(), rate.0, rate.1);
            if !peer.has_transfer_data() { 
                trace!("[{}] 关闭 peer 连接，因为没有传输数据", peer.name());
                // 已经没有传输中的数据了，那么就关闭这个 peer 连接
                self.peer_exit(peer.get_id(), PeerExitReason::NotHasJob).await;
            }
            Ok(())
        }
    }

    /// 尝试释放掉下载的慢，并且当前 peer 可以下载的 piece  
    /// 如果释放成功，则返回 true，否则返回 false       
    /// 保证同一个 peer 线程安全
    async fn try_free_slow_piece(&self, peer: &Peer, torrent: &TorrentArc) -> bool {
        // 抢夺上锁，避免并发抢夺时，状态不一致，而导致没有抢到的线程把 peer 关闭掉
        // 再因为抢夺时，ub 状态不会改为 Pause，然后抢到了还没开始，没抢到的线程把
        // 这个 peer 关掉后，抢到的 ub 无法归还（因为无法再从 peers 集合中找到它了）
        let sync_lock = peer.get_sync_lock();
        let _lock = sync_lock.lock().await;

        let mut peers = self.peers
            .iter().filter(|item| *item.key() != peer.get_id())
            .map(|item| item.value().wrapper())
            .collect::<Vec<_>>();
        peers.sort_unstable_by_key(|peer| peer.dashbord().bw());
        let mut free_piece = vec![];

        let bw = peer.dashbord().bw();
        for p in peers.iter() {
            if !coordinator::faster(bw, p.dashbord().bw()) {
                break;
            }
            
            // 从受害者（笑）的响应暂存集合中，抢走加害者可以下载的 piece
            p.get_response_pieces().retain(|piece_idx, _| {
                let (idx, ost) = bytes_util::bitmap_offset(*piece_idx);
                let ret = peer.op_bitfield().read_pe().get(idx)
                    .map(|bit| bit & ost != 0).unwrap_or(false);
                if ret {
                    // 不需要设置暂停，避免设置暂停后，被其他 peer 抢过去
                    let offset = self.get_piece_status_idx(*piece_idx).unwrap();
                    p.get_request_pieces().remove(piece_idx);
                    free_piece.push((*piece_idx, offset));
                }
                !ret
            });

            if !p.has_transfer_data() {
                // 受害者已经没有传输中的数据了，那么就关闭这个 peer 连接
                trace!("[{}] 关闭 peer 连接，因为它的任务已经被其它 peer 抢夺", p.name());
                self.peer_exit(p.get_id(), PeerExitReason::NotHasJob).await;
            }

            if !free_piece.is_empty() {
                for (piece_idx, offset) in free_piece {
                    peer.reset_request_piece_origin(piece_idx, offset, torrent.piece_length(piece_idx));
                }
                return true;
            }
        }

        false
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
        let callback = match self.callback() {
            Some(callback) => callback,
            None => return, // 直接返回也行，相当于刚创建出 peer 就销毁了
        };

        if let Err(e) = callback.on_handshake_success(dsc).await {
            let dsc = self.servant_context(peer.clone());
            callback.on_peer_exit(dsc, exception(e)).await;
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

    fn reported_read_size(&self, id: Id, read_size: u64) {
        if let Some(dsc) = self.get_servant_context(id) {
            if let Some(callback) = self.callback() {
                callback.reported_read_size(dsc, read_size);
            }
        }
    }

    /// 检查 piece 是否接收完成
    fn check_piece_download_finished(&self) -> bool {
        let torrent = try_get_torrent!(self);
        if self.piece_finished.load(Ordering::SeqCst) {
            return true;
        }

        let mut pn = torrent.piece_num() - 1; // 分片下标是从 0 开始的
        let mut last_v = !0u8 << (7 - (pn & 7));
        let bytefield = self.bytefield.lock_pe();
        for v in bytefield.iter().rev() {
            // 从后往前匹配
            if *v != last_v { return false; }
            pn -= 1;
            last_v = u8::MAX;
        }
        self.piece_finished.store(true, Ordering::SeqCst);
        true
    }

    async fn request_piece(&self, id: Id) -> Result<()> {
        let torrent = try_get_torrent!(self, "由于没有种子信息，无法进行 piece 请求");
        let peer = self.peers.get(&id)
           .map(|peer| peer.value().clone())
           .ok_or(anyhow!("peer id [{id}] not found"))?;

        if !peer.is_can_be_download() {
            return Ok(());
        }

        let sync_lock = peer.get_sync_lock();
        let lock = sync_lock.lock().await;
        while peer.has_send_window_space() {
            if let Some((piece_idx, block_offset)) = self.try_find_downloadable_peice(torrent, &peer) {
                let default_block_size = Context::get_config().block_size();
                let piece_length = torrent.piece_length(piece_idx);
                let block_size = default_block_size.min(piece_length - block_offset);
                peer.request_piece(piece_idx, block_offset, block_size).await?;
                peer.dashbord().send(block_size);
            } else {
                if peer.dashbord().inflight() == 0 && !peer.get_response_pieces().is_empty() && peer.get_request_pieces().is_empty() { 
                    // 响应丢失，没有传输的数据，但是有等待响应的数据，
                    // 同时请求记录又没了的情况。那么重置请求记录
                    let pieces = peer.get_response_pieces().iter().map(|piece| {
                        (*piece.key(), piece.block_offset())
                    }).collect::<Vec<_>>();
                    for (piece_idx, block_offset) in pieces {
                        peer.reset_request_piece_origin(piece_idx, block_offset, torrent.piece_length(piece_idx));    
                    }
                    continue;
                }
                drop(lock); // 释放锁，避免重入死锁
                self.try_loot_slow_peer(peer, torrent).await?;
                break;
            }
        }
        Ok(())
    }

    async fn request_metadata_piece(&self, id: Id) -> Result<()> {
        let peer = self.peers.get(&id)
            .map(|peer| peer.value().clone())
            .ok_or(anyhow!("peer id [{id}] not found"))?;

        let piece = peer.get_metadata_piece_idx();
        peer.request_metadata_piece(piece).await
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
            MsgType::ExtensionProtocol => self.handle_extension_protocol(peer, payload).await,
            _ => Ok(()),
        }
    }

    async fn peer_exit(&self, id: Id, reason: PeerExitReason) {
        trace!("peer [{id}] 退出，原因：{reason:?}");
        if let Some(dsc) = self.take_servant_context(id) {
            let peer = dsc.get_peer();
            peer.shutdown().await;

            // 归还进行中的分片
            for piece_idx in peer.get_response_pieces().iter() {
                self.set_piece_status_pause(*piece_idx.key());
            }

            if let Some(callback) = self.callback() {
                callback.on_peer_exit(dsc, reason).await;
            }
        } else {
            debug!("没有找到这个 peer - [{id}] 的上下文信息，无法退出");
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

    /// 让我们请求数据
    async fn handle_un_choke(&self, peer: Peer) -> Result<()> {
        trace!("[{}] 对端告诉我们可以下载数据", peer.name());
        // info!("[{}] 对端告诉我们可以下载数据", peer.name());
        peer.set_status(Status::UnChoke);
        try_get_callback!(self).request_available(self.servant_context(peer)).await
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
        let (piece_idx, block_offset) = bytes_util::bitmap_offset(bit);
        {
            let bitfield = peer.op_bitfield();
            bitfield.write_pe().get_mut(piece_idx).map_ext(|bit| *bit |= block_offset);
        }
        let sc = self.servant_context(peer);
        try_get_callback!(self).have_piece_available(sc, piece_idx as u32, block_offset as u32).await
    }

    /// 对端告诉我们他有哪些分块可以下载
    #[rustfmt::skip]
    async fn handle_bitfield(&self, peer: Peer, payload: Bytes) -> Result<()> {
        trace!("[{}] 对端告诉我们他有哪些分块可以下载: {:?}", peer.name(), &payload[..]);
        // info!("[{}] 对端告诉我们他有哪些分块可以下载", peer.name());
        let torrent = try_get_torrent!(self, "由于没有种子信息，无法处理对端的 Bitfield 消息");
        let bit_len = torrent.bitfield_len();

        anyhow_eq!(payload.len(), bit_len,
            "[{}] 对端的 Bitfield 消息长度不正确。预计长度：{}，实际长度：{}",
            peer.name(), bit_len, payload.len()
        );

        peer.set_op_bitfield(BytesMut::from(payload));
        let bitfield = peer.op_bitfield();
        let sc = self.servant_context(peer);
        try_get_callback!(self).owner_bitfield(sc, bitfield).await
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
        try_get_torrent!(self, "由于没有种子信息，无法处理对端的 Piece 消息");
        anyhow_ge!(payload.len(), 8, "[{}] 对端的 Piece 消息长度不正确", peer.name());

        let piece_idx = u32::from_be_slice(&payload[0..4]);
        let block_offset = u32::from_be_slice(&payload[4..8]);

        if !peer.is_valid_piece_response(piece_idx) {
            return Ok(());
        }

        let block_data = payload.split_off(8);
        let block_size = block_data.len() as u32;

        let sc: Box<dyn ServantContext> = self.servant_context(peer.clone());
        let callback = try_get_callback!(self);
        callback.received_block(sc, piece_idx, block_offset, block_size).await?;

        // 这里直接持久化，不用开线程，因为处理 handle piece 就是一个线程了，与接收以及发送的线程相互独立
        self.write_file(peer, piece_idx, block_offset, block_data, callback).await;
        Ok(())
    }    

    /// 对端撤回了刚刚请求的那个分块
    fn handle_cancel(&self, peer: Peer, _payload: Bytes) -> Result<()> {
        trace!("[{}] 对端撤回了刚刚请求的那个分块", peer.name());
        // todo - 处理对端撤回分块
        Ok(())
    }

    /// 收到了对方的扩展协议
    async fn handle_extension_protocol(&self, peer: Peer, mut payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议", peer.name());
        anyhow_ge!(payload.len(), 1, "[{}] 对端的扩展消息长度不正确", peer.name());

        let source_id = payload[0];
        let payload = payload.split_off(1);
        match peer.extend_id_transform(source_id) {
            Some(val) if val == ExtendedId::Handshake.get_value() => {
                self.handle_handshake(peer, payload).await
            },
            Some(val) if val == ExtendedId::UtPex.get_value() => {
                self.handle_ut_pex(peer, payload).await
            },
            Some(val) if val == ExtendedId::UtMetadata.get_value() => {
                self.handle_ut_metadata(peer, payload).await
            },
            Some(val) if val == ExtendedId::UploadOnly.get_value() => {
                self.handle_upload_only(peer, payload).await
            },
            Some(val) if val == ExtendedId::UtHolepunch.get_value() => {
                self.handle_ut_holepunch(peer, payload).await
            },
            Some(val) if val == ExtendedId::UtComment.get_value() => {
                self.handle_ut_comment(peer, payload).await
            },
            _ =>  {
                warn!("[{}] 对端的扩展协议 ID - [{}] 未知", peer.name(), source_id);
                Ok(())
            }
        }
    }
}

/// 处理扩展协议的实现
impl DefaultServant {
    /// 处理扩展协议的握手
    async fn handle_handshake(&self, peer: Peer, payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议握手", peer.name());
        let handshake_data = HandshakeData::from_bencode(&payload)
            .map_err(|e| anyhow!("[{}] 解析对方的扩展协议握手数据失败：{}", peer.name(), e))?;

        for (name, source_id) in &handshake_data.m {
            if let Some(target_id) = ExtendedId::from_name(name) {
                peer.add_extend_id(*source_id, target_id);
            } else {
                warn!("[{}] 对方的扩展协议名称 - [{}] 未知", peer.name(), name);
            }
        }

        peer.set_extend_handshake_data(handshake_data)?;
        try_get_callback!(self).on_extend_handshake_success(self.servant_context(peer)).await
    }

    /// todo - 处理扩展协议的 ut_pex
    async fn handle_ut_pex(&self, peer: Peer, _payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 ut_pex", peer.name());
        Ok(())
    }

    /// 处理扩展协议的 ut_metadata
    async fn handle_ut_metadata(&self, peer: Peer, mut payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 ut_metadata", peer.name());
        let (metadata, offset) = Metadata::from_bencode_first(&payload)
            .map_err(|e| anyhow!("[{}] 解析对方的扩展协议 ut_metadata 数据失败：{}", peer.name(), e))?;

        match metadata.msg_type {
            MetadataType::Request => self.handle_ut_metadata_request(peer, metadata).await,
            MetadataType::Data => self.handle_ut_metadata_data(peer, metadata, payload.split_off(offset)).await,
            MetadataType::Reject => self.handle_ut_metadata_reject(peer, metadata).await,
        }
    }

    /// todo - 处理扩展协议的 ut_metadata request
    async fn handle_ut_metadata_request(&self, peer: Peer, _metadata: Metadata) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 ut_metadata 请求", peer.name());
        Ok(())
    }

    /// 处理扩展协议的 ut_metadata data
    async fn handle_ut_metadata_data(&self, peer: Peer, metadata: Metadata, payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 ut_metadata 数据", peer.name());
        let metadata_size_limit = Context::get_config().metadata_size_limit();
        anyhow_le!(metadata.total_size.unwrap(), metadata_size_limit,
            "[{}] 对方的扩展协议 ut_metadata 数据过大，超过限制：{}", peer.name(), metadata_size_limit
        );

        peer.init_metadata_piece(metadata.total_size.unwrap() as usize);
        peer.set_metadata_piece(metadata.piece, payload)?;

        let is_complete = peer.is_metadata_complete();
        let sc = self.servant_context(peer);
        if is_complete {
            try_get_callback!(self).on_metadata_complete(sc).await
        } else {
            try_get_callback!(self).on_metadata_piece_write(sc).await
        }
    }

    /// todo - 处理扩展协议的 ut_metadata reject
    async fn handle_ut_metadata_reject(&self, peer: Peer, _metadata: Metadata) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 ut_metadata 拒绝", peer.name());
        Ok(())
    }

    /// todo - 处理扩展协议的 upload_only
    async fn handle_upload_only(&self, peer: Peer, _payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 upload_only", peer.name());
        Ok(())
    }

    /// todo - 处理扩展协议的 ut_holepunch
    async fn handle_ut_holepunch(&self, peer: Peer, _payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 ut_holepunch", peer.name());
        Ok(())
    }

    /// todo - 处理扩展协议的 ut_comment
    async fn handle_ut_comment(&self, peer: Peer, _payload: Bytes) -> Result<()> {
        trace!("[{}] 收到了对方的扩展协议 ut_comment", peer.name());
        Ok(())
    }
}