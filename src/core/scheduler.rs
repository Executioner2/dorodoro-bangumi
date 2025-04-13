//! 调度器
use crate::core::command::{CommandHandler, peer_manager, scheduler};
use crate::core::config::Config;
use crate::core::context::Context;
use crate::core::runtime::Runnable;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};

/// 接收到 scheduler 的命令
type ReceiverScheduler = Receiver<scheduler::Command>;

/// 发送命令给 peer manager
type SenderPeerManager = Sender<peer_manager::Command>;

pub struct Scheduler {
    recv: ReceiverScheduler,
    context: Context,
    cancel_token: CancellationToken,
    spm: SenderPeerManager,
    config: Config,
}

impl Scheduler {
    pub fn new(
        recv: ReceiverScheduler,
        context: Context,
        cancel_token: CancellationToken,
        spm: SenderPeerManager,
        config: Config,
    ) -> Self {
        Self {
            recv,
            context,
            cancel_token,
            spm,
            config,
        }
    }

    pub async fn handle_command(&mut self, cmd: scheduler::Command) {
        CommandHandler::handle(cmd, self).await;
    }

    pub fn shutdown(&mut self) {
        self.cancel_token.cancel();
    }
}

impl Runnable for Scheduler {
    async fn run(mut self) {
        info!("scheduler 已启动");
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    trace!("scheduler 收到关闭信号");
                    break;
                }
                recv = self.recv.recv() => {
                    trace!("scheduler 收到命令: {:?}", recv);
                    if let Some(recv) = recv {
                        self.handle_command(recv).await;
                    }
                }
            }
        }
        info!("scheduler 已关闭")
    }
}
