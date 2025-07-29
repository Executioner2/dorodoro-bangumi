//! 任务，其作用是管理 peer，执行一次任务，任务结束后，任务完成销毁。
//! 以共享锁的方式进行多线程运行。因为 peer 需要从 task 拿取信息，
//! 用 channel 的方式会大幅增加代码复杂度。

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use doro_util::global::Id;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::pin::Pin;

pub mod content;

/// 订阅者，用于接收任务内部执行信息
pub trait Subscriber {
    /// 通知订阅者任务内部执行信息 - 字符串形式
    fn notify_str(&self, message: String);

    /// 通知订阅者任务内部执行信息 - 字节形式
    fn notify_bytes(&self, data: Bytes);
}

pub type Async<T> = Pin<Box<dyn Future<Output = T> + Send + Sync + 'static>>;

pub trait Task: Send + Sync + 'static {
    /// 获取任务的唯一标识符
    fn get_id(&self) -> Id;

    /// 启动任务
    fn start(&self) -> Async<Result<()>>;

    /// 暂停任务
    fn pause(&self) -> Async<Result<()>>;

    ///关闭任务
    fn shutdown(&self) -> Async<Result<()>>;

    /// 订阅任务的内部执行信息
    fn subscribe_inside_info(&self, subscriber: Box<dyn Subscriber + Send + 'static>) -> Async<()>;

    /// 回调通知，当任务结束时通知到 task_manager
    fn callback(&self) -> Async<()>;
}

#[derive(Debug)]
pub enum HostSource {
    Tracker,
    DHT,
}

impl Display for HostSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            HostSource::Tracker => write!(f, "Tracker"),
            HostSource::DHT => write!(f, "DHT"),
        }
    }
}

/// 接收主机地址
#[async_trait]
pub trait ReceiveHost {
    /// 接收主机地址
    async fn receive_host(&self, host: SocketAddr, source: HostSource);

    /// 接收多个主机地址
    async fn receive_hosts(&self, hosts: Vec<SocketAddr>, source: HostSource);
}
