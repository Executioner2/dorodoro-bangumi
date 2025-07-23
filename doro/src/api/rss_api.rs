use crate::{register_route, rss_service};
use serde::{Deserialize, Serialize};
use doro_macro::route;
use crate::router::ret::Ret;
use anyhow::Result;

#[derive(Serialize, Deserialize, Debug)]
pub struct  RSSFeed {
    /// 订阅名
    pub name: Option<String>,

    /// 订阅链接
    pub url: String,
}

// ===========================================================================
// API
// ===========================================================================

/// 添加 rss 订阅
#[route(code = 1101)]
pub async fn add_rss_feed(rss_feed: RSSFeed) -> Result<Ret<bool>> {
    let ret = rss_service::add_rss_feed(rss_feed).await?;
    Ok(Ret::ok(ret))
}