//! 调度器

use crate::core::alias::{ReceiverScheduler, SenderPeerManager, SenderScheduler};
use crate::core::command::CommandHandler;
use crate::core::config::Config;
use crate::core::context::Context;
use crate::core::runtime::Runnable;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};

/// 多线程下的共享数据
pub struct SchedulerContext {
    pub config: Config,
    pub spm: SenderPeerManager,
    pub ss: SenderScheduler,
    pub cancel_token: CancellationToken,
}

pub struct Scheduler {
    channel: (SenderScheduler, ReceiverScheduler),
    context: Context,
    cancel_token: CancellationToken,
    spm: SenderPeerManager,
    config: Config,
}

impl Scheduler {
    pub fn new(
        channel: (SenderScheduler, ReceiverScheduler),
        context: Context,
        cancel_token: CancellationToken,
        spm: SenderPeerManager,
        config: Config,
    ) -> Self {
        Self {
            channel,
            context,
            cancel_token,
            spm,
            config,
        }
    }

    pub fn get_context(&self) -> SchedulerContext {
        SchedulerContext {
            config: self.config.clone(),
            spm: self.spm.clone(),
            ss: self.channel.0.clone(),
            cancel_token: self.cancel_token.clone(),
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
                recv = self.channel.1.recv() => {
                    trace!("scheduler 收到命令: {:?}", recv);
                    if let Some(cmd) = recv {
                        CommandHandler::handle(cmd, self.get_context()).await;
                    }
                }
            }
        }
        info!("scheduler 已关闭")
    }
}
