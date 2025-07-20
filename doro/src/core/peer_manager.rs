//! Peer Manager

use crate::core::command::CommandHandler;
use crate::core::context::Context;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::PEER_MANAGER;
use crate::core::runtime::Runnable;
use crate::emitter::transfer::TransferPtr;
use crate::mapper::torrent::{TorrentMapper, TorrentStatus};
use crate::peer_manager::command::Command;
use crate::peer_manager::gasket::Gasket;
use crate::rss::RSS;
use crate::runtime::{CommandHandleResult, ExitReason, RunContext};
use crate::store::Store;
use crate::torrent::TorrentArc;
use anyhow::Result;
use dashmap::{DashMap, DashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::{error, info};

pub mod command;
pub mod gasket;

pub struct GasketInfo {
    id: u64,
    peer_id: Arc<[u8; 20]>,
    join_handle: JoinHandle<()>,
}

#[derive(Clone)]
pub struct PeerManagerContext {
    /// 全局上下文
    pub(crate) context: Context,

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
    // 当前 peer 链接数
    // peer_conn_num: Arc<AtomicUsize>,
}

impl PeerManagerContext {
    pub async fn get_peer_id(&self) -> [u8; 20] {
        for _ in 0..10 {
            let peer_id = doro_util::rand::gen_peer_id();
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
    /// 上下文
    pmc: PeerManagerContext,

    /// 异步任务
    future_token: (CancellationToken, Vec<JoinHandle<()>>),
}

impl PeerManager {
    pub fn new(context: Context, emitter: Emitter) -> Self {
        let store = Store::new(context.get_config().clone(), emitter.clone());
        let pmc = PeerManagerContext {
            context,
            emitter,
            gaskets: Arc::new(DashMap::new()),
            gasket_id: Arc::new(AtomicU64::new(0)),
            store,
            peer_id_pool: Arc::new(DashSet::new()),
            // peer_conn_num: Arc::new(AtomicUsize::new(0)),
        };

        Self {
            pmc,
            future_token: (CancellationToken::new(), vec![]),
        }
    }

    pub async fn start_gasket(&self, torrent: TorrentArc) {
        let context = self.pmc.clone();

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
        )
        .await;

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
        let conn = self.pmc.context.get_conn().await.unwrap();
        let torrents = conn.list_torrent().unwrap();
        for torrent in torrents {
            if torrent.status == Some(TorrentStatus::Download) && torrent.serail.is_some() {
                self.start_gasket(torrent.serail.unwrap()).await;
            }
        }
    }

    /// 启动 rss。因为启动 rss 之后会扫描订阅源，并将订阅源中的种子加入下载队列，
    /// 所以必须在启动 rss 之前启动 peer manager
    fn start_rss(&mut self) {
        let rss = RSS::new(
            self.pmc.context.clone(),
            self.pmc.emitter.clone(),
            self.future_token.0.clone(),
        );
        let rss_handle = tokio::spawn(rss.run());
        self.future_token.1.push(rss_handle);
    }
}

impl Runnable for PeerManager {
    fn emitter(&self) -> &Emitter {
        &self.pmc.emitter
    }

    fn get_transfer_id<T: ToString>(_suffix: T) -> String {
        PEER_MANAGER.to_string()
    }

    async fn run_before_handle(&mut self, _rc: RunContext) -> Result<()> {
        self.load_task_from_db().await;
        self.start_rss();
        Ok(())
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.pmc.context.cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }

    async fn shutdown(&mut self, _reason: ExitReason) {
        self.future_token.0.cancel();
        for handle in self.future_token.1.iter_mut() {
            handle.await.unwrap();
        }

        for mut item in self.pmc.gaskets.iter_mut() {
            let handle = &mut item.join_handle;
            handle.await.unwrap()
        }
        match self.pmc.store.flush_all() {
            Ok(_) => info!("关机前 flush 数据到存储设备成功"),
            Err(e) => error!("flush 数据到存储设备失败！！！\t{}", e),
        }
    }
}
