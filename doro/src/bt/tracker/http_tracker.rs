//! HTTP Tracker 实现

#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use doro_util::bendy_ext::{Bytes2Object, SocketAddrExt};
use crate::bt::constant::http_tracker::HTTP_REQUEST_TIMEOUT;
use crate::tracker::{AnnounceInfo, Event};
use anyhow::{Result, anyhow};
use bendy::decoding::{Error, FromBencode, Object, ResultExt};
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::warn;
use crate::task_handler::PeerId;

#[derive(Debug)]
pub struct Announce {
    /// 下次 announce 间隔（秒）
    pub interval: u64,

    /// 当前未完成下载的 peer 数（UDP tracker 的 leechers）
    pub incomplete: u64,

    /// 已完成下载的 peer 数（UDP tracker 的 seeders）
    pub complete: u64,

    /// 已下载的数据量（字节）
    pub downloaded: Option<u64>,

    /// peer 主机列表
    pub peers: Vec<SocketAddr>,

    /// 最小 announce 间隔（秒）
    pub min_interval: Option<u64>,

    /// peer 主机列表（IPv6）
    pub peers6: Vec<SocketAddr>,
}

impl FromBencode for Announce {
    fn decode_bencode_object(object: Object) -> std::result::Result<Self, Error>
    where
        Self: Sized,
    {
        let mut interval = None;
        let mut incomplete = None;
        let mut complete = None;
        let mut downloaded = None;
        let mut peers: Option<Vec<SocketAddrExt>> = None;
        let mut min_interval = None;
        let mut peers6: Option<Vec<SocketAddrExt>> = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"interval", value) => {
                    interval = u64::decode_bencode_object(value)
                        .context("interval")
                        .map(Some)?;
                }
                (b"incomplete", value) => {
                    incomplete = u64::decode_bencode_object(value)
                        .context("incomplete")
                        .map(Some)?;
                }
                (b"complete", value) => {
                    complete = u64::decode_bencode_object(value)
                        .context("complete")
                        .map(Some)?;
                }
                (b"downloaded", value) => {
                    downloaded = u64::decode_bencode_object(value)
                        .context("downloaded")
                        .map(Some)?;
                }
                (b"peers", Object::Bytes(value)) => {
                    peers = Some(value.to_object()?);
                }
                (b"min interval", value) => {
                    min_interval = u64::decode_bencode_object(value)
                        .context("min interval")
                        .map(Some)?;
                }
                (b"peers6", Object::Bytes(value)) => {
                    peers6 = Some(value.to_object()?);
                }
                (unknown_field, _) => {
                    warn!("未知的字段: {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let interval = interval.ok_or_else(|| Error::missing_field("interval"))?;
        let incomplete = incomplete.ok_or_else(|| Error::missing_field("incomplete"))?;
        let complete = complete.ok_or_else(|| Error::missing_field("complete"))?;
        let peers = peers.unwrap_or_default().into_iter().map(|x| x.into()).collect::<Vec<SocketAddr>>();
        let peers6 = peers6.unwrap_or_default().into_iter().map(|x| x.into()).collect::<Vec<SocketAddr>>();

        Ok(Announce {
            interval,
            incomplete,
            complete,
            downloaded,
            peers,
            min_interval,
            peers6,
        })
    }
}

pub struct HttpTracker {
    announce: String,
    info_hash: Arc<[u8; 20]>,
    peer_id: PeerId,
    next_request_time: u64,
}

impl HttpTracker {
    pub fn new(announce: String, info_hash: Arc<[u8; 20]>, peer_id: PeerId) -> Self {
        Self {
            announce,
            info_hash,
            peer_id,
            next_request_time: 0,
        }
    }

    pub fn next_request_time(&self) -> u64 {
        self.next_request_time
    }

    pub fn unusable(&mut self) -> u64 {
        self.next_request_time = u64::MAX;
        self.next_request_time
    }

    pub fn announce(&self) -> &str {
        &self.announce
    }

    /// 向 Tracker 发送广播请求
    ///
    /// 正常情况下返回可用资源的地址
    pub async fn announcing(&mut self, event: Event, info: &AnnounceInfo) -> Result<Announce> {
        let download = info.download.load(Ordering::Acquire);
        let left = info.resource_size.saturating_sub(download);
        let query_url = format!(
            "{}?info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact=1&event={}",
            self.announce,
            percent_encode(self.info_hash.as_slice(), NON_ALPHANUMERIC).to_string(),
            percent_encode(self.peer_id.value().as_slice(), NON_ALPHANUMERIC).to_string(),
            info.port,
            info.uploaded.load(Ordering::Acquire),
            download,
            left,
            event.to_string(),
        );
        let response = if let Ok(Ok(response)) =
            tokio::time::timeout(HTTP_REQUEST_TIMEOUT, reqwest::get(&query_url)).await
        {
            response
        } else {
            return Err(anyhow!("HTTP request timeout"));
        };

        if !response.status().is_success() {
            return Err(anyhow!(
                "HTTP request failed with status code {}: {}",
                response.status(),
                response.text().await?,
            ));
        }

        let resp = response.bytes().await?;
        Announce::from_bencode(&resp).map_err(|e| anyhow!("解析 tracker 返回数据失败: {}", e))
    }
}
