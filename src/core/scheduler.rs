//! 调度器
use crate::core::alias::{ReceiverScheduler, SenderPeerManager};
use crate::core::command::{CommandHandler, scheduler};
use crate::core::config::Config;
use crate::core::context::Context;
use crate::core::runtime::Runnable;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};

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
                    if let Some(cmd) = recv {
                        CommandHandler::handle(cmd, &mut self).await;
                    }
                }
            }
        }
        info!("scheduler 已关闭")
    }
}
