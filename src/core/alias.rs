use tokio::sync::mpsc::{Receiver, Sender};
use crate::core::command::{peer_manager, scheduler, tcp_server};

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