use crate::context::Context;
use crate::mapper::torrent::{TorrentMapper, TorrentStatus};
use crate::rss;
use crate::store::Store;
use crate::task::Task;
use crate::task::content::DownloadContent;
use anyhow::Result;
use dashmap::DashMap;
use doro_util::global::{GlobalId, Id};
use doro_util::sync::MutexExt;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::task::JoinSet;
use tracing::{error, info};

static TASK_MANAGER: OnceLock<TaskManager> = OnceLock::new();

#[derive(Clone, Default)]
pub struct PeerId {
    peer_id: Arc<[u8; 20]>,
}

impl PeerId {
    pub fn new(peer_id: [u8; 20]) -> Self {
        Self {
            peer_id: Arc::new(peer_id),
        }
    }

    pub fn value(&self) -> &[u8; 20] {
        &self.peer_id
    }
}

pub struct TaskManager {
    /// 执行的任务
    tasks: Arc<DashMap<Id, Box<dyn Task + Send + Sync + 'static>>>,

    /// 异步任务（这个被销毁时，会自动中断里面的任务，因此不必特意执行 shutdown）
    /// 如果要手动执行 shutdown，需要将 Mutex 换成 tokio::sync::Mutex。
    handles: Arc<Mutex<JoinSet<()>>>,
}

impl TaskManager {
    pub async fn init() {
        let task_manager = TASK_MANAGER.get_or_init(|| TaskManager {
            tasks: Arc::new(DashMap::new()),
            handles: Arc::new(Mutex::new(JoinSet::new())),
        });
        task_manager.init_handle().await;
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
                if let Some(torrent) = torrent.serail {
                    let id = GlobalId::next_id();
                    let peer_id = self.get_peer_id();
                    let task = DownloadContent::new(id, peer_id, torrent);
                    self.handle_task(Box::new(task)).await.unwrap();
                } else {
                    panic!("严重的数据错误！{torrent:#?}");
                }
            }
        }
    }

    /// 启动 rss。因为启动 rss 之后会扫描订阅源，并将订阅源中的种子加入下载队列，
    /// 所以必须在启动 rss 之前启动 peer manager
    fn start_rss(&self) {
        self.handles
            .lock_pe()
            .spawn(Box::pin(rss::interval_refresh()));
    }
}

/// 对外常用接口
impl TaskManager {
    /// 获取全局实例
    pub fn global() -> &'static Self {
        TASK_MANAGER.get().unwrap()
    }

    pub fn get_peer_id(&self) -> PeerId {
        PeerId::new(doro_util::rand::gen_peer_id())
    }

    /// 处理任务
    pub async fn handle_task(&self, task: Box<dyn Task>) -> Result<()> {
        task.start().await?;
        self.tasks.insert(task.get_id(), task);
        Ok(())
    }

    /// 关闭 dorodoro-bangumi
    pub async fn shutdown(&self) {
        for task in self.tasks.iter_mut() {
            if let Err(e) = task.shutdown().await {
                error!("关闭任务失败：{:#?}", e);
            }
        }

        match Store::global().flush_all() {
            Ok(_) => info!("关机前 flush 数据到存储设备成功"),
            Err(e) => error!("flush 数据到存储设备失败！！！\t{}", e),
        }

        // 关机
        if !Context::global().is_cancelled() {
            Context::global().cancel();
        }
    }
}
