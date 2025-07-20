//! RSS 订阅实现。主要功能：
//! 1. 管理订阅源（创建、手动更新、删除、重命名、编辑 URL）
//! 2. 订阅列表已读未读标记
//! 3. 自动更新订阅源
//! 4. 自动解析订阅源内容，提取有效链接
//! 5. 自动对更新资源进行下载
//! 6. 自定义过滤规则，过滤掉不必要的资源（item）

mod command;

use crate::command::CommandHandler;
use crate::context::Context;
use crate::core::rss::command::Command;
use doro_util::datetime;
use crate::emitter::Emitter;
use crate::emitter::constant::RSS;
use crate::emitter::transfer::TransferPtr;
use crate::mapper::rss::{RSSEntity, RSSMapper};
use crate::rss::command::ConsumerNotRead;
use crate::runtime::{CommandHandleResult, CustomTaskResult, Runnable};
use crate::scheduler::Scheduler;
use crate::scheduler::command::{TorrentAdd, TorrentSource};
use crate::torrent::{Parse, TorrentArc};
use anyhow::{Result, anyhow};
use bytes::Bytes;
use futures::stream::FuturesUnordered;
use rss::Channel;
use sha1::{Digest, Sha1};
use std::pin::Pin;
use std::time::Duration;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing::error;

/// HTTP 请求超时时间
pub const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 订阅源刷新间隔 15 分钟
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub struct RSS {
    /// 全局上下文
    ctx: Context,

    /// 命令发送器
    emitter: Emitter,

    /// 任务取消标记
    cancel_token: CancellationToken,
}

impl RSS {
    pub fn new(ctx: Context, emitter: Emitter, cancel_token: CancellationToken) -> Self {
        Self { ctx, emitter, cancel_token }
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

        let mut conn = self.ctx.get_conn().await?;
        let res = conn.add_subscribe(rss_entity);
        if let Ok(true) = res {
            self.emitter
                .send(
                    &RSS::get_transfer_id(""),
                    ConsumerNotRead { url, channel }.into(),
                )
                .await?;
        }
        res
    }

    pub async fn consumer_not_read(&self, url: String, channel: Channel) -> Result<()> {
        let conn = self.ctx.get_conn().await?;
        let rss_entity = conn
            .get_rss_by_url(&url)?
            .ok_or(anyhow!("not found subscribe by url: {}", url))?;

        let read_guids = conn.list_mark_read_guid(&url)?; // 已读的 guid
        let download_path = self.ctx.get_config().default_download_dir();

        for item in channel.items {
            if let (Some(guid), Some(enclosure)) = (item.guid, item.enclosure) {
                if !read_guids.contains(&guid.value) {
                    // todo - 应该还要有一些其他过滤条件
                    match Self::load_torrent_from_url(enclosure.url()).await {
                        Ok(torrent) => {
                            let cmd = TorrentAdd {
                                torrent,
                                path: download_path.clone(),
                                source: TorrentSource::RSSFeed(rss_entity.id.unwrap(), guid.value),
                            }
                            .into();
                            self.emitter
                                .send(&Scheduler::get_transfer_id(""), cmd)
                                .await?;
                        }
                        Err(e) => {
                            error!("load torrent from url error: {}", e)
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn load_torrent_from_url(url: &str) -> Result<TorrentArc> {
        let content = tokio::time::timeout(HTTP_REQUEST_TIMEOUT, reqwest::get(url))
            .await??
            .bytes()
            .await?;
        TorrentArc::parse_torrent(content)
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
        let conn = self.ctx.get_conn().await?;
        let last_update = datetime::now_secs() - REFRESH_INTERVAL.as_secs();
        let rss_entities = conn.list_subscribe_not_update(last_update)?;
        for rss_entity in rss_entities {
            let (channel, content) = Self::get_subscribe_channel(rss_entity.url.as_ref().unwrap()).await?;
            let mut hasher = Sha1::new();
            hasher.update(&content);
            let hash = hex::encode(hasher.finalize().as_mut_slice());
            if hash != rss_entity.hash.unwrap() {
                conn.update_subscribe(rss_entity.id.unwrap(), datetime::now_secs(), &hash)?;
                self.emitter
                    .send(
                        &RSS::get_transfer_id(""),
                        ConsumerNotRead {
                            url: rss_entity.url.unwrap(),
                            channel,
                        }
                        .into(),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// 注册定时刷新任务
    fn register_interval_refresh(
        &self,
    ) -> Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>> {
        let id = Self::get_transfer_id(self.get_suffix());
        let emitter = self.emitter.clone();
        Box::pin(async move {
            loop {
                let _ = emitter
                    .send(&id, command::RSSFeedFlush.into())
                    .await;
                tokio::time::sleep(REFRESH_INTERVAL).await;
            }
        })
    }
}

impl Runnable for RSS {
    fn emitter(&self) -> &Emitter {
        &self.emitter
    }

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
        // 不用全局上下文中的信号量，因为 RSS 的运行周期收到 peer_manager 管控
        self.cancel_token.cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }
}
