use crate::buffer::ByteBuffer;
use crate::core::alias::{ReceiverTcpServer, SenderScheduler, SenderTcpServer};
use crate::core::command::CommandHandler;
use crate::core::config::Config;
use crate::core::controller::Controller;
use crate::core::protocol;
use crate::core::protocol::{Identifier, Protocol};
use crate::core::runtime::Runnable;
use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::option::Option;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio::sync::Mutex;
use tokio::sync::mpsc::channel;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, trace, warn};

/// 读超时时间 60 秒
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// 链接id，一般需要 TcpServer 管理资源释放的才需要这个
type ConnId = u64;

struct ConnInfo {
    id: ConnId,
    join_handle: JoinHandle<()>,
}

/// 多线程下的共享数据
pub struct TcpServerContext {
    ss: SenderScheduler,
    sts: SenderTcpServer,
    cancel_token: CancellationToken,
    conn_id: Arc<AtomicU64>,
    conns: Arc<Mutex<HashMap<ConnId, ConnInfo>>>,
    config: Config,
}

impl TcpServerContext {
    pub async fn remove_conn(&self, conn_id: ConnId) {
        let mut conns = self.conns.lock().await;
        conns.remove(&conn_id);
    }
}

pub struct TcpServer {
    /// 向调度器发送命令
    send: SenderScheduler,

    /// 监听地址
    addr: SocketAddr,

    /// 关机信号
    cancel_token: CancellationToken,

    /// 链接增长 id
    conn_id: Arc<AtomicU64>,

    /// 链接
    conns: Arc<Mutex<HashMap<ConnId, ConnInfo>>>,

    /// 与 tcp server 通信的 channel
    channel: (SenderTcpServer, ReceiverTcpServer),

    /// 配置项
    config: Config,
}

impl TcpServer {
    pub fn new(config: Config, cancel_token: CancellationToken, send: SenderScheduler) -> Self {
        TcpServer {
            send,
            addr: config.tcp_server_addr(),
            cancel_token,
            conn_id: Arc::new(AtomicU64::new(0)),
            conns: Arc::new(Mutex::new(HashMap::new())),
            channel: channel(config.channel_buffer()),
            config,
        }
    }

    fn get_context(&self) -> TcpServerContext {
        TcpServerContext {
            ss: self.send.clone(),
            sts: self.channel.0.clone(),
            cancel_token: self.cancel_token.clone(),
            conn_id: self.conn_id.clone(),
            conns: self.conns.clone(),
            config: self.config.clone(),
        }
    }

    async fn shutdown(self) {
        let mut conns = self.conns.lock().await;
        trace!("等待关闭的子线程数量: {}", conns.len());
        for (_, conn) in conns.iter_mut() {
            let join_handle = &mut conn.join_handle;
            join_handle.await.unwrap()
        }
    }

    async fn accept(mut socket: TcpStream, context: TcpServerContext) {
        select! {
            _ = context.cancel_token.cancelled() => {
                trace!("accpet socket 接收到关机信号");
            },
            result = Accept::new(&mut socket)  => {
                match result {
                    Some(protocol) => {
                        Self::protocol_dispatch(context, socket, protocol).await;
                    },
                    None => {
                        trace!("accpet socket 接收到关闭信号");
                    }
                }
            }
        }
    }

    /// 协议分发，根据协议 ID 分发到不同的处理函数
    #[inline(always)]
    async fn protocol_dispatch(context: TcpServerContext, socket: TcpStream, protocol: Protocol) {
        match protocol.id {
            Identifier::BitTorrent => {
                trace!("接收到 BitTorrent 协议");
            }
            Identifier::RemoteControl => {
                trace!("接收到 RemoteControl 协议");
                let id = context
                    .conn_id
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let controller = Controller::new(
                    id,
                    socket,
                    context.cancel_token,
                    context.ss,
                    context.sts,
                    context.config,
                );
                controller.run().await;
            }
        }
    }
}

impl Runnable for TcpServer {
    async fn run(mut self) {
        let listener: TcpListener;
        match TcpListener::bind(&self.addr).await {
            Ok(l) => {
                info!("tcp server 正在监听 {}", self.addr);
                listener = l;
            }
            Err(e) => {
                error!("tcp server 绑定地址失败: {}", e);
                self.cancel_token.cancel();
                return;
            }
        }

        loop {
            select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                res = listener.accept() => {
                    match res {
                        Ok((socket, addr)) => {
                            trace!("tcp server 接收到连接: {}", addr);
                            let context = self.get_context();
                            tokio::spawn(Self::accept(socket, context));
                        },
                        Err(e) => {
                            warn!("tcp server 接收连接错误: {}", e);
                        }
                    }
                }
                res = self.channel.1.recv() => {
                    if let Some(cmd) = res {
                        trace!("tcp server 收到了消息: {:?}", cmd);
                        tokio::spawn(cmd.handle(self.get_context()));
                    }
                }
            }
        }

        info!("tcp server 等待资源释放");
        self.shutdown().await;
        info!("tcp server 已关闭");
    }
}

struct Accept<'a> {
    socket: &'a mut TcpStream,
    size: usize,
    protocol: Option<Bytes>,
    protocol_id: Option<Identifier>,
    state: State,
}

#[derive(Eq, PartialEq)]
enum State {
    ProtocolLen,
    Protocol,
    ParseProtocol,
    Finished,
}

impl<'a> Accept<'a> {
    fn new(socket: &'a mut TcpStream) -> Self {
        Self {
            socket,
            size: 1, // 协议长度占用 1 个字节
            protocol: None,
            protocol_id: None,
            state: State::ProtocolLen,
        }
    }

    fn read(&mut self, size: usize, cx: &mut Context<'_>) -> Option<Poll<Bytes>> {
        if size == 0 {
            return Some(Poll::Ready(Bytes::new()));
        }
        let mut buf = ByteBuffer::new(size);
        let mut binding = buf.as_mut();
        let future = self.socket.read_exact(&mut binding);
        match pin!(future).poll(cx) {
            Poll::Ready(Ok(0)) => {
                trace!("客户端主动说bye-bye");
                None
            }
            Poll::Ready(Ok(_len)) => Some(Poll::Ready(Bytes::from_owner(buf))),
            Poll::Ready(Err(e)) => {
                warn!("因神秘力量，和客户端失去了联系\t{}", e);
                None
            }
            Poll::Pending => Some(Poll::Pending),
        }
    }

    /// 解析出协议和协议载荷长度
    fn parse_protocol(&self, protocol: &[u8]) -> Option<(Identifier, usize)> {
        match protocol {
            protocol::BIT_TORRENT_PROTOCOL => {
                Some((Identifier::BitTorrent, protocol::BIT_TORRENT_PAYLOAD_LEN))
            }
            protocol::REMOTE_CONTROL_PROTOCOL => Some((
                Identifier::RemoteControl,
                protocol::REMOTE_CONTROL_PAYLOAD_LEN,
            )),
            _ => None,
        }
    }
}

impl<'a> Future for Accept<'_> {
    type Output = Option<Protocol>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let accept = unsafe { self.get_unchecked_mut() };
        let buf = match accept.read(accept.size, cx) {
            Some(Poll::Ready(buf)) => buf,
            Some(Poll::Pending) => {
                trace!("protocol 的数据还没准备好");
                return Poll::Pending;
            }
            None => return Poll::Ready(None),
        };

        match accept.state {
            State::ProtocolLen => {
                accept.size = buf[0] as usize;
                accept.state = State::Protocol;
            }
            State::Protocol => {
                accept.protocol = Some(buf);
                let protocol = accept.protocol.as_ref().unwrap();
                accept.state = State::ParseProtocol;
                if let Some((id, size)) = accept.parse_protocol(protocol) {
                    accept.size = size;
                    accept.protocol_id = Some(id);
                } else {
                    warn!("未知协议: {}", String::from_utf8_lossy(protocol));
                    return Poll::Ready(None);
                }
            }
            State::ParseProtocol => {
                let id = accept.protocol_id.take().unwrap();
                let protocol = Protocol { id, payload: buf };
                return Poll::Ready(Some(protocol));
            }
            State::Finished => {
                return Poll::Ready(None);
            }
        }
        pin!(accept).poll(cx)
    }
}
