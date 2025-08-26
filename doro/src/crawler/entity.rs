use ahash::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 季度
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct Quarter {
    /// 所在组
    pub group: String,

    /// 季度名称
    pub name: String,
}

/// 资源
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct Resource {
    /// 资源 ID
    pub id: String,

    /// 资源名称
    pub name: String,

    /// 资源链接
    pub link: String,

    /// 最后一次更新日期
    pub last_update: Option<DateTime<Utc>>,

    /// 封面链接地址
    pub image_url: String,

    /// 扩展数据
    pub extend: HashMap<String, String>,
}

/// 资源组
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct Group {
    /// 资源组 ID
    pub id: String,

    /// 资源组名称
    pub name: String,
}

/// 资源源
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct Source {
    /// 资源所在组
    pub group: Option<Group>,

    /// 资源源名称
    pub name: String,

    /// 更新日期
    pub update_date: DateTime<Utc>,

    /// 种子链接
    pub torrent_url: String,

    /// 磁力链接
    pub magnet_url: String,

    /// 详情页地址
    pub detail_url: String,

    /// 资源大小
    pub size: String,
}