//! 调度器

use crate::core::command::CommandHandler;
use crate::core::config::Config;
use crate::core::context::Context;
use crate::core::emitter::Emitter;
use crate::core::emitter::constant::SCHEDULER;
use crate::core::runtime::Runnable;
use command::Command;
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, trace};

pub mod command;
mod error;

/// 多线程下的共享数据
#[allow(dead_code)]
pub struct SchedulerContext {
    config: Config,
    cancel_token: CancellationToken,
    emitter: Emitter,
}

pub struct Scheduler {
    #[allow(dead_code)]
    context: Context,
    cancel_token: CancellationToken,
    config: Config,
    emitter: Emitter,
}

impl Scheduler {
    pub fn new(
        context: Context,
        cancel_token: CancellationToken,
        config: Config,
        emitter: Emitter,
    ) -> Self {
        Self {
            context,
            cancel_token,
            config,
            emitter,
        }
    }

    pub fn get_context(&self) -> SchedulerContext {
        SchedulerContext {
            config: self.config.clone(),
            cancel_token: self.cancel_token.clone(),
            emitter: self.emitter.clone(),
        }
    }

    fn shutdown(self) {
        self.emitter.remove(SCHEDULER);
    }
}

impl Runnable for Scheduler {
    async fn run(mut self) {
        let (send, mut recv) = channel(self.config.channel_buffer());
        self.emitter.register(SCHEDULER, send);

        info!("scheduler 已启动");
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    trace!("scheduler 收到关闭信号");
                    break;
                }
                recv = recv.recv() => {
                    if let Some(cmd) = recv {
                        let cmd: Command = cmd.instance();
                        trace!("scheduler 收到命令: {:?}", cmd);
                        if let Err(e) = cmd.handle(&mut self).await {
                            error!("处理指令出现错误\t{}", e);
                            break;
                        }
                    }
                }
            }
        }

        self.shutdown();
        info!("scheduler 已关闭")
    }
}
