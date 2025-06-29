pub mod command;
mod coordinator;

use crate::command::CommandHandler;
use crate::core::config::Config;
use crate::core::emitter::constant::GASKET_PREFIX;
use crate::core::emitter::Emitter;
use crate::db::ConnWrapper;
use crate::mapper::torrent::{TorrentEntity, TorrentMapper, TorrentStatus};
use crate::peer::rate_control::probe::Dashbord;
use crate::peer::rate_control::RateControl;
use crate::peer::{Peer, Piece};
use crate::peer_manager::gasket::command::{Command, SaveProgress, StartWaittingAddr};
use crate::peer_manager::gasket::coordinator::Coordinator;
use crate::peer_manager::PeerManagerContext;
use crate::runtime::Runnable;
use crate::store::Store;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};
use crate::{if_else, net, peer, util};
use bincode::{Decode, Encode};
use bytes::BytesMut;
use dashmap::{DashMap, DashSet};
use fnv::FnvHashSet;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::channel;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::{debug, error, info, level_enabled, trace, Level};
use crate::dht::DHT;

#[derive(PartialEq, Debug)]
pub enum ExitReason {
    /// 正常退出
    Normal,

    /// 没有任务的退出
    NotHasJob,

    /// 周期性的临时 peer 替换
    PeriodicPeerReplace,

    /// 异常退出
    Exception,
}

#[derive(Eq, PartialEq, Decode, Encode, Clone, Debug)]
pub enum PieceStatus {
    /// 进行中
    Ing(u32),

    /// 暂停，未开始也用这个标记
    Pause(u32),
}

#[derive(Debug)]
pub struct PeerInfo {
    /// peer 的编号
    no: u64,

    /// 通信地址
    addr: Arc<SocketAddr>,

    /// 速率仪表盘
    dashbord: Dashbord,

    /// 是否长期运行
    lt_running: bool,

    /// 是否正在等待有可用分片
    wait_piece: bool,

    /// 正在下载的分片
    download_piece: FnvHashSet<u32>,

    /// 对端拥有的分片
    bitfield: Arc<Mutex<BytesMut>>,

    /// 异步任务句柄
    join_handle: Option<JoinHandle<()>>,
}

impl PeerInfo {
    fn new(
        no: u64,
        addr: Arc<SocketAddr>,
        dashbord: Dashbord,
        join_handle: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            no,
            addr,
            dashbord,
            lt_running: true,
            join_handle,
            download_piece: FnvHashSet::default(),
            bitfield: Arc::new(Mutex::new(BytesMut::new())),
            wait_piece: false,
        }
    }

    pub fn is_lt(&self) -> bool {
        self.lt_running
    }
}

#[derive(Clone)]
pub struct GasketContext {
    /// gasket 的 id
    gtid: String,

    /// peer manager context
    pm_ctx: PeerManagerContext,

    /// peer id
    peer_id: Arc<[u8; 20]>,

    /// 种子信息
    torrent: TorrentArc,

    /// 保存路径
    save_path: Arc<PathBuf>,

    /// peer_id 自增计数
    peer_no_count: Arc<AtomicU64>,

    /// 正在运行中的 peer
    peers: Arc<DashMap<u64, PeerInfo>>,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 存储的是可链接地址，避免 tracker 扫描出相同的然后重复请求链接
    unstart_host: Arc<DashSet<Arc<SocketAddr>>>,

    /// 可连接，但是没有任务可分配的 peer
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// 命令发射器
    emitter: Emitter,

    /// peer 传输速率
    peer_transfer_speed: Arc<DashMap<u64, u64>>,

    /// 是否下载完成
    status: Arc<Mutex<TorrentStatus>>,
}

impl GasketContext {
    fn new(
        gtid: String,
        pm_ctx: PeerManagerContext,
        peer_id: Arc<[u8; 20]>,
        torrent: TorrentArc,
        entity: TorrentEntity,
        emitter: Emitter,
    ) -> Self {
        let ub: DashMap<u32, PieceStatus> =
            entity.underway_bytefield.unwrap().into_iter().collect();
        let status = entity.status.unwrap();

        Self {
            gtid,
            pm_ctx,
            peer_id,
            torrent,
            peer_no_count: Arc::new(AtomicU64::new(0)),
            peers: Arc::new(DashMap::new()),
            save_path: Arc::new(entity.save_path.unwrap()),
            download: Arc::new(AtomicU64::new(entity.download.unwrap())),
            uploaded: Arc::new(AtomicU64::new(entity.uploaded.unwrap())),
            bytefield: Arc::new(Mutex::new(entity.bytefield.unwrap())),
            underway_bytefield: Arc::new(ub),
            unstart_host: Arc::new(DashSet::new()),
            wait_queue: Arc::new(Mutex::new(VecDeque::new())),
            emitter,
            peer_transfer_speed: Arc::new(DashMap::new()),
            status: Arc::new(Mutex::from(status)),
        }
    }

    pub fn cancel_token(&self) -> WaitForCancellationFuture {
        self.pm_ctx.context.cancelled()
    }

    pub fn peer_id(&self) -> &[u8; 20] {
        self.peer_id.as_ref()
    }

    pub fn torrent(&self) -> TorrentArc {
        self.torrent.clone()
    }

    pub fn config(&self) -> Config {
        self.pm_ctx.context.get_config().clone()
    }

    pub fn save_path(&self) -> &PathBuf {
        &self.save_path
    }

    pub fn bytefield(&self) -> &Arc<Mutex<BytesMut>> {
        &self.bytefield
    }

    pub async fn peer_exit(&mut self, peer_no: u64, reason: ExitReason) {
        debug!("peer_no [{}] 退出了，退出原因: {:?}", peer_no, reason);
        if self.pm_ctx.context.is_cancelled() {
            return;
        }

        if let Some((_, mut peer)) = self.peers.remove(&peer_no) {
            debug!("成功移除 peer_no [{}]", peer_no);
            if reason == ExitReason::NotHasJob || reason == ExitReason::PeriodicPeerReplace {
                peer.join_handle = None;
                self.wait_queue.lock().await.push_back(peer);
                self.try_notify_wait_piece().await;
            } else {
                self.unstart_host.remove(&peer.addr);
                trace!("将这个地址从不可用host中移除了");

                // 从等待队列中唤醒一个
                if let Err(e) = self
                    .emitter
                    .send(&self.gtid, StartWaittingAddr.into())
                    .await
                {
                    error!("唤醒等待队列中的失败！{}", e);
                }
            }
        }

        trace!("发送了唤醒消息");
    }

    /// 尝试唤醒一个等待 piece 的，下载速率相对来说还不错的 peer
    async fn try_notify_wait_piece(&self) -> Option<()> {
        let peer_no = *self.peers.iter_mut().find(|item| item.wait_piece)?.key();
        self.assign_peer_handle(peer_no).await;
        Some(())
    }

    /// 尝试分配任务
    pub async fn assign_peer_handle(&self, peer_no: u64) {
        if let Some(mut item) = self.peers.get_mut(&peer_no) {
            trace!("唤醒了等待任务的 peer [{}]，ip addr: {}", item.no, item.addr);
            let tid = Peer::get_transfer_id(peer_no);
            let cmd = peer::command::TryRequestPiece.into();
            item.wait_piece = false;
            let _ = self.emitter.send(&tid, cmd).await;
        }
    }

    /// 上报 bitfield
    pub fn reported_bitfield(&self, peer_no: u64, bitfield: Arc<Mutex<BytesMut>>) {
        self.peers
            .get_mut(&peer_no)
            .map(|mut item| item.bitfield = bitfield);
    }

    /// 申请下载分块
    pub async fn apply_download_piece(&self, peer_no: u64, piece_index: u32) -> Option<(u32, u32)> {
        trace!("peer {} 申请下载分块", peer_no);
        let mut res = None;

        let (index, offset) = util::bytes::bitmap_offset(piece_index as usize);
        let mut bytefield = self.bytefield.lock().await;
        if *bytefield.get_mut(index)? & offset == 0 {
            self.underway_bytefield
                .entry(piece_index)
                .and_modify(|value| {
                    if let PieceStatus::Pause(block_offset) = value {
                        res = Some((piece_index, *block_offset));
                        *value = PieceStatus::Ing(*block_offset)
                    }
                })
                .or_insert_with(|| {
                    res = Some((piece_index, 0));
                    PieceStatus::Ing(0)
                });
        }

        if let Some((pi, _)) = res.as_ref() {
            self.peers.get_mut(&peer_no).map(|mut item| {
                item.download_piece.insert(*pi);
            });
        }

        res
    }

    /// 归还分块下载
    pub fn give_back_download_pieces(&self, peer_no: u64, give_back: Vec<(u32, Piece)>) {
        debug!("peer {} 归还下载分块", peer_no);
        for (piece_index, piece) in give_back {
            if !piece.is_finish() {
                self.underway_bytefield
                    .insert(piece_index, PieceStatus::Pause(piece.block_offset()));
            }
        }
    }

    /// 有 peer 告诉我们没有分块可以下载了。在这里根据下载情况，决定是否让这个 peer 去抢占别人的任务
    pub async fn report_no_downloadable_piece(&self, peer_no: u64) {
        trace!("peer {} 没有可下载的分块了", peer_no);
        if self.check_finished().await {
            self.store_progress().await;
            self.download_finished_after_handle().await;
        } else {
            if level_enabled!(Level::DEBUG) {
                let mut str = String::new();
                self.peers.iter().for_each(|item| {
                    let (bw, unit) = net::rate_formatting(item.dashbord.bw());
                    str.push_str(&format!("{}-{}: {}{} {:?}\n",
                                      item.no, item.addr.port(), bw, unit, 
                                      item.download_piece.clone())
                    );
                });
                debug!("peer_no: {} 没有任务了\t当前运行中的 peer: \n{}", peer_no, str);    
            }

            // 判断当前 peer no 的地位，如果是超快 peer，那么就从最慢
            // 的 peer 开始，寻找当前 peer 可以抢过来下载的 piece
            if self.try_free_slow_piece(peer_no).await != Some(true) {
                self.notify_peer_stop(peer_no, ExitReason::NotHasJob).await;
                info!("没有可下载的分块了，停掉peer: {}", peer_no);
            } else {
                self.peers.get_mut(&peer_no).map(|mut item| item.wait_piece = true);
            }
        }
    }

    /// 释放下得慢点 piece
    async fn try_free_slow_piece(&self, peer_no: u64) -> Option<bool> {
        let mut flag = false;
        let peer = self
            .peers
            .get(&peer_no)
            .map(|item| (item.no, item.dashbord.bw(), item.bitfield.clone()))?;

        let mut peers = self
            .peers
            .iter()
            .filter(|item| *item.key() != peer.0)
            .map(|item| (item.no, item.dashbord.bw()))
            .collect::<Vec<_>>();
        peers.sort_unstable_by(|a, b| a.1.cmp(&b.1));

        // 寻找速率比这个慢，同时正在进行要停掉 peer 可以下载的 piece。
        // 那么就告诉这个 peer，放弃下载这个 piece
        for p in peers.iter() {
            if coordinator::faster(peer.1, p.1) {
                let item = self.peers.get_mut(&p.0);
                let mut item = if_else!(item.is_none(), continue, item.unwrap());

                let peer_bitmap = peer.2.lock().await;

                let mut free_piece = vec![];
                item.download_piece.retain(|piece_index| {
                    let (idx, ost) = util::bytes::bitmap_offset(*piece_index as usize);
                    let res = peer_bitmap.get(idx).map(|val| val & ost != 0).unwrap_or(false);
                    if res { free_piece.push(*piece_index); }
                    !res
                });

                if !free_piece.is_empty() {
                    let tid = Peer::get_transfer_id(p.0);
                    let cmd = peer::command::FreePiece {
                        peer_no: peer.0,
                        pieces: free_piece,
                    }.into();
                    let _ = self.emitter.send(&tid, cmd).await;
                    flag = true;
                    break;
                }
            }
        }

        Some(flag)
    }

    /// 通知 peer 停止运行
    pub async fn notify_peer_stop(&self, peer_no: u64, reason: ExitReason) {
        let tid = Peer::get_transfer_id(peer_no);
        let cmd = peer::command::Exit { reason }.into();
        let _ = self.emitter.send(&tid, cmd).await; // 可能在通知退出前就已经退出了，所以忽略错误
    }

    /// 升级为 lt peer
    pub fn upgrage_lt_peer(&self, tmp_peer: u64) -> Option<()> {
        self.peers.get_mut(&tmp_peer)?.lt_running = true;
        Some(())
    }

    /// 替换 peer，用一个临时 peer 替换一个 lt peer，并把这个临时 peer 升级为 lt peer
    pub async fn replace_peer(&self, tmp_peer: u64, lt_peer: u64) -> Option<()> {
        // 将 tmp_peer 升级为 lt_peer
        self.upgrage_lt_peer(tmp_peer)?;

        // 通知 lt_peer 停止运行
        self.notify_peer_stop(lt_peer, ExitReason::PeriodicPeerReplace)
            .await;

        Some(())
    }

    /// 开启一个临时 peer
    pub async fn start_temp_peer(&self) {
        // gasket 通知关闭 peer 关闭，此过程是异步的，因此可能有 1 个 peer 的延迟
        if self.config().torrent_peer_conn_limit() < self.peers.len() {
            info!(
                "无法启动临时 peer，因为当前 peer 数量已达上限，当前 peers 数量: {}",
                self.peers.len()
            );
            return;
        }

        // 统一由 gasket 启动 peer，避免数量更新不一致导致多启动 peer
        if let Some(peer_info) = self.wait_queue.lock().await.pop_front() {
            info!("启动一个新的 temp peer, addr: {}", peer_info.addr);
            let cmd = command::StartTempPeer { peer_info }.into();
            self.emitter.send(&self.gtid, cmd).await.unwrap();
        }
    }

    /// 上报下载量信息
    pub fn reported_download(&self, piece_index: u32, block_offset: u32, block_size: u64) {
        self.download.fetch_add(block_size, Ordering::Relaxed);
        if let Some(mut item) = self.underway_bytefield.get_mut(&piece_index) {
            if let PieceStatus::Ing(value) = item.value_mut() {
                *value = block_offset;
            }
        }
    }

    /// 上报上传信息
    pub fn reported_uploaded(&self, block_size: u64) {
        self.uploaded.fetch_add(block_size, Ordering::Relaxed);
    }

    /// 上报分块下载完成
    pub async fn reported_piece_finished(&self, peer_no: u64, piece_index: u32) {
        let (index, offset) = util::bytes::bitmap_offset(piece_index as usize);
        self.bytefield
            .lock()
            .await
            .get_mut(index)
            .map(|byte| *byte |= offset);

        self.underway_bytefield.remove(&piece_index);
        self.peers.get_mut(&peer_no).map(|mut item| {
            item.download_piece.remove(&piece_index);
        });

        self.check_finished().await;
        self.store_progress().await;

        self.download_finished_after_handle().await;
    }

    ///　保存进度
    async fn store_progress(&self) {
        let status = self.status.lock().await.clone();
        // 存数据库，通过通道信号传递给 gasket 执行存储动作，避免阻塞 peer 线程
        let cmd = SaveProgress {
            status: Some(status),
        }
        .into();
        self.emitter.send(&self.gtid, cmd).await.unwrap();
    }

    /// 下载完成后的后置处理
    async fn download_finished_after_handle(&self) {
        // 通知各个 peer 退出
        if *self.status.lock().await == TorrentStatus::Finished {
            info!("下载完成，通知 peer 退出");
            let peers = self
                .peers
                .iter()
                .map(|item| *item.key())
                .collect::<Vec<_>>();
            for peer_no in peers.iter() {
                self.notify_peer_stop(*peer_no, ExitReason::Normal).await;
            }
        }
    }

    /// 检查是否下载完成
    async fn check_finished(&self) -> bool {
        if *self.status.lock().await == TorrentStatus::Finished {
            return true;
        }

        let mut pn = self.torrent.piece_num() - 1; // 分片下标是从 0 开始的
        let mut last_v = !0u8 << (7 - (pn & 7));
        let bytefield = self.bytefield.lock().await;
        for v in bytefield.iter().rev() { // 从后往前匹配
            if *v != last_v {
                return false;
            }
            pn -= 1;
            last_v = u8::MAX;
        }

        let info_hash = hex::encode(self.torrent.info_hash);
        info!("torrent [{}] 下载完成", info_hash);
        *self.status.lock().await = TorrentStatus::Finished;
        true
    }

    /// 上报读取到的数据大小
    pub fn reported_read_size(&self, peer_no: u64, read_size: u64) {
        *self.peer_transfer_speed.entry(peer_no).or_insert(0) += read_size
    }
}

#[inline]
pub fn get_transfer_id(id: u64) -> String {
    format!("{}{}", GASKET_PREFIX, id)
}

/// 同一个任务的peer交给一个垫片来管理，垫片对peer进行分块下载任务的分配
pub struct Gasket {
    /// gasket 的 id
    id: u64,

    /// gasket context
    ctx: GasketContext,

    /// 存储处理
    store: Store,
}

impl Gasket {
    pub async fn new(
        id: u64,
        torrent: TorrentArc,
        pm_ctx: PeerManagerContext,
        peer_id: Arc<[u8; 20]>,
        emitter: Emitter,
        store: Store,
    ) -> Self {
        let conn = pm_ctx.context.get_conn().await.unwrap();
        let entity = conn.recover_from_db(&torrent.info_hash);
        let ctx = GasketContext::new(
            get_transfer_id(id),
            pm_ctx,
            peer_id,
            torrent,
            entity,
            emitter,
        );

        Self { id, ctx, store }
    }

    fn get_config(&self) -> Config {
        self.ctx.config()
    }

    async fn get_conn(&self) -> ConnWrapper {
        self.ctx.pm_ctx.context.get_conn().await.unwrap()
    }
    
    async fn start_peer(&self, addr: Arc<SocketAddr>) {
        debug!("启动peer: {}", addr);
        if self.ctx.unstart_host.contains(&addr) || self.ctx.check_finished().await {
            return;
        }

        let limit = self.get_config().torrent_lt_peer_conn_limit();
        let unstart_host = &self.ctx.unstart_host;
        unstart_host.insert(addr.clone());

        // 超过配额，加入等待队列中
        let dashbord = Dashbord::new();
        if limit <= self.ctx.peers.len() {
            debug!("peer 数量已超过额定限制");
            let pi = PeerInfo::new(0, addr, dashbord, None);
            self.ctx.wait_queue.lock().await.push_back(pi);
            return;
        }

        self.do_start_peer(addr, dashbord, true);
    }
    
    async fn start_wait_peer(&self, pi: PeerInfo) {
        // 超过配额，加入等待队列中
        let limit = self.get_config().torrent_peer_conn_limit();
        if limit < self.ctx.peers.len() {
            self.ctx.wait_queue.lock().await.push_back(pi);
            return;
        }

        let addr = pi.addr.clone();
        let dashbord = pi.dashbord.clone();
        self.do_start_peer(addr, dashbord, true);
    }

    async fn start_temp_peer(&self, pi: PeerInfo) {
        // 超过配额，加入等待队列中
        // gasket 通知关闭 peer 关闭，此过程是异步的，因此可能有 1 个 peer 的延迟
        let limit = self.get_config().torrent_peer_conn_limit();
        if limit < self.ctx.peers.len() {
            self.ctx.wait_queue.lock().await.push_back(pi);
            return;
        }

        let addr = pi.addr.clone();
        let dashbord = pi.dashbord.clone();
        self.do_start_peer(addr, dashbord, false);
    }

    fn do_start_peer(&self, addr: Arc<SocketAddr>, dashbord: Dashbord, lt: bool) {
        let peer_no = self.ctx.peer_no_count.fetch_add(1, Ordering::Acquire);
        let peer = Peer::new(
            peer_no,
            addr.clone(),
            self.ctx.clone(),
            self.ctx.emitter.clone(),
            self.store.clone(),
            dashbord.clone(),
        );

        let join_handle = tokio::spawn(peer.run());
        let mut peer = PeerInfo::new(peer_no, addr, dashbord, Some(join_handle));
        peer.lt_running = lt;
        self.ctx.peers.insert(peer_no, peer);
    }

    async fn shutdown(self) {
        // 先把任务句柄读取出来，避免在循环中等待任务结束，以免和 peer_exit 中的 remove peer 操作形成死锁
        debug!("等待 peers 关闭");
        let handles = self
            .ctx
            .peers
            .iter_mut()
            .map(|mut item| item.join_handle.take())
            .collect::<Vec<Option<JoinHandle<()>>>>();
        for handle in handles {
            if let Some(handle) = handle {
                handle.await.unwrap();
            }
        }
        self.save_progress(None).await;
    }

    async fn save_progress(&self, status: Option<TorrentStatus>) {
        trace!("保存下载进度");
        let conn = self.get_conn().await;

        // 将 ub 转换为 Vec，将 Ing 改为 Pause
        let mut ub = Vec::with_capacity(self.ctx.underway_bytefield.len());
        for item in self.ctx.underway_bytefield.iter() {
            let key = item.key().clone();
            let mut value = item.value().clone();
            if let PieceStatus::Ing(v) = value {
                value = PieceStatus::Pause(v);
            }
            ub.push((key, value));
        }

        let entity = TorrentEntity {
            underway_bytefield: Some(ub),
            info_hash: Some(self.ctx.torrent.info_hash.to_vec()),
            bytefield: Some(self.ctx.bytefield.lock().await.clone()),
            download: Some(self.ctx.download.load(Ordering::Relaxed)),
            uploaded: Some(self.ctx.uploaded.load(Ordering::Relaxed)),
            status,
            ..Default::default()
        };
        conn.save_progress(entity);
    }

    fn start_tracker(&self, cancel_token: CancellationToken) -> JoinHandle<()> {
        let transfer_id = get_transfer_id(self.id);
        trace!("启动 {} 的 tracker", transfer_id);
        let info = AnnounceInfo::new(
            self.ctx.download.clone(),
            self.ctx.uploaded.clone(),
            self.ctx.torrent.info.length,
            self.get_config().tcp_server_addr().port(),
        );
        let tracker = Tracker::new(
            self.ctx.torrent.clone(),
            self.ctx.peer_id.clone(),
            info,
            self.ctx.emitter.clone(),
            self.ctx.pm_ctx.clone(),
            transfer_id,
            cancel_token
        );
        tokio::spawn(tracker.run())
    }
    
    fn start_dht(&self, cancel_token: CancellationToken) -> JoinHandle<()> {
        let transfer_id = get_transfer_id(self.id);
        trace!("启动 {} 的 dht", transfer_id);
        let dht = DHT::new(
            self.id,
            Arc::new(*NODE_ID),
            Arc::new(self.ctx.torrent.info_hash),
            self.ctx.emitter.clone(),
            self.ctx.pm_ctx.context.clone(),
            cancel_token,
            self.ctx.pm_ctx.udp_server.clone()
        );
        tokio::spawn(dht.run())
    }
    
    fn start_coordinator(&self, cancel_token: CancellationToken) -> JoinHandle<()> {
        let coor = Coordinator::new(self.ctx.clone(), cancel_token);
        tokio::spawn(coor.run())
    }

    #[inline]
    async fn pop_wait_peer(&self) -> Option<PeerInfo> {
        self.ctx.wait_queue.lock().await.pop_front()
    }
}

/// 临时 node id todo - 临时的，记得改
static NODE_ID: &[u8; 20] = b"adkoqwei123jk3341ks0";

impl Runnable for Gasket {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.get_config().channel_buffer());
        let transfer_id = get_transfer_id(self.id);
        self.ctx.emitter.register(transfer_id.clone(), send.clone());

        trace!("启动 gasket");
        let cancel_token = CancellationToken::new();
        let future_handle = vec![
            self.start_tracker(cancel_token.clone()),
            self.start_dht(cancel_token.clone()),
            self.start_coordinator(cancel_token.clone())
        ];

        loop {
            tokio::select! {
                _ = self.ctx.pm_ctx.context.cancelled() => {
                    info!("gasket 退出");
                    break;
                }
                cmd = recv.recv() => {
                    if let Some(cmd) = cmd {
                        let cmd: Command = cmd.instance();
                        if let Err(e) = cmd.handle(&mut self).await {
                            error!("处理指令出现错误\t{}", e);
                            break;
                        }
                    }
                }
            }
        }
        
        // 向 handle 发送退出命令
        cancel_token.cancel();
        for handle in future_handle {
            handle.await.unwrap();
        }
        self.shutdown().await;
        debug!("gasket {} 已退出！", transfer_id);
    }
}
