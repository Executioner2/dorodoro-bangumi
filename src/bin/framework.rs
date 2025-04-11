//! 这个 bin 是 bt 模块的主要架构模型。架构设计见 docs 中的系统架构图

use crate::peer::PeerManager;
use crate::torrent::Torrent;
use dorodoro_bangumi::log;
use dorodoro_bangumi::tracker::udp_tracker::buffer::ByteBuffer;
use std::collections::{HashMap};
use std::sync::{Arc};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;
use tracing::{Level, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;

pub mod torrent {
    /// Torrent
    pub struct Torrent {}
}

pub mod tracker {
    /// 这个代表 Tracker 模块
    pub struct Tracker {}
}

pub mod peer {
    use crate::Command;
    use dorodoro_bangumi::tracker::udp_tracker::buffer::ByteBuffer;
    use std::collections::{HashMap};
    use std::sync::{Arc};
    
    
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;
    use tracing::{error, info};

    /// Peer
    struct Peer {
        socket: TcpStream,
        cancel_token: CancellationToken,
        send: Arc<tokio::sync::mpsc::Sender<Command>>,
    }

    impl Peer {
        async fn start(mut self) {
            info!("开始处理peer的请求");
            let mut tsuzuki_masuka = true;
            while tsuzuki_masuka {
                tsuzuki_masuka = tokio::select! {
                    _ = self.cancel_token.cancelled() => {
                        info!("peer 取消了请求");
                        false
                    },
                    result = self.socket.read_u32() => {
                        match result {
                            Ok(len) => {
                                let mut buf = ByteBuffer::new(len as usize);
                                match self.socket.read(&mut buf.as_mut()).await {
                                    Ok(n) => {
                                        info!("读取到对端 peer 传来的数据，长度为 {}", n);
                                        true
                                    },
                                    Err(e) => {
                                        error!("读取对端 peer 传来的数据失败，错误信息：{}", e);
                                        false
                                    }
                                }
                            },
                            Err(e) => {
                                error!("读取对端 peer 传来的数据失败，错误信息：{}", e);
                                false
                            }
                        }
                    }
                };
            }
            info!("peer 退出处理请求");
        }

        /// 是否还要继续处理请求与响应？
        ///
        /// 根据当前下载速率等多方面因素判断是否还要继续和这个peer合作
        fn tsuzuki_masuka(&self) -> bool {
            !self.cancel_token.is_cancelled()
        }
    }

    /// 记录一些 peer 的信息
    pub struct PeerInfo {}

    /// Peer 管理器
    pub struct PeerManager {
        pub peers: HashMap<String, PeerInfo>, // 实际应该是个map，这里只做流程确定，细节忽略
        pub send: Arc<tokio::sync::mpsc::Sender<Command>>,
        pub join_handles: Vec<JoinHandle<()>>,
    }

    impl PeerManager {
        pub fn process(&mut self, socket: TcpStream, cancel_token: CancellationToken) {
            info!("有peer申请某个区块的资源，处理请求");
            let peer = Peer {
                socket,
                cancel_token,
                send: self.send.clone(),
            };
            let handle = tokio::spawn(peer.start());
            self.join_handles.push(handle);
        }

        pub async fn join_peers(&mut self) {
            info!(
                "等待peer线程退出，当前handles数量: {}",
                self.join_handles.len()
            );
            for handle in self.join_handles.iter_mut() {
                handle.abort();
                handle.await.unwrap();
            }
        }
    }
}

pub enum TorrentCommand {
    Add(Torrent),
}

pub enum PeerCommand {
    RequestConnection(TcpStream),
}

/// 控制命令
pub enum Command {
    Torrent(TorrentCommand),
    Peer(PeerCommand),
    Shutdown,
}

/// 控制器，用于接收操作指令
struct Controller {
    send: Arc<tokio::sync::mpsc::Sender<Command>>,
    cancel_token: CancellationToken,
    socket: TcpStream,
}

impl Controller {
    async fn process(mut self) {
        info!("这里接收数据，读取数据，并解析");
        let mut tsuzuki_masuka = true;
        while tsuzuki_masuka {
            tsuzuki_masuka = tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    info!("关闭控制器");
                    false
                },
                result = self.socket.read_u8() => {
                    match result {
                        Ok(read) => {
                            match read {
                                0 => {
                                    info!("收到关闭信号");
                                    self.send.send(Command::Shutdown).await.unwrap();
                                }
                                1 => {
                                    info!("收到添加种子的命令");
                                    self.send.send(Command::Torrent(TorrentCommand::Add(Torrent {})))
                                        .await
                                        .unwrap();
                                }
                                _ => {
                                    info!("收到未知命令");
                                }
                            }
                            true
                        },
                        Err(e) => {
                            error!("读取命令失败，错误信息：{}", e);
                            false
                        }
                    }
                }
            };
        }
    }
}

/// 网络服务端
struct TcpServer {
    /// 向调度器发送命令
    send: Arc<tokio::sync::mpsc::Sender<Command>>,

    /// 关机信号
    cancel_token: CancellationToken,
}

impl TcpServer {
    async fn start(self) {
        let listener = TcpListener::bind("0.0.0.0:3315").await.unwrap();
        let mut tsuzuki_masuka = true;
        while tsuzuki_masuka {
            info!("等待客户端连接");
            tsuzuki_masuka = tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    info!("收到关机命令，退出TcpServer");
                    false
                },
                result = listener.accept() => {
                    match result {
                        Ok((socket, _)) => {
                            tokio::spawn(Self::process(socket, self.send.clone()));
                        }
                        Err(e) => {
                            error!("监听失败，错误信息：{}", e);
                        }
                    }
                    true
                }
            }
        }
        info!("TcpServer 退出")
    }

    async fn process(mut socket: TcpStream, send: Arc<tokio::sync::mpsc::Sender<Command>>) {
        info!("读取请求，区分是啥协议");
        let protocol_len = socket.read_u8().await.unwrap();
        if protocol_len == 0 {
            warn!("错误的协议长度");
            return;
        }
        info!("protocol_len: {}", protocol_len);
        let mut protocol = ByteBuffer::new(protocol_len as usize);
        socket.read(&mut protocol.as_mut()).await.unwrap();
        match protocol.as_ref() {
            b"BitTorrent protocol" => {
                info!("收到 BitTorrent 请求");
                send.send(Command::Peer(PeerCommand::RequestConnection(socket)))
                    .await
                    .unwrap();
            }
            b"dorodoro-bangumi" => {
                info!("收到 dorodoro-bangumi 请求，即客户端控制请求");
                let mut controller = Controller {
                    send: send.clone(),
                    cancel_token: CancellationToken::new(),
                    socket,
                };
                controller.process().await;
            }
            _ => {
                warn!("不支持的协议");
            }
        }
    }
}

/// 上下文
struct Context {
    torrent: Vec<Torrent>,
}

/// 调度器
struct Scheduler {
    recv: Receiver<Command>,
    context: Context,
    cancel_token: CancellationToken,
    peer_manager: PeerManager,
}

impl Scheduler {
    fn new(
        recv: Receiver<Command>,
        context: Context,
        cancel_token: CancellationToken,
        peer_manager: PeerManager,
    ) -> Self {
        Self {
            recv,
            context,
            cancel_token,
            peer_manager,
        }
    }

    async fn start(mut self) {
        info!("启动调度器");
        while let Some(command) = self.recv.recv().await {
            match command {
                Command::Torrent(command) => {
                    self.torrent_command(command);
                }
                Command::Peer(command) => {
                    self.peer_command(command);
                }
                Command::Shutdown => {
                    info!("收到关机命令");
                    break;
                }
            }
        }

        info!("退出程序");
        self.cancel_token.cancel(); // 发出关机
        self.peer_manager.join_peers().await; // 等待 peer 线程退出
    }

    fn torrent_command(&mut self, command: TorrentCommand) {
        match command {
            TorrentCommand::Add(torrent) => {
                info!("添加种子，并启动peer");
                self.context.torrent.push(torrent);
                // 假装启动了peer
            }
        }
    }

    fn peer_command(&mut self, command: PeerCommand) {
        match command {
            PeerCommand::RequestConnection(socket) => {
                info!("收到peer的连接请求");
                self.peer_manager.process(socket, self.cancel_token.clone());
            }
        }
    }
}

struct Bootstrap {}

impl Bootstrap {
    async fn start() {
        info!("启动程序");

        let (send, recv) = tokio::sync::mpsc::channel(100);
        let cancel_token = CancellationToken::new();
        let send = Arc::new(send);

        // 启动 TcpServer
        let tcp_server = TcpServer {
            send: send.clone(),
            cancel_token: cancel_token.clone(),
        };

        let tcp_server_handle = tokio::spawn(tcp_server.start());

        let context = Context { torrent: vec![] };
        let peer_manager = PeerManager {
            peers: HashMap::new(),
            send,
            join_handles: Vec::new(),
        };
        let scheduler = Scheduler::new(recv, context, cancel_token, peer_manager);
        scheduler.start().await;
        tcp_server_handle.await.unwrap(); // 执行到这里，说明已经发出了停机指令，等待 TcpServer 线程退出
        info!("关闭程序")
    }
}

/// 全局初始化
fn global_init() -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    let guard = log::register_logger("logs", "dorodoro-bangumi", 10 << 20, 2, Level::INFO)?;
    Ok(guard)
}

#[tokio::main]
async fn main() {
    let _guard = global_init().unwrap();
    // let scheduler = Scheduler::new();
    // let handle = scheduler.start();
    // handle.await.unwrap();
    Bootstrap::start().await;
}
