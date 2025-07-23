use crate::api::rss_api::RSSFeed;
use crate::rss::RSS;
use anyhow::Result;

/// 添加 rss 订阅
pub async fn add_rss_feed(rss_feed: RSSFeed) -> Result<bool> {
    RSS::subscribe(rss_feed.url, rss_feed.name).await
}
