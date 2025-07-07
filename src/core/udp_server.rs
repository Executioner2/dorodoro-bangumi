//! udp 服务器实现
//!
//! 因为需要多个端共用，为了区分响应与请求的对应关系，需要 transaction id 来标识。
//! 当某一个端通过 UdpServer 向指定地址发起请求，那么该请求被分配一个 transaction id，
//! 完成请求后，返回一个 Future 给调用者。当相同 transaction id 的响应数据达到时，会通过
//! 这个 Future 通知到调用者。
//!
//! todo: transcation id 是否需要全局唯一，还是每个地址唯一即可？即是通过 【地址端口 + 端事务 ID】作为唯一标识
//!       还是 【全局事务 ID】作为唯一标识？
//!
//! 暂时先用上面的后者作为唯一标识。

use crate::buffer::ByteBuffer;
use crate::context::Context;
use crate::dht::entity::DHTBase;
use bendy::decoding::FromBencode;
use bendy::value::Value;
use bytes::Bytes;
use dashmap::DashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::task::{Poll, Waker};
use tokio::net::UdpSocket;
use tracing::{error, info};
use anyhow::Result;

/// 循环 id
struct CycleId {
    val: AtomicU16,
}

impl CycleId {
    fn new() -> Self {
        Self { val: AtomicU16::new(0) }
    }

    fn next_tran_id(&self) -> u16 {
        self.val.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct UdpServer {
    /// 监听的地址
    socket: Arc<UdpSocket>,

    /// 正在处理的请求。
    /// Waker 可能为 None，即发送数据到调用 Response poll 之间的空档期
    inflight: Arc<DashMap<u16, Option<Waker>>>,

    /// 响应结果
    result_sotre: Arc<DashMap<u16, (Bytes, SocketAddr)>>,

    /// 全局上下文
    context: Context,

    /// 循环 id
    cycle_id: Arc<CycleId>,
}

impl UdpServer {
    pub async fn new(context: Context) -> Result<Self> {
        let bind = context.get_config().udp_server_addr();
        let socket = UdpSocket::bind(bind).await?;
        Ok(Self {
            socket: Arc::new(socket),
            inflight: Arc::new(DashMap::new()),
            result_sotre: Arc::new(DashMap::new()),
            context,
            cycle_id: Arc::new(CycleId::new()),
        })
    }

    pub async fn request(
        &self,
        tran_id: u16,
        data: &[u8],
        addr: &SocketAddr,
    ) -> Result<Response, std::io::Error> {
        if self.inflight.contains_key(&tran_id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "same tran_id",
            ));
        }

        self.inflight.insert(tran_id, None);
        if let Err(e) = self.socket.send_to(data, addr).await{
            self.inflight.remove(&tran_id);
            return Err(e);
        }
        
        Ok(Response::new(
            tran_id,
            self.inflight.clone(),
            self.result_sotre.clone(),
        ))
    }

    pub fn tran_id(&self) -> u16 {
        self.cycle_id.next_tran_id()
    }

    fn packet_size(&self) -> usize {
        self.context.get_config().udp_packet_limit()
    }

    /// 解析收到的数据
    fn parse_recv_data(&self, data: Bytes, addr: SocketAddr) {
        match DHTBase::<Value>::from_bencode(data.as_ref()) {
            Ok(base) => {
                // todo - 这里可以优化，重复解析了
                let id = base.t();
                if let Some((_, resp)) = self.inflight.remove(&id) {
                    self.result_sotre.insert(id, (data, addr));
                    resp.map(|waker| waker.wake());
                }
            }
            Err(e) => {
                error!("parse dht resp error: {}", e);
            }
        }
    }
}

impl UdpServer {
    pub async fn run(self) {
        let mut recv_buf = ByteBuffer::new(self.packet_size());
        loop {
            tokio::select! {
                _ = self.context.cancelled() => {
                    break;
                }
                result = self.socket.recv_from(recv_buf.as_mut()) => {
                    match result {
                        Ok((size, addr)) => {
                            recv_buf.resize(size);
                            self.parse_recv_data(recv_buf.take(), addr);
                        }
                        Err(e) => {
                            error!("udp server recv error: {}", e);
                        }
                    }
                    recv_buf = ByteBuffer::new(self.packet_size());
                }
            }
        }

        info!("udp server 已退出");
    }
}

#[derive(Debug)]
pub struct Response {
    tran_id: u16,
    inflight: Arc<DashMap<u16, Option<Waker>>>,
    result_sotre: Arc<DashMap<u16, (Bytes, SocketAddr)>>,
}

impl Response {
    pub fn new(
        tran_id: u16,
        inflight: Arc<DashMap<u16, Option<Waker>>>,
        result_sotre: Arc<DashMap<u16, (Bytes, SocketAddr)>>,
    ) -> Self {
        Self {
            tran_id,
            inflight,
            result_sotre,
        }
    }
}

impl Future for Response {
    type Output = (Bytes, SocketAddr);

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        match this.result_sotre.remove(&this.tran_id) {
            Some((_, addr)) => Poll::Ready(addr),
            None => {
                this.inflight.insert(this.tran_id, Some(cx.waker().clone()));
                Poll::Pending
            }
        }
    }
}

impl Drop for Response {
    fn drop(&mut self) {
        self.inflight.remove(&self.tran_id);
        self.result_sotre.remove(&self.tran_id);
    }
}
