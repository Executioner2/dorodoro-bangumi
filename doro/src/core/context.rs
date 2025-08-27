//! 全局上下文

use core::fmt::Formatter;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

use crate::config::{ClientAuth, Config, ConfigInner};
use crate::db::{ConnWrapper, Db};

/// 异步任务信号量
pub struct AsyncSemaphore {
    /// 异步任务数量计数
    task_count: Arc<AtomicUsize>,

    /// 禁止 sync，只能 send
    _not_sync: std::marker::PhantomData<Mutex<()>>,
}

impl AsyncSemaphore {
    fn new(task_count: Arc<AtomicUsize>) -> Self {
        Self {
            task_count,
            _not_sync: std::marker::PhantomData,
        }
    }
}

impl Drop for AsyncSemaphore {
    fn drop(&mut self) {
        self.task_count.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct Context {
    /// 数据库连接池
    db: Arc<Db>,

    /// 全局配置信息
    config: Config,

    /// 异步任务数量计数
    async_task_count: Arc<AtomicUsize>,

    /// 异步启动 peer 数量计数
    async_peer_start_count: Arc<AtomicUsize>,

    /// 全局停机监听
    cancel_token: CancellationToken,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Context").finish()
    }
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

impl Context {
    /// 实例化全局上下文
    pub fn init(db: Db, config: Config) {
        CONTEXT
            .set(Self {
                db: Arc::new(db),
                config,
                cancel_token: CancellationToken::new(),
                async_task_count: Arc::new(AtomicUsize::new(0)),
                async_peer_start_count: Arc::new(AtomicUsize::new(0)),
            })
            .unwrap();
    }

    pub fn global() -> &'static Self {
        CONTEXT.get().unwrap()
    }

    /// 设置全局配置信息
    pub fn set_config(config: ConfigInner) {
        CONTEXT.get().unwrap().config.update_config(config);
    }

    /// 更新客户端认证信息
    pub fn update_auth(auth: ClientAuth) {
        Context::global().config.update_auth(auth);
    }

    /// 返回全局配置信息
    pub fn get_config() -> &'static Config {
        &Context::global().config
    }

    /// 返回一个数据库链接
    pub async fn get_conn() -> Result<ConnWrapper> {
        let conn = Context::global().db.get_conn().await?;
        Ok(conn)
    }

    /// 停机监听
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel_token.cancelled()
    }

    /// 停机令牌
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    /// 关机
    pub fn cancel(&self) {
        self.cancel_token.cancel()
    }

    /// 是否关机
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// 获取异步任务信号量
    pub fn take_async_task_semaphore() -> Option<AsyncSemaphore> {
        let context = Context::global();
        let current_count =
            context
                .async_task_count
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                    if current < context.config.async_task_pool_size() {
                        Some(current + 1)
                    } else {
                        None
                    }
                });

        match current_count {
            Ok(_) => {
                let async_task_count = context.async_task_count.clone();
                let async_semaphore = AsyncSemaphore::new(async_task_count);
                Some(async_semaphore)
            }
            Err(_) => None,
        }
    }

    /// 获取 peer 异步启动信号量
    pub fn take_async_peer_start_semaphore() -> Option<AsyncSemaphore> {
        let context = Context::global();
        let current_count =
            context
                .async_task_count
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                    if current < context.config.async_peer_start_pool_size() {
                        Some(current + 1)
                    } else {
                        None
                    }
                });

        match current_count {
            Ok(_) => {
                let async_peer_start_count = context.async_peer_start_count.clone();
                let async_semaphore = AsyncSemaphore::new(async_peer_start_count);
                Some(async_semaphore)
            }
            Err(_) => None,
        }
    }
}
