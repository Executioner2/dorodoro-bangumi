//! 这个 bin 是 bt 模块的主要架构模型。架构设计见 docs 中的系统架构图

use crate::peer::PeerManager;
use crate::torrent::Torrent;
use doro_util::buffer::ByteBuffer;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{Level, error, info, warn};
use doro_util::default_logger;

pub mod torrent {
    /// Torrent
    pub struct Torrent {}
}

pub mod tracker {
    /// 这个代表 Tracker 模块
    pub struct Tracker {}
}

pub mod peer {
    use crate::{Command, PeerCommand};
    use doro_util::buffer::ByteBuffer;
    use std::collections::HashMap;

    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;
    use tokio::sync::mpsc::{Receiver, Sender, channel};
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;
    use tracing::{error, info};

    /// Peer
    struct Peer {
        id: u64,
        socket: TcpStream,
        cancel_token: CancellationToken,
        send: Sender<PeerCommand>,
        recv: Receiver<PeerCommand>,
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
                    _command = self.recv.recv() => {
                        info!("接收到了peer manage的指令");
                        true
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
                                        self.send.send(PeerCommand::Exit(self.id)).await.unwrap(); // 告诉 PeerManage，自己要主动退出了
                                        false
                                    }
                                }
                            },
                            Err(e) => {
                                error!("读取对端 peer 传来的数据失败，错误信息：{}", e);
                                self.send.send(PeerCommand::Exit(self.id)).await.unwrap(); // 告诉 PeerManage，自己要主动退出了
                                false
                            }
                        }
                    }
                }
            }
            info!("peer 退出处理请求");
        }
    }

    /// 记录一些 peer 的信息
    pub struct PeerInfo {
        id: u64,
        _send: Sender<PeerCommand>,
        join_handle: JoinHandle<()>,
    }

    /// Peer 管理器
    pub struct PeerManager {
        pub send: Sender<Command>, // 发送给 scheduler 的
        pub cancel_token: CancellationToken,
        peers: HashMap<u64, PeerInfo>,
        peer_id: u64,
        channel: (Sender<PeerCommand>, Receiver<PeerCommand>),
    }

    impl PeerManager {
        pub fn new(send: Sender<Command>, cancel_token: CancellationToken) -> Self {
            Self {
                send,
                cancel_token,
                peers: HashMap::new(),
                peer_id: 0,
                channel: channel(100),
            }
        }

        pub fn get_sender(&self) -> Sender<PeerCommand> {
            self.channel.0.clone()
        }

        pub async fn process(mut self) {
            let mut tsuziki_masuka = true;
            while tsuziki_masuka {
                tsuziki_masuka = tokio::select! {
                    _ = self.cancel_token.cancelled() => {
                        self.join_peers().await;
                        false
                    },
                    command = self.channel.1.recv() => {
                        match command {
                            Some(PeerCommand::RequestConnection(socket)) => {
                                info!("有peer申请某个区块的资源，处理请求");
                                let (send, recv) = channel(100);
                                let peer = Peer {
                                    id: self.peer_id,
                                    socket,
                                    cancel_token: self.cancel_token.clone(),
                                    send: self.channel.0.clone(),
                                    recv
                                };
                                let join_handle = tokio::spawn(peer.start());

                                let peer_info = PeerInfo {
                                    id: self.peer_id,
                                    _send: send,
                                    join_handle
                                };

                                self.peer_id += 1;
                                self.peers.insert(peer_info.id, peer_info);
                            },
                            Some(PeerCommand::Exit(id)) => {
                                info!("peer[{}]主动退出了", id);
                                self.peers.remove(&id).map(async |peer_info| {
                                    info!("删除peer[{}]", peer_info.id);
                                    peer_info.join_handle.await.unwrap();
                                });
                            }
                            _ => ()
                        }
                        true
                    }
                }
            }
        }

        pub async fn join_peers(&mut self) {
            info!("等待peer线程退出，当前handles数量: {}", self.peers.len());
            for (_, handle) in self.peers.iter_mut() {
                let handle = &mut handle.join_handle;
                handle.await.unwrap()
            }
        }
    }
}

pub enum TorrentCommand {
    Add(Torrent),
}

pub enum PeerCommand {
    RequestConnection(TcpStream),
    Exit(u64), // peer 退出了
}

/// 控制命令
pub enum Command {
    Torrent(TorrentCommand),
    Peer(PeerCommand),
    Shutdown,
}

/// 控制器，用于接收操作指令
struct Controller {
    send: Sender<Command>,
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
    send: Sender<Command>,

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

    async fn process(mut socket: TcpStream, send: Sender<Command>) {
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
                let controller = Controller {
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
    peer_manager_send: Sender<PeerCommand>,
}

impl Scheduler {
    fn new(
        recv: Receiver<Command>,
        context: Context,
        cancel_token: CancellationToken,
        peer_manager_send: Sender<PeerCommand>,
    ) -> Self {
        Self {
            recv,
            context,
            cancel_token,
            peer_manager_send,
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
                    self.peer_command(command).await;
                }
                Command::Shutdown => {
                    info!("收到关机命令");
                    break;
                }
            }
        }

        info!("退出程序");
        self.cancel_token.cancel(); // 发出关机
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

    async fn peer_command(&mut self, command: PeerCommand) {
        self.peer_manager_send.send(command).await.unwrap()
    }
}

struct Bootstrap {}

impl Bootstrap {
    async fn start() {
        info!("启动程序");

        let (send, recv) = tokio::sync::mpsc::channel(100);
        let cancel_token = CancellationToken::new();

        // 启动 TcpServer
        let tcp_server = TcpServer {
            send: send.clone(),
            cancel_token: cancel_token.clone(),
        };

        let tcp_server_handle = tokio::spawn(tcp_server.start());

        let context = Context { torrent: vec![] };
        let peer_manager = PeerManager::new(send, cancel_token.clone());
        let scheduler = Scheduler::new(recv, context, cancel_token, peer_manager.get_sender());

        let peer_manageer_handle = tokio::spawn(peer_manager.process());

        scheduler.start().await;

        info!("等待资源关闭");

        tcp_server_handle.await.unwrap(); // 执行到这里，说明已经发出了停机指令，等待 TcpServer 线程退出
        peer_manageer_handle.await.unwrap();

        info!("关闭程序")
    }
}

default_logger!();

#[tokio::main]
async fn main() {
    Bootstrap::start().await;
}
