//! HTTP Tracker 实现

pub mod error;

#[cfg(test)]
mod tests;

use crate::bt::bencoding;
use crate::bt::bencoding::BEncodeHashMap;
use crate::bt::constant::http_tracker::HTTP_REQUEST_TIMEOUT;
use crate::tracker;
use crate::tracker::http_tracker::error::Error::{
    FieldValueError, MissingField, ResponseStatusNotOk,
};
use crate::tracker::{AnnounceInfo, Event};
use error::Result;
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[derive(Debug)]
pub struct Announce {
    /// 下次 announce 间隔（秒）
    pub interval: u64,

    /// 当前未完成下载的 peer 数（UDP tracker 的 leechers）
    pub incomplete: u64,

    /// 已完成下载的 peer 数（UDP tracker 的 seeders）
    pub complete: u64,

    /// 已下载的数据量（字节）
    pub downloaded: u64,

    /// peer 主机列表
    pub peers: Vec<SocketAddr>,

    /// 最小 announce 间隔（秒）
    pub min_interval: Option<u64>,

    /// peer 主机列表（IPv6）
    pub peers6: Vec<SocketAddr>,
}

pub struct HttpTracker {
    announce: String,
    info_hash: Arc<[u8; 20]>,
    peer_id: Arc<[u8; 20]>,
    next_request_time: u64,
}

impl HttpTracker {
    pub fn new(announce: String, info_hash: Arc<[u8; 20]>, peer_id: Arc<[u8; 20]>) -> Self {
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
        let left = info.resource_size - download;
        let query_url = format!(
            "{}?info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact=1&event={}",
            self.announce,
            percent_encode(self.info_hash.as_slice(), NON_ALPHANUMERIC).to_string(),
            percent_encode(self.peer_id.as_slice(), NON_ALPHANUMERIC).to_string(),
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
            return Err(error::Error::TimeoutError);
        };

        if !response.status().is_success() {
            return Err(ResponseStatusNotOk(
                response.status(),
                response.text().await?,
            ));
        }

        let encode = bencoding::decode(response.bytes().await?)?;
        let encode = encode
            .as_dict()
            .ok_or(bencoding::error::Error::TransformError)?;

        let interval = encode.get_int("interval").ok_or(MissingField("interval"))?;
        if interval < 0 {
            return Err(FieldValueError(format!("interval = {}", interval)));
        }
        self.next_request_time += interval as u64;

        let complete = encode.get_int("complete").ok_or(MissingField("complete"))?;
        if complete < 0 {
            return Err(FieldValueError(format!("complete = {}", complete)));
        }

        let incomplete = encode
            .get_int("incomplete")
            .ok_or(MissingField("incomplete"))?;
        if incomplete < 0 {
            return Err(FieldValueError(format!("incomplete = {}", incomplete)));
        }

        let downloaded = encode
            .get_int("downloaded")
            .ok_or(MissingField("downloaded"))?;
        if downloaded < 0 {
            return Err(FieldValueError(format!("downloaded = {}", downloaded)));
        }

        let min_interval = match encode.get_int("min interval") {
            Some(min_interval) => {
                if min_interval < 0 {
                    return Err(FieldValueError(format!("{}", min_interval)));
                }
                self.next_request_time += min_interval as u64;
                Some(min_interval as u64)
            }
            None => None,
        };

        let peers = encode
            .get_bytes_conetnt("peers")
            .ok_or(MissingField("peers"))?;
        let peers = tracker::parse_peers_v4(&peers)?;

        let peers6 = match encode.get_bytes_conetnt("peers6") {
            Some(peers6) => tracker::parse_peers_v6(peers6)?,
            None => vec![],
        };

        Ok(Announce {
            interval: interval as u64,
            incomplete: incomplete as u64,
            complete: complete as u64,
            downloaded: downloaded as u64,
            peers,
            min_interval,
            peers6,
        })
    }
}
