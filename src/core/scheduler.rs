//! 调度器

use crate::core::command;
use crate::core::context::Context;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};
use crate::core::config::Config;

/// 接收到 scheduler 的命令
type ReceiverScheduler = Receiver<command::scheduler::Command>;

/// 发送命令给 peer manager
type SenderPeer = Arc<Sender<command::peer::Command>>;

pub struct Scheduler {
    recv: ReceiverScheduler,
    context: Context,
    cancel_token: CancellationToken,
    peer_manager_send: SenderPeer,
    config: Config,
}

impl Scheduler {
    pub fn new(
        recv: ReceiverScheduler,
        context: Context,
        cancel_token: CancellationToken,
        peer_manager_send: SenderPeer,
        config: Config
    ) -> Self {
        Self {
            recv,
            context,
            cancel_token,
            peer_manager_send,
            config,
        }
    }

    pub async fn run(mut self) {
        info!("scheduler 已启动");
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    info!("scheduler 已停止");
                    break;
                }
                recv = self.recv.recv() => {
                    trace!("scheduler 收到命令: {:?}", recv);
                    if let Some(recv) = recv {
                        if !self.handle_command(recv) {
                            break;       
                        }
                    }
                }
            }
        }
    }

    fn handle_command(&mut self, cmd: command::scheduler::Command) -> bool {
        match cmd {

        }
    }
}
