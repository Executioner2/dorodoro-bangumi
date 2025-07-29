use anyhow::Result;
use doro_util::default_logger;
use rss::Channel;
use tracing::{Level, info};

default_logger!(Level::DEBUG);

/// rss 解析
#[tokio::test]
async fn test_rss_parse() -> Result<()> {
    let content = reqwest::get("https://mikanani.me/RSS/Bangumi?bangumiId=3649&subgroupid=370 ")
        .await?
        .bytes()
        .await?;
    let channel = Channel::read_from(&content[..])?;
    info!("{:?}", channel);
    Ok(())
}
