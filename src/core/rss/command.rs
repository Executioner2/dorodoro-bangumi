use crate::core::rss::RSS;
use crate::command_system;
use anyhow::Result;
use rss::Channel;
use tracing::{error, info, warn};

command_system! {
    ctx: RSS,
    Command {
        Subscribe,
        ConsumerNotRead,
        RSSFeedFlush
    }
}

#[derive(Debug)]
pub struct Subscribe {
    url: String,
    title: Option<String>
}
impl<'a> CommandHandler<'a, Result<()>> for Subscribe {
    type Target = &'a mut RSS;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        match ctx.subscribe(self.url.clone(), self.title).await {
            Ok(true) => {
                // rss 订阅成功
                info!("订阅 RSS [{}]成功，并且开始自动下载资源", self.url)
            },
            Ok(false) => {
                // 订阅失败，可能是已经订阅过了
                warn!("已经订阅过 RSS [{}]", self.url)
            }
            Err(e) => {
                error!("订阅 RSS [{}]失败\t{}", self.url, e)
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ConsumerNotRead {
    pub url: String,
    pub channel: Channel
}
impl<'a> CommandHandler<'a, Result<()>> for ConsumerNotRead {
    type Target = &'a mut RSS;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.consumer_not_read(self.url, self.channel).await
    }
}

#[derive(Debug)]
pub struct RSSFeedFlush;
impl<'a> CommandHandler<'a, Result<()>> for RSSFeedFlush {
    type Target = &'a mut RSS;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.flush_all_feeds().await
    }
}