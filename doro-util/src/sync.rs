use std::sync::{Mutex, MutexGuard};
use tracing::warn;

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