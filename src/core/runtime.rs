use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

/// 这个标记实现者是一个持续运行时
pub trait Runnable {
    fn run(self) -> impl Future<Output = ()> + Send;
}

#[derive(Debug)]
enum DelayedTaskState {
    Wait(Pin<Box<tokio::time::Sleep>>),
    Ready,
    Finished,
}

pub struct DelayedTask<F> {
    task: F,
    state: DelayedTaskState,
    cancel: Arc<AtomicBool>,
}

impl<U, F> DelayedTask<F>
where
    U: Default + Send + 'static,
    F: Future<Output = U> + Send + 'static,
{
    pub fn new(delay: Duration, task: F) -> Self {
        Self {
            task,
            state: DelayedTaskState::Wait(Box::pin(tokio::time::sleep(delay))),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 取消掉任务
    pub fn cancel(self) {
        self.cancel.fetch_and(true, Ordering::Relaxed);
    }
}

impl<U, F> Future for DelayedTask<F>
where
    F: Future<Output = U> + Send + 'static,
{
    type Output = Option<U>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        while !this.cancel.load(Ordering::Relaxed) {
            match &mut this.state {
                DelayedTaskState::Wait(sleep) => match Pin::new(sleep).poll(cx) {
                    Poll::Ready(_) => {
                        this.state = DelayedTaskState::Ready;
                    }
                    Poll::Pending => return Poll::Pending,
                },
                DelayedTaskState::Ready => {
                    return match unsafe { Pin::new_unchecked(&mut this.task).poll(cx) } {
                        Poll::Ready(u) => {
                            this.state = DelayedTaskState::Finished;
                            Poll::Ready(Some(u))
                        }
                        Poll::Pending => Poll::Pending,
                    };
                }
                DelayedTaskState::Finished => return Poll::Ready(None),
            }
        }
        Poll::Ready(None)
    }
}
