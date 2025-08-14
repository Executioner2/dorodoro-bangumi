use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use dashmap::DashMap;
use doro_util::global::{GlobalId, Id};
use doro_util::sync::MutexExt;
use tokio::task::JoinSet;
use tracing::{error, info};

use crate::context::Context;
use crate::mapper::torrent::{TorrentMapper, TorrentStatus};
use crate::rss;
use crate::store::Store;
use crate::task::{Async, Task, TaskCallback};
use crate::task::content::DownloadContent;

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
    tasks: Arc<DashMap<Id, Box<dyn Task>>>,

    /// 任务回调
    callback: TaskManagerCallback,

    /// 异步任务（这个被销毁时，会自动中断里面的任务，因此不必特意执行 shutdown）
    /// 如果要手动执行 shutdown，需要将 Mutex 换成 tokio::sync::Mutex。
    handles: Arc<Mutex<JoinSet<()>>>,
}

impl TaskManager {
    pub async fn init() {
        let task_manager = TASK_MANAGER.get_or_init(|| {
            let tasks = Arc::new(DashMap::new());
            let callback = TaskManagerCallback { tasks: tasks.clone() };
            TaskManager {
                tasks,
                callback,
                handles: Arc::new(Mutex::new(JoinSet::new())),
            }
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
        for entity in torrents {
            if entity.status == Some(TorrentStatus::Download) && entity.serail.is_some() {
                if let Some(torrent) = entity.serail {
                    let id = GlobalId::next_id();
                    let peer_id = self.get_peer_id();
                    let save_path = entity.save_path.unwrap();
                    let task = DownloadContent::new(id, peer_id, torrent, save_path).await.unwrap();
                    self.handle_task(Box::new(task)).await.unwrap();
                } else {
                    panic!("严重的数据错误！{entity:#?}");
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
        task.set_callback(Box::new(self.callback.clone()));
        task.start().await?;
        self.tasks.insert(task.get_id(), task);
        Ok(())
    }

    /// 关闭指定的任务
    pub async fn shutdown_task(&self, id: Id) {
        if let Some((_, task)) = self.tasks.remove(&id) {
            task.shutdown().await;
        }
    }

    /// 关闭 dorodoro-bangumi
    pub async fn shutdown(&self) {
        for task in self.tasks.iter_mut() {
            task.shutdown().await;
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

#[derive(Clone)]
struct TaskManagerCallback {
    /// 执行的任务
    tasks: Arc<DashMap<Id, Box<dyn Task>>>,
}

impl TaskCallback for TaskManagerCallback {
    /// 任务完成
    fn finish(&self, id: Id) -> Async<()> {
        // 这里 remove task，会保证 task 实例被销毁，task 中的资源在 task 被销毁
        // 时自动释放掉，因此不需要在 task 中特意执行 shutdown
        let task = self.tasks.remove(&id);
        Box::pin(async move {
            if let Some((_, task)) = task {
                task.shutdown().await;
            }
        })
    }

    /// 任务错误
    fn error(&self, id: Id, error: anyhow::Error) -> Async<()> {
        let task = self.tasks.remove(&id);
        error!("任务 {id} 出错：{error:#?}", id = id, error = error);
        Box::pin(async move {
            if let Some((_, task)) = task {
                task.shutdown().await;
            }
        })
    }
}