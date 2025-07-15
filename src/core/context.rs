//! 全局上下文

use core::fmt::Formatter;
use crate::config::Config;
use crate::db::{ConnWrapper, Db};
use std::sync::{Arc, OnceLock};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use anyhow::Result;

#[derive(Clone)]
pub struct Context {
    db: Arc<Db>,
    config: Config,
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
        CONTEXT.set(Self {
            db: Arc::new(db),
            config,
            cancel_token: CancellationToken::new(),
        }).unwrap();
    }

    pub fn global() -> &'static Self {
        CONTEXT.get().unwrap()
    }

    /// 返回全局配置信息
    pub fn get_config(&self) -> &Config {
        &self.config
    }

    /// 返回一个数据库链接
    pub async fn get_conn(&self) -> Result<ConnWrapper> {
        let conn = self.db.get_conn().await?;
        Ok(conn)
    }

    /// 停机监听
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel_token.cancelled()
    }

    /// 关机
    pub fn cancel(&self) {
        self.cancel_token.cancel()
    }

    /// 是否关机
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }
}
