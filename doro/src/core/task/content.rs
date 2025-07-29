use crate::base_peer::BasePeer;
use crate::base_peer::error::{ErrorType, PeerExitReason, deadly_error};
use crate::base_peer::rate_control::RateControl;
use crate::base_peer::rate_control::probe::Dashbord;
use crate::context::Context;
use crate::default_servant::DefaultServant;
use crate::dht;
use crate::dht::routing::NodeId;
use crate::mapper::torrent::{PieceStatus, TorrentEntity, TorrentMapper, TorrentStatus};
use crate::servant::ServantCallback;
use crate::task::content::coordinator::{Coordinator, PeerSwitch};
use crate::task::{Async, HostSource, ReceiveHost, Subscriber, Task};
use crate::task_manager::PeerId;
use crate::torrent::TorrentArc;
use crate::tracker::{AnnounceInfo, Tracker};
use anyhow::{Ok, Result, anyhow};
use async_trait::async_trait;
use bytes::BytesMut;
use dashmap::{DashMap, DashSet};
use doro_util::global::{GlobalId, Id};
use doro_util::option_ext::OptionExt;
use doro_util::sync::MutexExt;
use doro_util::{bytes_util, net};
use fnv::FnvHashSet;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::task::JoinSet;
use tracing::{Level, debug, error, info, level_enabled, trace};

mod coordinator;

/// 每间隔一分钟扫描一次 peers
const DHT_FIND_PEERS_INTERVAL: Duration = Duration::from_secs(60);

/// 等待的 peer 上限，如果超过这个数量，就不会进行 dht 扫描
const DHT_WAIT_PEER_LIMIT: usize = 20;

/// 期望每次 dht 扫描能找到 25 个 peer
const DHT_EXPECT_PEERS: usize = 25;

#[derive(Debug)]
pub struct PeerInfo {
    /// peer 的编号
    id: Id,

    /// 通信地址
    addr: SocketAddr,

    /// 速率仪表盘
    dashbord: Dashbord,

    /// 是否长期运行
    lt_running: bool,

    /// 是否正在等待有可用分片
    wait_piece: bool,

    /// 正在下载的分片
    download_piece: FnvHashSet<u32>,

    /// 对端拥有的分片
    bitfield: BytesMut,

    /// 错误的分块数量
    error_piece_cnt: u32,
}

impl PeerInfo {
    fn new(no: Id, addr: SocketAddr, dashbord: Dashbord) -> Self {
        Self {
            id: no,
            addr,
            dashbord,
            lt_running: true,
            download_piece: FnvHashSet::default(),
            bitfield: BytesMut::new(),
            wait_piece: false,
            error_piece_cnt: 0,
        }
    }

    pub fn is_lt(&self) -> bool {
        self.lt_running
    }
}

/// 下载种子内容的任务
#[allow(dead_code)]
pub struct DownloadContentInner {
    /// 任务 id
    id: Id,

    /// peer id
    peer_id: PeerId,

    /// 种子信息
    torrent: TorrentArc,

    /// 执行派遣
    dispatch: Dispatch,

    /// tracker
    tracker: Tracker<Dispatch>,

    /// 保存路径
    save_path: Arc<RwLock<PathBuf>>,

    /// 正在运行中的 peer
    peers: Arc<DashMap<Id, PeerInfo>>,

    /// 已下载的量
    download: Arc<AtomicU64>,

    /// 已上传的量
    uploaded: Arc<AtomicU64>,

    /// 这个是正儿八经下下来了的分块
    bytefield: Arc<Mutex<BytesMut>>,

    /// 正在下载中的分块
    underway_bytefield: Arc<DashMap<u32, PieceStatus>>,

    /// 存储的是不允许启动的地址，避免 tracker 扫描出相同的然后重复请求链接。
    /// 多次响应无法通过校验的分片的 peer，会从 peers 和 wait_queue 中移除，
    /// 但是不会从 unstart_host 中移除，以起到黑名单作用
    unstart_host: Arc<DashSet<SocketAddr>>,

    /// 可连接，但是没有任务可分配的 peer
    wait_queue: Arc<Mutex<VecDeque<PeerInfo>>>,

    /// peer 传输速率
    peer_transfer_speed: Arc<DashMap<Id, u64>>,

    /// 是否下载完成
    status: Arc<Mutex<TorrentStatus>>,

    /// peer 事件处理
    servant: Arc<DefaultServant>,

    /// 异步任务
    handles: Mutex<Option<JoinSet<()>>>,
}

/// 下载种子内容的任务的外部包装
#[derive(Clone)]
pub struct DownloadContent(Arc<DownloadContentInner>);

impl Deref for DownloadContent {
    type Target = Arc<DownloadContentInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DownloadContent {
    pub fn new(id: Id, peer_id: PeerId, torrent: TorrentArc) -> Self {
        let dispatch = Dispatch {
            inner: Arc::new(DispatchInner {}),
        };
        let download = Arc::new(AtomicU64::new(0));
        let uploaded = Arc::new(AtomicU64::new(0));
        let info = AnnounceInfo::new(
            download.clone(),
            uploaded.clone(),
            torrent.info.length,
            Context::global().get_config().tcp_server_addr().port(),
        );
        let tracker = Tracker::new(dispatch.clone(), peer_id.clone(), torrent.clone(), info);
        let dcsc = DownloadContentServantCallback {};
        Self(Arc::new(DownloadContentInner {
            id,
            peer_id,
            torrent,
            dispatch,
            tracker,
            save_path: Arc::new(Default::default()),
            peers: Arc::new(Default::default()),
            download,
            uploaded,
            bytefield: Arc::new(Mutex::new(Default::default())),
            underway_bytefield: Arc::new(Default::default()),
            unstart_host: Arc::new(Default::default()),
            wait_queue: Arc::new(Mutex::new(Default::default())),
            peer_transfer_speed: Arc::new(Default::default()),
            status: Arc::new(Mutex::new(TorrentStatus::Download)),
            servant: Arc::new(DefaultServant::new(dcsc)),
            handles: Mutex::new(None),
        }))
    }

    /// 初始化资源，从数据库中恢复进度
    #[rustfmt::skip]
    async fn init(&self) -> Result<()> {
        let conn = Context::global().get_conn().await?;
        let entity = conn.recover_from_db(&self.torrent.info_hash)?;

        *self.save_path.lock_pe() = entity.save_path.unwrap_or(
            Context::global()
                .get_config()
                .default_download_dir()
                .clone(),
        );

        let order = Ordering::Relaxed;
        self.download.store(entity.download.unwrap_or_default(), order);
        self.uploaded.store(entity.uploaded.unwrap_or_default(), order);
        entity.bytefield.map_ext(|b| *self.bytefield.lock_pe() = b);
        entity.status.map_ext(|s| *self.status.lock_pe() = s);
        entity.underway_bytefield.into_iter().for_each(|ub| {
            for (k, v) in ub {
                self.underway_bytefield.insert(k, v);
            }
        });

        Ok(())
    }

    // ===========================================================================
    // Content 相关
    // ===========================================================================

    /// 保存下载进度
    async fn save_progress(&self) {
        trace!("保存下载进度");
        let conn = Context::global().get_conn().await.unwrap();

        // 将 ub 转换为 Vec，将 Ing 改为 Pause
        let mut ub = Vec::with_capacity(self.underway_bytefield.len());
        for item in self.underway_bytefield.iter() {
            let key = *item.key();
            let mut value = item.value().clone();
            if let PieceStatus::Ing(v) = value {
                value = PieceStatus::Pause(v);
            }
            ub.push((key, value));
        }

        let entity = TorrentEntity {
            underway_bytefield: Some(ub),
            info_hash: Some(self.torrent.info_hash.to_vec()),
            bytefield: Some(self.bytefield.lock_pe().clone()),
            download: Some(self.download.load(Ordering::Relaxed)),
            uploaded: Some(self.uploaded.load(Ordering::Relaxed)),
            status: Some(*self.status.lock_pe()),
            ..Default::default()
        };
        conn.save_progress(entity).unwrap();
    }

    /// 启动异步任务
    async fn start_handle(&self) -> Result<()> {
        let mut handles = self.handles.lock_pe();
        if handles.is_none() {
            *handles = Some(JoinSet::new());
        }
        let handles = handles.as_mut().unwrap();
        handles.spawn(Box::pin(self.tracker.clone().run())); // 启动 tracker
        handles.spawn(Box::pin(self.clone().start_interval_find_peers_from_dht())); // 启动 dht 扫描
        handles.spawn(Box::pin(Coordinator::new(self.clone()).run())); // 启动下载协调器
        Ok(())
    }

    /// 定时从 dht 中查询 peer
    async fn start_interval_find_peers_from_dht(self) {
        let info_hash = NodeId::new(self.torrent.info_hash);

        loop {
            if *self.status.lock_pe() == TorrentStatus::Finished {
                break;
            }
            if self.wait_queue.lock_pe().len() < DHT_WAIT_PEER_LIMIT {
                #[rustfmt::skip]
                dht::find_peers(
                    info_hash.clone(),
                    self.dispatch.clone(),
                    self.id,
                    DHT_EXPECT_PEERS,
                ).await;
            }
            tokio::time::sleep(DHT_FIND_PEERS_INTERVAL).await;
        }
    }

    // ===========================================================================
    // peer 相关
    // ===========================================================================

    #[rustfmt::skip]
    pub async fn peer_exit(&self, peer_no: Id, reason: PeerExitReason) {
        debug!("peer_no [{}] 退出了，退出原因: {}", peer_no, reason);
        if Context::global().is_cancelled() {
            return;
        }

        if let Some((_, peer)) = self.peers.remove(&peer_no) {
            debug!("成功移除 peer_no [{}]", peer_no);
            match reason {
                PeerExitReason::DownloadFinished => {
                    self.unstart_host.remove(&peer.addr);
                }
                PeerExitReason::NotHasJob | PeerExitReason::PeriodicPeerReplace => {
                    self.wait_queue.lock_pe().push_back(peer);
                    self.try_notify_wait_piece().await;
                }
                PeerExitReason::Exception(e) if e.error_type() == ErrorType::DeadlyError => {
                    error!("peer_no [{}] 引发致命错误，加入黑名单，原因: {}", peer_no, e);
                }
                _ => {
                    self.unstart_host.remove(&peer.addr);
                    trace!("将这个地址从不可用 host 中移除，并从等待队列中唤醒一个");
                    // 有个副作用，原本应该先唤醒的 addr，如果遇到当前没
                    // 有可用的 peer 配额时，会被移动到最后。

                    // todo - 这里要优化，避免等待 peer 启动失败，长时间尝试启动
                    loop {
                        let peer = {
                            if let Some(peer) = self.wait_queue.lock_pe().pop_front() {
                                peer
                            } else {
                                info!("没有 peer 可以用了");
                                break;
                            }
                        };

                        debug!("尝试唤醒 [{}]", peer.addr);
                        if self.start_wait_peer(peer).await.is_ok() {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// 尝试唤醒一个等待 piece 的，下载速率相对来说还不错的 peer
    async fn try_notify_wait_piece(&self) -> Option<()> {
        let peer_no = *self.peers.iter_mut().find(|item| item.wait_piece)?.key();
        self.assign_peer_handle(peer_no).await;
        Some(())
    }

    /// 尝试分配任务
    pub async fn assign_peer_handle(&self, _peer_no: Id) {
        todo!()
        // if let Some(mut item) = self.peers.get_mut(&peer_no) {
        //     trace!(
        //         "唤醒了等待任务的 peer [{}]，ip addr: {}",
        //         item.no, item.addr
        //     );
        //     let tid = Peer::get_transfer_id(peer_no);
        //     let cmd = peer::command::TryRequestPiece.into();
        //     item.wait_piece = false;
        //     let _ = Emitter::global().send(&tid, cmd).await;
        // }
    }

    /// 申请下载分块
    pub async fn apply_download_piece(&self, peer_no: Id, piece_index: u32) -> Option<(u32, u32)> {
        trace!("peer {} 申请下载分块", peer_no);
        let mut res = None;

        let (index, offset) = bytes_util::bitmap_offset(piece_index as usize);
        let mut bytefield = self.bytefield.lock_pe();
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
            if let Some(mut item) = self.peers.get_mut(&peer_no) {
                item.download_piece.insert(*pi);
            }
        }

        res
    }

    // 归还分块下载
    // pub fn give_back_download_pieces(&self, peer_no: Id, give_back: &Vec<(&u32, &Piece)>) {
    //     let mut i = 0;
    //     for (piece_index, piece) in give_back {
    //         if !piece.is_finish() {
    //             self.underway_bytefield
    //                 .insert(**piece_index, PieceStatus::Pause(piece.block_offset()));
    //             i += 1;
    //         }
    //     }
    //     debug!("peer {} 归还下载分块\t归还的分块数量: {}", peer_no, i);
    // }

    /// 上报 bitfield
    pub fn reported_bitfield(&self, peer_no: Id, bitfield: BytesMut) {
        self.peers
            .get_mut(&peer_no)
            .map_ext(|mut item| item.bitfield = bitfield);
    }

    /// 有 peer 告诉我们没有分块可以下载了。在这里根据下载情况，决定是否让这个 peer 去抢占别人的任务
    pub async fn report_no_downloadable_piece(&self, peer_no: Id) {
        debug!("peer {} 没有可下载的分块了", peer_no);
        if self.check_finished() {
            self.save_progress().await;
            self.download_finished_after_handle().await;
        } else {
            if level_enabled!(Level::DEBUG) {
                let mut str = String::new();
                self.peers.iter().for_each(|item| {
                    let (bw, unit) = net::rate_formatting(item.dashbord.bw());
                    str.push_str(&format!(
                        "{}-{}: {:.2}{} {:?}\n",
                        item.id,
                        item.addr.port(),
                        bw,
                        unit,
                        item.download_piece.clone()
                    ));
                });
                debug!(
                    "peer_no: {} 没有任务了\t当前运行中的 peer: \n{}",
                    peer_no, str
                );
            }

            // 判断当前 peer no 的地位，如果是超快 peer，那么就从最慢
            // 的 peer 开始，寻找当前 peer 可以抢过来下载的 piece
            if self.try_free_slow_piece(peer_no).await != Some(true) {
                self.notify_peer_stop(peer_no, PeerExitReason::NotHasJob)
                    .await;
                info!("没有可下载的分块了，停掉peer: {}", peer_no);
            } else {
                self.peers
                    .get_mut(&peer_no)
                    .map_ext(|mut item| item.wait_piece = true);
            }
        }
    }

    /// 通知 peer 停止运行
    pub async fn notify_peer_stop(&self, _peer_no: Id, _reason: PeerExitReason) {
        todo!()
        // let tid = Peer::get_transfer_id(peer_no);
        // let cmd = peer::command::Exit { reason }.into();
        // let _ = Emitter::global().send(&tid, cmd).await; // 可能在通知退出前就已经退出了，所以忽略错误
    }

    /// todo - 下载完成后的后置处理
    async fn download_finished_after_handle(&self) {
        if *self.status.lock_pe() == TorrentStatus::Finished {
            info!("下载完成，通知 peer 退出");
            // 清空等待列表
            self.wait_queue.lock_pe().clear();

            // 通知各个 peer 退出
            let peers = self
                .peers
                .iter()
                .map(|item| *item.key())
                .collect::<Vec<_>>();
            for peer_no in peers.iter() {
                self.notify_peer_stop(*peer_no, PeerExitReason::DownloadFinished)
                    .await;
            }

            // 关闭资源下载任务
            if let Err(e) = self.shutdown().await {
                error!("关闭资源下载任务失败，原因: {}", e);
            }
        }
    }

    /// 检查是否下载完成
    fn check_finished(&self) -> bool {
        if *self.status.lock_pe() == TorrentStatus::Finished {
            return true;
        }

        let mut pn = self.torrent.piece_num() - 1; // 分片下标是从 0 开始的
        let mut last_v = !0u8 << (7 - (pn & 7));
        let bytefield = self.bytefield.lock_pe();
        for v in bytefield.iter().rev() {
            // 从后往前匹配
            if *v != last_v {
                return false;
            }
            pn -= 1;
            last_v = u8::MAX;
        }

        let info_hash = hex::encode(self.torrent.info_hash);
        info!("torrent [{}] 下载完成", info_hash);
        *self.status.lock_pe() = TorrentStatus::Finished;
        true
    }

    /// 释放下得慢点 piece
    async fn try_free_slow_piece(&self, peer_no: Id) -> Option<bool> {
        let current_peer = self.peers.get(&peer_no)?;
        let (peer_no, current_bw, bitfield) = (
            current_peer.id,
            current_peer.dashbord.bw(),
            current_peer.bitfield.clone(),
        );

        let peers = self.get_sorted_peers(peer_no);

        // 寻找速率比这个慢，同时正在进行要停掉 peer 可以下载的 piece。
        // 那么就告诉这个 peer，放弃下载这个 piece
        for (other_no, other_bw) in peers.iter() {
            if coordinator::faster(current_bw, *other_bw) {
                if let Some(_free_piece) = self.try_free_piece_for_peer(other_no, &bitfield) {
                    todo!();
                    // let tid = Peer::get_transfer_id(other_no);
                    // let cmd = peer::command::FreePiece {
                    //     peer_no,
                    //     pieces: free_piece,
                    // };
                    // let _ = Emitter::global().send(&tid, cmd.into()).await;
                    // return Some(true);
                }
            }
        }

        Some(false)
    }

    /// 获取按下载速度排序后的 peers
    fn get_sorted_peers(&self, exclude_no: Id) -> Vec<(Id, u64)> {
        let mut peers = self
            .peers
            .iter()
            .filter(|item| *item.key() != exclude_no)
            .map(|item| (item.id, item.dashbord.bw()))
            .collect::<Vec<_>>();
        peers.sort_unstable_by(|a, b| a.1.cmp(&b.1));
        peers
    }

    /// 尝试释放 piece
    fn try_free_piece_for_peer(&self, peer_no: &Id, bitfield: &BytesMut) -> Option<Vec<u32>> {
        let mut item = self.peers.get_mut(peer_no)?;
        let mut free_pieces = vec![];
        item.download_piece.retain(|&piece_index| {
            let (idx, ost) = bytes_util::bitmap_offset(piece_index as usize);
            let res = bitfield.get(idx).map(|val| val & ost != 0).unwrap_or(false);
            if res {
                free_pieces.push(piece_index);
            }
            !res
        });

        if !free_pieces.is_empty() {
            Some(free_pieces)
        } else {
            None
        }
    }

    // ===========================================================================
    // 相关上报
    // ===========================================================================

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

    /// 上报分片下载失败
    #[rustfmt::skip]
    pub async fn reported_piece_failed(&self, peer_no: Id, piece_idx: u32) {
        // 获取真实的分片大小
        let read_length = self.torrent.piece_length(piece_idx);

        // 分片下载失败，删除一个分片的大小
        self.download.fetch_sub(read_length as u64, Ordering::Relaxed);

        // 错误分片数加 1
        if let Some(mut peer) = self.peers.get_mut(&peer_no) {
            peer.error_piece_cnt += 1;
            if peer.error_piece_cnt >= Context::global().get_config().error_piece_limit() {
                self.notify_peer_stop(
                    peer_no,
                    deadly_error(anyhow!("错误分片数达到上限"))
                ).await
            }
        }
    }

    /// 上报分块下载完成
    pub async fn reported_piece_finished(&self, peer_no: Id, piece_index: u32) {
        let (index, offset) = bytes_util::bitmap_offset(piece_index as usize);
        self.bytefield
            .lock_pe()
            .get_mut(index)
            .map_ext(|byte| *byte |= offset);

        self.underway_bytefield.remove(&piece_index);
        self.peers.get_mut(&peer_no).map_ext(|mut item| {
            item.download_piece.remove(&piece_index);
        });

        self.check_finished();
        self.save_progress().await;

        self.download_finished_after_handle().await;
    }

    /// 上报读取到的数据大小
    pub fn reported_read_size(&self, peer_no: Id, read_size: u64) {
        *self.peer_transfer_speed.entry(peer_no).or_insert(0) += read_size
    }

    // ===========================================================================
    // 启动 peer
    // ===========================================================================

    /// 从等待队列中唤醒 peer
    async fn start_wait_peer(&self, pi: PeerInfo) -> Result<()> {
        // 超过配额，加入等待队列中
        let limit = Context::global().get_config().torrent_peer_conn_limit();
        if limit < self.peers.len() {
            self.wait_queue.lock_pe().push_back(pi);
            return Ok(());
        }

        let addr = pi.addr;
        let dashbord = pi.dashbord.clone();
        self.do_start_peer(addr, dashbord, true).await
    }

    /// 从临时队列中唤醒 peer
    #[allow(dead_code)]
    async fn start_temp_peer(&self, pi: PeerInfo) -> Result<()> {
        // 超过配额，加入等待队列中
        // gasket 通知关闭 peer 关闭，此过程是异步的，因此可能有 1 个 peer 的延迟
        let limit = Context::global().get_config().torrent_peer_conn_limit();
        if limit < self.peers.len() {
            self.wait_queue.lock_pe().push_back(pi);
            return Ok(());
        }

        let addr = pi.addr;
        let dashbord = pi.dashbord.clone();
        self.do_start_peer(addr, dashbord, false).await
    }

    /// 启动 peer
    async fn do_start_peer(&self, addr: SocketAddr, dashbord: Dashbord, lt: bool) -> Result<()> {
        let peer_no = GlobalId::next_id();
        let default_servant = self.servant.clone();
        let bp = BasePeer::new(peer_no, addr, default_servant, dashbord.clone());
        bp.async_run().await?;
        let mut peer = PeerInfo::new(peer_no, addr, dashbord);
        peer.lt_running = lt;
        self.peers.insert(peer_no, peer);
        Ok(())
    }
}

impl Task for DownloadContent {
    fn get_id(&self) -> Id {
        self.id
    }

    fn start(&self) -> Async<Result<()>> {
        let this = self.clone();
        Box::pin(async move {
            this.init().await?;
            this.start_handle().await
        })
    }

    /// todo - 暂停任务
    fn pause(&self) -> Async<Result<()>> {
        // let this = self.clone();
        Box::pin(async move { Ok(()) })
    }

    /// todo - 关闭任务
    fn shutdown(&self) -> Async<Result<()>> {
        // let this = self.clone();
        Box::pin(async move { Ok(()) })
    }

    /// todo - 订阅任务的状态变化
    fn subscribe_inside_info(
        &self,
        _subscriber: Box<dyn Subscriber + Send + 'static>,
    ) -> Async<()> {
        // let this = self.clone();
        Box::pin(async move {})
    }

    /// todo - 回调处理
    fn callback(&self) -> Async<()> {
        // let this = self.clone();
        Box::pin(async move {})
    }
}

#[derive(Default)]
struct DispatchInner {}

/// 执行派遣，派遣任务给 peer
#[derive(Clone, Default)]
struct Dispatch {
    inner: Arc<DispatchInner>,
}

impl Deref for Dispatch {
    type Target = Arc<DispatchInner>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[async_trait]
#[allow(unused_variables)]
impl ReceiveHost for Dispatch {
    /// 接收主机地址
    async fn receive_host(&self, host: SocketAddr, source: HostSource) {
        todo!()
    }

    /// 接收多个主机地址
    async fn receive_hosts(&self, hosts: Vec<SocketAddr>, source: HostSource) {
        todo!()
    }
}

struct DownloadContentServantCallback {}

#[async_trait]
impl ServantCallback for DownloadContentServantCallback {}

#[async_trait]
#[allow(unused_variables)]
impl PeerSwitch for DownloadContent {
    /// 升级为 lt peer
    fn upgrage_lt_peer(&self, id: Id) {
        todo!()
    }

    /// 替换 peer
    async fn replace_peer(&self, old_id: Id, new_id: Id) {
        todo!()
    }

    /// 通知 peer 停止
    async fn notify_peer_stop(&self, id: Id, reason: PeerExitReason) {
        todo!()
    }

    /// 开启一个新的临时 peer
    async fn start_temp_peer(&self) {
        todo!()
    }

    /// 找到最慢的 lt peer
    fn find_slowest_lt_peer(&self) -> Option<&PeerInfo> {
        todo!()
    }

    /// 找到最快的 temp peer
    fn find_fastest_temp_peer(&self) -> Option<&PeerInfo> {
        todo!()
    }

    /// 拿走累计的 peer 传输速度
    fn take_peer_transfer_speed(&self) -> u64 {
        todo!()
    }

    /// 获得下载文件大小
    fn file_length(&self) -> u64 {
        todo!()
    }

    /// 已下载的文件大小
    fn download_length(&self) -> u64 {
        todo!()
    }

    /// 是否达到 peer 限制
    fn is_peer_limit(&self) -> bool {
        todo!()
    }

    /// 是否完成下载
    fn is_finished(&self) -> bool {
        todo!()
    }
}
