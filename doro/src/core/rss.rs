//! RSS 订阅实现。主要功能：
//! 1. 管理订阅源（创建、手动更新、删除、重命名、编辑 URL）
//! 2. 订阅列表已读未读标记
//! 3. 自动更新订阅源
//! 4. 自动解析订阅源内容，提取有效链接
//! 5. 自动对更新资源进行下载
//! 6. 自定义过滤规则，过滤掉不必要的资源（item）

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use bytes::Bytes;
use doro_util::datetime;
use rss::Channel;
use sha1::{Digest, Sha1};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::error;

use crate::api::task_api::{Task, TorrentSource};
use crate::context::Context;
use crate::mapper::rss::{RSSEntity, RSSMapper};
use crate::task_service;

/// HTTP 请求超时时间
pub const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 订阅源刷新间隔 15 分钟
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub async fn subscribe(url: String, title: Option<String>) -> Result<bool> {
    let (channel, content) = get_subscribe_channel(&url).await?;
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

    let res = {
        let mut conn = Context::global().get_conn().await?;
        conn.add_subscribe(rss_entity)
    };

    if let Ok(true) = res {
        consumer_not_read(&url, channel).await?;
    }
    res
}

async fn consumer_not_read(url: &String, channel: Channel) -> Result<()> {
    let (rss_entity, read_guids) = {
        let conn = Context::global().get_conn().await?;
        let rss_entity = conn
            .get_rss_by_url(url)?
            .ok_or(anyhow!("not found subscribe by url: {}", url))?;

        let read_guids = conn.list_mark_read_guid(url)?; // 已读的 guid

        (rss_entity, read_guids)
    };

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

async fn get_subscribe_channel(url: &str) -> Result<(Channel, Bytes)> {
    let content = tokio::time::timeout(HTTP_REQUEST_TIMEOUT, reqwest::get(url))
        .await??
        .bytes()
        .await?;
    let channel = Channel::read_from(&content[..])?;

    Ok((channel, content))
}

async fn flush_feed(rss_entity: Arc<RSSEntity>, permit: OwnedSemaphorePermit) -> Result<()> {
    let (channel, content) = get_subscribe_channel(rss_entity.url.as_ref().unwrap()).await?;
    let mut hasher = Sha1::new();
    hasher.update(&content);
    let hash = hex::encode(hasher.finalize().as_mut_slice());
    if hash != *rss_entity.hash.as_ref().unwrap() {
        let conn = Context::global().get_conn().await?;
        conn.update_subscribe(rss_entity.id.unwrap(), datetime::now_secs(), &hash)?;
        consumer_not_read(rss_entity.url.as_ref().unwrap(), channel).await?;
    }
    drop(permit);
    Ok(())
}

pub async fn flush_all_feeds() -> Result<()> {
    let conn = Context::global().get_conn().await?;
    let last_update = datetime::now_secs() - REFRESH_INTERVAL.as_secs();
    let rss_entities = conn.list_subscribe_not_update(last_update)?;
    let hash_semaphore = Arc::new(Semaphore::new(
        Context::get_config().rss_refresh_concurrency(),
    ));
    let mut handles = Vec::with_capacity(rss_entities.len());

    for rss_entity in rss_entities {
        let permit = hash_semaphore.clone().acquire_owned().await?;
        let rss_entity = Arc::new(rss_entity);
        handles.push((
            tokio::spawn(Box::pin(flush_feed(rss_entity.clone(), permit))),
            rss_entity,
        ));
    }

    for (handle, rss_entity) in handles {
        match handle.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                error!("刷新 rss [{:?}] 订阅失败！{}", rss_entity.title, e)
            }
            Err(e) => {
                error!("刷新 rss [{:?}] 订阅失败！{}", rss_entity.title, e)
            }
        }
    }

    Ok(())
}

/// 注册定时刷新订阅任务
pub async fn interval_refresh() {
    let mut tick = tokio::time::interval(REFRESH_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    
    loop {
        tick.tick().await;
        if let Err(e) = flush_all_feeds().await {
            error!("刷新 rss 订阅失败！{}", e)
        }
    }
}
