use std::sync::{Arc, OnceLock, RwLock};

use ahash::HashMap;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use doro_util::sync::{ReadLockExt, WriteLockExt};

use crate::entity::{Group, Quarter, Resource, Source};

pub mod entity;

#[async_trait]
pub trait Crawler: Sync + Send + 'static {
    /// 列出所有季度
    async fn list_quarter(&self) -> Result<Vec<Quarter>>;

    /// 列出所有资源
    async fn list_resource(&self) -> Result<Vec<Resource>>;

    /// 列出指定季度的资源
    async fn list_resource_from_quarter(&self, quarter: Quarter) -> Result<Vec<Resource>>;

    /// 列出指定资源的组
    async fn list_source_group(&self, resource_id: &str) -> Result<Vec<Group>>;

    /// 列出指定资源的订阅源
    async fn list_subscribe_sources(&self, resource_id: &str) -> Result<Vec<Vec<Source>>>;

    /// 搜索资源
    async fn search_resource(&self, keyword: &str) -> Result<(Vec<Resource>, Vec<Source>)>;
}

/// 动漫资源爬取器管理
#[derive(Default)]
pub struct BangumiCrawler {
    /// 资源爬取器注册
    crawlers: RwLock<HashMap<String, Arc<dyn Crawler>>>,
}

impl BangumiCrawler {
    pub fn global() -> &'static Self {
        static ROUTER: OnceLock<BangumiCrawler> = OnceLock::new();
        ROUTER.get_or_init(BangumiCrawler::default)
    }

    pub fn register_crawler(&self, name: &str, crawler: Arc<dyn Crawler>) -> Result<()> {
        if self.crawlers.write_pe().insert(name.to_string(), crawler).is_some() {
            return Err(anyhow!("crawler already registered"));
        }
        Ok(())
    }

    pub fn get_crawler(&self, name: &str) -> Option<Arc<dyn Crawler>> {
        self.crawlers.read_pe().get(name).cloned()
    }
}
