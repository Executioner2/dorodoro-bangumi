#[cfg(test)]
mod tests;

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
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

#[derive(Clone)]
pub struct CancelToken {
    cancel: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl CancelToken {
    fn new(waker: Option<Waker>) -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            waker: Arc::new(Mutex::new(waker)),
        }
    }

    fn update_waker(&self, waker: Waker) {
        match self.waker.lock() {
            Ok(mut mutex) => {
                let _ = mutex.insert(waker);
            }
            Err(mut e) => {
                let _ = e.get_mut().insert(waker);
            }
        }
    }
    
    fn is_cancel(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    
    pub fn cancel(self) {
        self.cancel.store(true, Ordering::Relaxed);
        match self.waker.lock() {
            Ok(mut mutex) => {
                mutex.take().map(|waker| waker.wake());
            }
            Err(mut e) => {
                e.get_mut().take().map(|waker| waker.wake());
            }
        }
    }
}

pub struct DelayedTask<F> {
    task: F,
    state: DelayedTaskState,
    cancel: CancelToken,
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
            cancel: CancelToken::new(None),
        }
    }
    
    /// 获取取消 token
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }
}

impl<U, F> Future for DelayedTask<F>
where
    F: Future<Output = U> + Send + 'static,
{
    type Output = Option<U>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        while !this.cancel.is_cancel() {
            this.cancel.update_waker(cx.waker().clone());
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
