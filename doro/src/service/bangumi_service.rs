use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::entity::{Group, Quarter, Resource, Source};
use crate::{BangumiCrawler, Crawler};

/// 获取指定名称的爬取器
fn get_crawler(name: &str) -> Result<Arc<dyn Crawler>> {
    BangumiCrawler::global()
        .get_crawler(name)
        .ok_or(anyhow!("[{name}] Crawler not found"))
}

// ===========================================================================
// 对外开放的函数
// ===========================================================================

/// 列出番剧列表
pub async fn list_bangumi(crawler: String, quarter: Option<Quarter>) -> Result<Vec<Resource>> {
    let crawler = get_crawler(&crawler)?;

    if let Some(q) = quarter {
        crawler.list_resource_from_quarter(q).await
    } else {
        crawler.list_resource().await
    }
}

/// 列出季度列表
pub async fn list_quarter(crawler: String) -> Result<Vec<Quarter>> {
    let crawler = get_crawler(&crawler)?;
    crawler.list_quarter().await
}

/// 列出上传了该资源的字幕组
pub async fn list_source_group(crawler: String, resource_id: String) -> Result<Vec<Group>> {
    let crawler = get_crawler(&crawler)?;
    crawler.list_source_group(&resource_id).await
}

/// 列出资源的订阅源
pub async fn list_subscribe_sources(crawler: String, resource_id: String) -> Result<Vec<Vec<Source>>> {
    let crawler = get_crawler(&crawler)?;
    crawler.list_subscribe_sources(&resource_id).await
}

/// 搜索资源
pub async fn search_resource(crawler: String, keyword: String) -> Result<(Vec<Resource>, Vec<Source>)> {
    let crawler = get_crawler(&crawler)?;
    crawler.search_resource(&keyword).await
}