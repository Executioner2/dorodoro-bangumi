//! RSS 订阅实现。主要功能：
//! 1. 管理订阅源（创建、手动更新、删除、重命名、编辑 URL）
//! 2. 订阅列表已读未读标记
//! 3. 自动更新订阅源
//! 4. 自动解析订阅源内容，提取有效链接
//! 5. 自动对更新资源进行下载
//! 6. 自定义过滤规则，过滤掉不必要的资源（item）

mod command;

use crate::api::task_api::{Task, TorrentSource};
use crate::command::CommandHandler;
use crate::context::Context;
use crate::core::rss::command::Command;
use crate::emitter::Emitter;
use crate::emitter::constant::RSS;
use crate::emitter::transfer::TransferPtr;
use crate::mapper::rss::{RSSEntity, RSSMapper};
use crate::rss::command::ConsumerNotRead;
use crate::runtime::{CommandHandleResult, CustomTaskResult, Runnable};
use crate::task_service;
use anyhow::{Result, anyhow};
use bytes::Bytes;
use doro_util::datetime;
use futures::stream::FuturesUnordered;
use rss::Channel;
use sha1::{Digest, Sha1};
use std::pin::Pin;
use std::time::Duration;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

/// HTTP 请求超时时间
pub const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 订阅源刷新间隔 15 分钟
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub struct RSS {
    /// 任务取消标记
    cancel_token: CancellationToken,
}

impl RSS {
    pub fn new(cancel_token: CancellationToken) -> Self {
        Self { cancel_token }
    }

    pub async fn subscribe(&self, url: String, title: Option<String>) -> Result<bool> {
        let (channel, content) = Self::get_subscribe_channel(&url).await?;
        let mut hasher = Sha1::new();
        hasher.update(&content);
        let hash = hex::encode(hasher.finalize().as_mut_slice());

        let rss_entity = RSSEntity {
            title: Some(title.unwrap_or(channel.title.clone())),
            url: Some(url.clone()),
            hash: Some(hash),
            last_update: Some(datetime::now_secs()),
            ..Default::default()
        };

        let mut conn = Context::global().get_conn().await?;
        let res = conn.add_subscribe(rss_entity);
        if let Ok(true) = res {
            #[rustfmt::skip]
            Emitter::global().send(
                &RSS::get_transfer_id(""),
                ConsumerNotRead { url, channel }.into(),
            ).await?;
        }
        res
    }

    pub async fn consumer_not_read(&self, url: String, channel: Channel) -> Result<()> {
        let conn = Context::global().get_conn().await?;
        let rss_entity = conn
            .get_rss_by_url(&url)?
            .ok_or(anyhow!("not found subscribe by url: {}", url))?;

        let read_guids = conn.list_mark_read_guid(&url)?; // 已读的 guid

        for item in channel.items {
            if let (Some(guid), Some(enclosure)) = (item.guid, item.enclosure) {
                if !read_guids.contains(&guid.value) {
                    // todo - 应该还要有一些其他过滤条件
                    let task = Task {
                        task_name: None,
                        download_path: None,
                        source: TorrentSource::RSSFeed(
                            rss_entity.id.unwrap(),
                            guid.value,
                            enclosure.url,
                        ),
                    };
                    task_service::add_task(task).await?;
                }
            }
        }

        Ok(())
    }

    pub async fn get_subscribe_channel(url: &str) -> Result<(Channel, Bytes)> {
        let content = tokio::time::timeout(HTTP_REQUEST_TIMEOUT, reqwest::get(url))
            .await??
            .bytes()
            .await?;
        let channel = Channel::read_from(&content[..])?;

        Ok((channel, content))
    }

    pub async fn flush_all_feeds(&self) -> Result<()> {
        let conn = Context::global().get_conn().await?;
        let last_update = datetime::now_secs() - REFRESH_INTERVAL.as_secs();
        let rss_entities = conn.list_subscribe_not_update(last_update)?;
        for rss_entity in rss_entities {
            let (channel, content) =
                Self::get_subscribe_channel(rss_entity.url.as_ref().unwrap()).await?;
            let mut hasher = Sha1::new();
            hasher.update(&content);
            let hash = hex::encode(hasher.finalize().as_mut_slice());
            if hash != rss_entity.hash.unwrap() {
                conn.update_subscribe(rss_entity.id.unwrap(), datetime::now_secs(), &hash)?;
                #[rustfmt::skip]
                Emitter::global().send(
                    &RSS::get_transfer_id(""),
                    ConsumerNotRead {
                        url: rss_entity.url.unwrap(),
                        channel,
                    }.into(),
                ).await?;
            }
        }
        Ok(())
    }

    /// 注册定时刷新订阅任务
    fn register_interval_refresh(
        &self,
    ) -> Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>> {
        let id = Self::get_transfer_id(self.get_suffix());
        Box::pin(async move {
            loop {
                let _ = Emitter::global()
                    .send(&id, command::RSSFeedFlush.into())
                    .await;
                tokio::time::sleep(REFRESH_INTERVAL).await;
            }
        })
    }
}

impl Runnable for RSS {
    fn get_transfer_id<T: ToString>(_suffix: T) -> String {
        RSS.to_string()
    }

    fn register_lt_future(
        &mut self,
    ) -> FuturesUnordered<Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>>> {
        let futures = FuturesUnordered::new();
        futures.push(self.register_interval_refresh());
        futures
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        // 不用全局上下文中的信号量，因为 RSS 的运行周期收到 task_handler 管控
        self.cancel_token.cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }
}
