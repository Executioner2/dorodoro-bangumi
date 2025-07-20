//! 调度器

use crate::core::command::CommandHandler;
use crate::core::context::Context;
use crate::core::emitter::constant::SCHEDULER;
use crate::core::emitter::Emitter;
use crate::core::runtime::Runnable;
use crate::emitter::transfer::TransferPtr;
use crate::runtime::CommandHandleResult;
use anyhow::Result;
use command::Command;
use tokio_util::sync::WaitForCancellationFuture;

pub mod command;

/// 多线程下的共享数据
pub struct SchedulerContext {
    emitter: Emitter,
    context: Context,
}

pub struct Scheduler {
    context: Context,
    emitter: Emitter,
}

impl Scheduler {
    pub fn new(context: Context, emitter: Emitter) -> Self {
        Self { context, emitter }
    }

    pub fn get_context(&self) -> SchedulerContext {
        SchedulerContext {
            emitter: self.emitter.clone(),
            context: self.context.clone()
        }
    }
}

impl Runnable for Scheduler {
    fn emitter(&self) -> &Emitter {
        &self.emitter
    }

    fn get_transfer_id<T: ToString>(_suffix: T) -> String {
        SCHEDULER.to_string()
    }

    fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.context.cancelled()
    }

    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult> {
        let cmd: Command = cmd.instance();
        cmd.handle(self).await?;
        Ok(CommandHandleResult::Continue)
    }
}
