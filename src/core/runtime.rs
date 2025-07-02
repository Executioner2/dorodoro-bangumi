use crate::config::CHANNEL_BUFFER;
use crate::emitter::transfer::TransferPtr;
use crate::emitter::Emitter;
use anyhow::{anyhow, Result};
use core::fmt::Debug;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::pin::Pin;
use tokio::sync::mpsc::{channel, Sender};
use tokio_util::sync::WaitForCancellationFuture;
use tracing::{debug, error, info, warn};

pub trait CustomExitReason: Debug {
    /// 退出代码
    fn exit_code(&self) -> u8;

    /// 退出信息
    fn exit_message(&self) -> String {
        String::new()
    }
}

/// 处理命令的结果
pub enum CommandHandleResult {
    /// 正常处理
    Continue,

    /// 退出执行
    Exit(ExitReason),
}

/// 自定义任务的结果
pub enum CustomTaskResult {
    /// 继续执行
    Continue(Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>>),

    /// 完成
    Finished,

    /// 退出执行
    Exit(ExitReason),
}

#[derive(Debug)]
pub enum ExitReason {
    /// 正常结束
    Normal,

    /// 错误退出
    Error(anyhow::Error),

    /// 自定义退出
    Custom(Box<dyn CustomExitReason + Send + 'static>),
}

/// 运行时上下文
pub struct RunContext {
    /// 发送命令的通道
    send: Sender<TransferPtr>,
}

impl RunContext {
    pub fn get_sender(&self) -> Sender<TransferPtr> {
        self.send.clone()
    }
}

/// 这个标记实现者是一个持续运行时
pub trait Runnable {
    /// 启动实例
    #[allow(async_fn_in_trait)]
    async fn run(mut self) where Self: Sized {
        // 注册命令通道
        let mut emitter = self.emitter().clone();
        let id = Self::get_transfer_id(self.get_suffix());
        info!("{id} 启动中...");
        let (send, mut recv) = {
            let (send, recv) = channel(CHANNEL_BUFFER);
            emitter.register(id.clone(), send.clone());
            (send, recv)
        };

        let exit_reason: ExitReason;
        let rc = RunContext { send };
        
        if let Err(e) = self.run_before_handle(rc).await {
            error!("{id} 启动失败: {:?}", e);
            exit_reason = ExitReason::Error(e);
            self.shutdown(exit_reason).await;
            return;
        }

        let mut futures = self.register_lt_future();

        loop {
            tokio::select! {
                _ = self.cancelled() => {
                    exit_reason = ExitReason::Normal;
                    break;
                }
                cmd = recv.recv() => {
                    match cmd {
                        Some(cmd) => {
                            match self.command_handle(cmd).await {
                                Ok(CommandHandleResult::Continue) => {}
                                Ok(CommandHandleResult::Exit(reason)) => {
                                    exit_reason = reason;
                                    break;
                                }
                                Err(e) => {
                                    exit_reason = ExitReason::Error(e);
                                    break;
                                }
                            }
                        }
                        None => {
                            exit_reason = ExitReason::Error(anyhow!("Command channel closed"));
                            break;
                        }
                    }
                }
                // 运行实现者自定义的异步任务
                lt_future = futures.next(), if !futures.is_empty() => {
                    match lt_future {
                        Some(CustomTaskResult::Continue(task)) => {
                            futures.push(task);
                        }
                        Some(CustomTaskResult::Finished) => {
                            // 什么都不需要做
                        }
                        Some(CustomTaskResult::Exit(reason)) => {
                            exit_reason = reason;
                            break;
                        }
                        None => {
                            warn!("lt_future 还没有准备好！");
                        }
                    }
                }
            }
        }

        // 移除命令通道，然后执行清理操作
        debug!("[{id}] - 退出原因: {:?}", exit_reason);
        self.shutdown(exit_reason).await;
        emitter.remove(&id);
        debug!("[{id}] - 已退出");
    }

    /// 获取 Emitter 实例
    fn emitter(&self) -> &Emitter;

    /// 获取 TransferId
    fn get_transfer_id<T: ToString>(suffix: T) -> String;

    /// 获取实例的后缀
    fn get_suffix(&self) -> String {
        String::new()
    }

    /// 注册长时间运行的异步任务
    fn register_lt_future(&mut self) -> FuturesUnordered<Pin<Box<dyn Future<Output = CustomTaskResult> + Send + 'static>>> {
        FuturesUnordered::default()
    }

    /// 运行前置准备
    #[allow(async_fn_in_trait)]
    async fn run_before_handle(&mut self, _rc: RunContext) -> Result<()> {
        Ok(())
    }

    /// 中止信号
    fn cancelled(&self) -> WaitForCancellationFuture<'_>;

    /// 持续运行实例
    #[allow(async_fn_in_trait)]
    async fn command_handle(&mut self, cmd: TransferPtr) -> Result<CommandHandleResult>;

    /// 结束运行
    #[allow(async_fn_in_trait)]
    async fn shutdown(&mut self, _reason: ExitReason) {}
}