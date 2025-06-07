//! Peer Manager

use crate::core::command::CommandHandler;
use crate::core::context::Context;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::emitter::Emitter;
use crate::core::runtime::Runnable;
use crate::mapper::torrent::{TorrentMapper, TorrentStatus};
use crate::peer_manager::command::Command;
use crate::peer_manager::gasket::Gasket;
use crate::store::Store;
use crate::torrent::TorrentArc;
use crate::tracker;
use dashmap::{DashMap, DashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::channel;
use tokio::task::JoinHandle;
use tracing::{error, info, trace};

pub mod command;
mod error;
pub mod gasket;

pub struct GasketInfo {
    id: u64,
    peer_id: Arc<[u8; 20]>,
    join_handle: JoinHandle<()>,
}

#[derive(Clone)]
pub struct PeerManagerContext {
    pub context: Context,
    pub emitter: Emitter,
    pub gaskets: Arc<DashMap<u64, GasketInfo>>,
    pub gasket_id: Arc<AtomicU64>,
    pub store: Store,
    pub peer_conn_num: Arc<AtomicUsize>,
    peer_id_pool: Arc<DashSet<[u8; 20]>>,
}

impl PeerManagerContext {
    pub async fn get_peer_id(&self) -> [u8; 20] {
        for _ in 0..10 {
            let peer_id = tracker::gen_peer_id();
            if !self.peer_id_pool.contains(&peer_id) {
                self.peer_id_pool.insert(peer_id.clone());
                return peer_id;
            }
        }
        panic!("无法再获取新的 peer id");
    }

    pub async fn add_gasket(&self, gasket: GasketInfo) {
        self.gaskets.insert(gasket.id, gasket);
    }

    pub async fn remove_gasket(&self, gasket_id: u64) {
        if let Some((_key, gasket)) = self.gaskets.remove(&gasket_id) {
            self.peer_id_pool.remove(&*gasket.peer_id);
        }
    }
}

pub struct PeerManager {
    /// 全局上下文
    context: Context,

    /// peer 垫片，相同 torrent 的 peer由一个垫片管理
    gaskets: Arc<DashMap<u64, GasketInfo>>,

    /// 命令发射器
    emitter: Emitter,

    /// 垫片 id
    gasket_id: Arc<AtomicU64>,

    /// peer_id 生成池，一个垫片一个
    peer_id_pool: Arc<DashSet<[u8; 20]>>,

    /// 存储器，所有 peer 接收到 piece 后由 store 统一处理持久化事项
    store: Store,

    /// 当前 peer 链接数
    peer_conn_num: Arc<AtomicUsize>,
}

impl PeerManager {
    pub fn new(context: Context, emitter: Emitter) -> Self {
        let store = Store::new(context.get_config().clone(), emitter.clone());
        Self {
            context,
            gaskets: Arc::new(DashMap::new()),
            emitter,
            gasket_id: Arc::new(AtomicU64::new(0)),
            peer_id_pool: Arc::new(DashSet::new()),
            store,
            peer_conn_num: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn get_context(&self) -> PeerManagerContext {
        PeerManagerContext {
            context: self.context.clone(),
            emitter: self.emitter.clone(),
            gaskets: self.gaskets.clone(),
            gasket_id: self.gasket_id.clone(),
            store: self.store.clone(),
            peer_id_pool: self.peer_id_pool.clone(),
            peer_conn_num: self.peer_conn_num.clone(),
        }
    }

    pub async fn start_gasket(&self, torrent: TorrentArc) {
        let context = self.get_context();

        let mut emitter = Emitter::new();
        context.emitter.get(PEER_MANAGER).map(async |send| {
            emitter.register(PEER_MANAGER, send);
        });

        let gasket_id = context.gasket_id.fetch_add(1, Ordering::Relaxed);
        let peer_id = Arc::new(context.get_peer_id().await);
        let gasket = Gasket::new(
            gasket_id,
            torrent,
            context.clone(),
            peer_id.clone(),
            emitter,
            context.store.clone(),
        ).await;

        let join_handle = tokio::spawn(gasket.run());
        let gasket_info = GasketInfo {
            id: gasket_id,
            peer_id,
            join_handle,
        };

        context.add_gasket(gasket_info).await;
    }

    /// 从数据库中加载任务
    async fn load_task_from_db(&self) {
        let conn = self.context.get_conn().await.unwrap();
        let torrents = conn.list_torrent();
        for torrent in torrents {
            if torrent.status == Some(TorrentStatus::Download) && torrent.serail.is_some() {
                self.start_gasket(torrent.serail.unwrap()).await;
            }
        }
    }

    async fn shutdown(self) {
        self.emitter.remove(PEER_MANAGER);
        for mut item in self.gaskets.iter_mut() {
            let handle = &mut item.join_handle;
            handle.await.unwrap()
        }
        match self.store.flush_all() {
            Ok(_) => info!("关机前 flush 数据到存储设备成功"),
            Err(e) => error!("flush 数据到存储设备失败！！！\t{}", e)
        }
    }
}

impl Runnable for PeerManager {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.context.get_config().channel_buffer());
        self.emitter.register(PEER_MANAGER, send);
        self.load_task_from_db().await;

        info!("peer manager 已启动");
        loop {
            tokio::select! {
                _ = self.context.cancelled() => {
                    break;
                }
                recv = recv.recv() => {
                    trace!("peer manager 收到了消息: {:?}", recv);
                    if let Some(cmd) = recv {
                        let cmd: Command = cmd.instance();
                        if let Err(e) = cmd.handle(&mut self).await {
                            error!("处理指令出现错误\t{}", e);
                            break;
                        }
                    }
                }
            }
        }

        info!("等待 peer 退出");
        self.shutdown().await;
        info!("peer manager 已关闭");
    }
}
