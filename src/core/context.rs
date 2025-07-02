//! 全局上下文

use crate::config::Config;
use crate::db::{ConnWrapper, Db};
use std::sync::Arc;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use anyhow::Result;
use crate::dht::node_id::NodeId;

#[derive(Clone)]
pub struct Context {
    db: Arc<Db>,
    node_id: Arc<NodeId>,
    config: Config,
    cancel_token: CancellationToken,
}

impl Context {
    /// 实例化全局上下文
    pub fn new(db: Db, config: Config, node_id: NodeId) -> Self {
        Self {
            db: Arc::new(db),
            node_id: Arc::new(node_id),
            config,
            cancel_token: CancellationToken::new(),
        }
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
    
    /// 返回节点ID
    pub fn get_node_id(&self) -> Arc<NodeId> {
        self.node_id.clone()
    }
}
