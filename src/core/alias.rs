use crate::core::command::{gasket, peer, peer_manager, scheduler, tcp_server};
use crate::core::controller::RespClient;
use tokio::sync::mpsc::{Receiver, Sender};

/// 发送命令给 scheduler
pub type SenderScheduler = Sender<scheduler::Command>;

/// 接收到 scheduler 的命令
pub type ReceiverScheduler = Receiver<scheduler::Command>;

/// 发送命令给 peer manager
pub type SenderPeerManager = Sender<peer_manager::Command>;

/// 接收发送给 peer manager 的
pub type ReceiverPeerManager = Receiver<peer_manager::Command>;

/// 发送命令给 tcp server
pub type SenderTcpServer = Sender<tcp_server::Command>;

/// 接收 tcp server 的命令
pub type ReceiverTcpServer = Receiver<tcp_server::Command>;

/// 发送命令给 controller
pub type SenderController = Sender<RespClient>;

/// 接收 controller 的命令
pub type ReceiverController = Receiver<RespClient>;

/// 发送命令给 peer
pub type SenderPeer = Sender<peer::Command>;

/// 接收 peer 的命令
pub type ReceiverPeer = Receiver<peer::Command>;

/// 发送命令给 peer gasket
pub type SenderGasket = Sender<gasket::Command>;

/// 接收 peer gasket 的命令
pub type ReceiverGasket = Receiver<gasket::Command>;
