use std::mem;
use std::sync::{Arc, Mutex, OnceLock};
use dashmap::{DashMap, DashSet};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use doro_util::global::{GlobalId, Id};
use doro_util::sync::MutexExt;
use crate::context::Context;
use crate::mapper::torrent::{TorrentMapper, TorrentStatus};
use crate::task_handler::gasket::Gasket;
use crate::rss::RSS;
use crate::runtime::Runnable;
use crate::store::Store;
use crate::torrent::TorrentArc;

pub mod gasket;

static TASK_HANDLER: OnceLock<TaskHandler> = OnceLock::new();

#[derive(Clone)]
pub struct PeerId {
    peer_id: Arc<[u8; 20]>,
    peer_id_pool: Arc<DashSet<[u8; 20]>>,
}

impl PeerId {
    pub fn new(peer_id: [u8; 20], peer_id_pool: Arc<DashSet<[u8; 20]>>) -> Self {
        Self {
            peer_id: Arc::new(peer_id),
            peer_id_pool,
        }
    }
    
    pub fn from_peer_id(peer_id: [u8; 20]) -> Self {
        let peer_id_pool = Arc::new(DashSet::new());
        peer_id_pool.insert(peer_id.clone());
        PeerId {
            peer_id: Arc::new(peer_id),
            peer_id_pool,
        }
    }
    
    pub fn value(&self) -> &[u8; 20] {
        &*self.peer_id
    }
}

impl Drop for PeerId {
    fn drop(&mut self) {
        self.peer_id_pool.remove(&*self.peer_id);
    }
}

pub struct GasketInfo {
    id: Id,
    join_handle: JoinHandle<()>,
}

pub struct TaskHandler {
    /// peer 垫片，相同 torrent 的 peer由一个垫片管理
    gaskets: Arc<DashMap<Id, GasketInfo>>,

    /// peer_id 生成池，一个垫片一个
    peer_id_pool: Arc<DashSet<[u8; 20]>>,

    /// 存储器，所有 peer 接收到 piece 后由 store 统一处理持久化事项
    store: Store,

    /// 异步任务
    future_token: Arc<Mutex<(CancellationToken, Vec<JoinHandle<()>>)>>,
}

impl TaskHandler {
    pub async fn init() {
        let task_handler = TASK_HANDLER.get_or_init(|| {
            TaskHandler {
                gaskets: Arc::new(DashMap::new()),
                peer_id_pool: Arc::new(DashSet::new()),
                store: Store::new(),
                future_token: Arc::new(Mutex::new((CancellationToken::new(), Vec::new()))),
            }
        });
        task_handler.init_handle().await;
    }

    async fn init_handle(&self) {
        self.load_task_from_db().await;
        self.start_rss();
    }

    /// 从数据库中加载任务
    async fn load_task_from_db(&self) {
        let conn = Context::global().get_conn().await.unwrap();
        let torrents = conn.list_torrent().unwrap();
        for torrent in torrents {
            if torrent.status == Some(TorrentStatus::Download) && torrent.serail.is_some() {
                self.handle_task(torrent.serail.unwrap()).await;
            }
        }
    }

    /// 启动 rss。因为启动 rss 之后会扫描订阅源，并将订阅源中的种子加入下载队列，
    /// 所以必须在启动 rss 之前启动 peer manager
    fn start_rss(&self) {
        let rss = RSS::new(
            self.future_token.lock_pe().0.clone(),
        );
        let rss_handle = tokio::spawn(rss.run());
        self.future_token.lock_pe().1.push(rss_handle);
    }

    fn get_peer_id(&self) -> PeerId {
        for _ in 0..10 {
            let peer_id = doro_util::rand::gen_peer_id();
            if self.peer_id_pool.insert(peer_id) {
                return PeerId::new(peer_id, self.peer_id_pool.clone());
            }
        }
        panic!("无法再获取新的 peer id");
    }

    fn add_gasket(&self, gasket: GasketInfo) {
        self.gaskets.insert(gasket.id, gasket);
    }

    fn remove_gasket(&self, gasket_id: Id) {
        self.gaskets.remove(&gasket_id);
    }
}

/// 对外常用接口
impl TaskHandler {
    /// 获取全局实例
    pub fn global() -> &'static Self {
        TASK_HANDLER.get().unwrap()
    }

    /// 处理任务
    pub async fn handle_task(&self, torrent: TorrentArc) {
        let id = GlobalId::global().next_id();
        let peer_id = self.get_peer_id();

        #[rustfmt::skip]
        let gasket = Gasket::new(
            id, torrent,
            peer_id,
            self.store.clone(),
        ).await;

        let join_handle = tokio::spawn(gasket.run());
        let gasket_info = GasketInfo {
            id,
            join_handle,
        };

        self.add_gasket(gasket_info);
    }

    /// 关闭 dorodoro-bangumi
    pub async fn shutdown(&self) {
        let mut handles = vec![];
        {
            let ft = &mut self.future_token.lock_pe();
            ft.0.cancel();
            mem::swap(&mut ft.1, &mut handles);
        }
        
        for handle in handles {
            handle.await.unwrap();
        }

        for mut item in self.gaskets.iter_mut() {
            let handle = &mut item.join_handle;
            handle.await.unwrap()
        }
        match self.store.flush_all() {
            Ok(_) => info!("关机前 flush 数据到存储设备成功"),
            Err(e) => error!("flush 数据到存储设备失败！！！\t{}", e),
        }

        // 关机
        if !Context::global().is_cancelled() {
            Context::global().cancel();
        }
    }
}