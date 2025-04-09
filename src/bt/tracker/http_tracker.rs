//! HTTP Tracker 实现

pub mod error;

#[cfg(test)]
mod tests;

use crate::bt::bencoding;
use crate::bt::bencoding::BEncodeHashMap;
use crate::tracker;
use crate::tracker::http_tracker::error::Error::{
    FieldValueError, MissingField, ResponseStatusNotOk,
};
use crate::tracker::{Event, Host};
use error::Result;
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Peer {
    peer_id: String,
    host: Host,
}

#[derive(Debug)]
pub struct Announce {
    /// 下次 announce 间隔（秒）
    interval: u64,

    /// 当前未完成下载的 peer 数（UDP tracker 的 leechers）
    incomplete: u64,

    /// 已完成下载的 peer 数（UDP tracker 的 seeders）
    complete: u64,

    /// 已下载的数据量（字节）
    downloaded: u64,

    /// peer 主机列表
    peers: Vec<Host>,

    /// 最小 announce 间隔（秒）
    min_interval: Option<u64>,

    /// peer 主机列表（IPv6）
    peers6: Vec<Host>,
}

struct HttpTracker<'a> {
    announce: &'a str,
    info_hash: &'a [u8; 20],
    peer_id: [u8; 20],
    download: u64,
    left: u64,
    uploaded: u64,
    port: u16,
}

impl<'a> HttpTracker<'a> {
    pub fn new(
        announce: &'a str,
        info_hash: &'a [u8; 20],
        peer_id: [u8; 20],
        download: u64,
        left: u64,
        uploaded: u64,
        port: u16,
    ) -> Self {
        Self {
            announce,
            info_hash,
            peer_id,
            download,
            left,
            uploaded,
            port,
        }
    }

    /// 向 Tracker 发送广播请求
    ///
    /// 正常情况下返回可用资源的地址
    pub fn announcing(&self, event: Event) -> Result<Announce> {
        let query_url = format!(
            "{}?info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact=1&event={}",
            self.announce,
            percent_encode(self.info_hash, NON_ALPHANUMERIC).to_string(),
            percent_encode(&self.peer_id, NON_ALPHANUMERIC).to_string(),
            self.port,
            self.uploaded,
            self.download,
            self.left,
            event.to_string(),
        );
        let response = reqwest::blocking::get(&query_url)?;

        if !response.status().is_success() {
            return Err(ResponseStatusNotOk(response.status(), response.text()?));
        }

        let encode = bencoding::decode(response.bytes()?)?;
        let encode = encode
            .as_dict()
            .ok_or(bencoding::error::Error::TransformError)?;

        let interval = encode.get_int("interval").ok_or(MissingField("interval"))?;
        if interval < 0 {
            return Err(FieldValueError(format!("interval = {}", interval)));
        }

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
                Some(min_interval as u64)
            }
            None => None,
        };

        let peers = encode
            .as_bytes_conetnt("peers")
            .ok_or(MissingField("peers"))?;
        let peers = tracker::parse_peers_v4(&peers)?;

        let peers6 = match encode.as_bytes_conetnt("peers6") {
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
