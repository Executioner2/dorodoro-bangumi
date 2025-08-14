//! 任务，其作用是管理 peer，执行一次任务，任务结束后，任务完成销毁。
//! 以共享锁的方式进行多线程运行。因为 peer 需要从 task 拿取信息，
//! 用 channel 的方式会大幅增加代码复杂度。

use std::any::Any;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use doro_util::global::Id;
use tokio::sync::mpsc::Sender;

pub mod content;
pub mod magnet;

pub type Subscriber = Sender<Box<dyn Any + Send + Sync + 'static>>;

pub type Async<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub trait Task: Send + Sync + 'static {
    /// 获取任务的唯一标识符
    fn get_id(&self) -> Id;

    /// 设置任务的回调函数  
    fn set_callback(&self, callback: Box<dyn TaskCallback>);

    /// 启动任务
    fn start(&self) -> Async<Result<()>>;

    /// 暂停任务
    fn pause(&self) -> Async<Result<()>>;

    ///关闭任务
    fn shutdown(&self) -> Async<()>;

    /// 订阅任务的内部执行信息
    fn subscribe_inside_info(&self, subscriber: Subscriber);
}

pub trait TaskCallback: Send + Sync + 'static {
    /// 任务完成
    fn finish(&self, id: Id) -> Async<()>;

    /// 任务错误
    fn error(&self, id: Id, error: anyhow::Error) -> Async<()>;
}

#[derive(Debug, Clone, Copy)]
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