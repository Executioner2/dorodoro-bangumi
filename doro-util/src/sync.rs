use std::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;
use tracing::{error, warn};

pub trait MutexExt<T> {
    fn lock_pe(&self) -> MutexGuard<T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_pe(&self) -> MutexGuard<T> {
        match self.lock() {
            Ok(mg) => mg,
            Err(pe) => {
                warn!("Mutex poisoned: {}", pe);
                pe.into_inner()
            }
        }
    }
}

/// 发出中断信号并等待所有 JoinHandle 完成
/// 一般用于退出后清理子线程
pub async fn wait_join_handles_close(handles: impl Iterator<Item = &mut JoinHandle<()>>) {
    for handle in handles {
        wait_join_handle_close(handle).await;
    }
}

/// 发出中断信号并等待 JoinHandle 完成
/// 一般用于退出后清理子线程
pub async fn wait_join_handle_close(handle: &mut JoinHandle<()>) {
    handle.abort();
    if let Err(e) = handle.await {
        if e.is_panic() {
            error!("JoinHandle panicked: {}", e);
        }
    }
}