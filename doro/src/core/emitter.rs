//! 命令（信号）发射器。统一管理所有运行时实例的 channel sender 端。

pub mod constant;
#[cfg(test)]
mod test;
pub mod transfer;

use anyhow::{Result, anyhow};
use dashmap::DashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc::Sender;
use transfer::TransferPtr;

#[derive(Clone)]
pub struct Emitter {
    mpsc_senders: Arc<DashMap<String, Sender<TransferPtr>>>,
}

unsafe impl Send for Emitter {}
unsafe impl Sync for Emitter {}

impl Emitter {
    pub fn global() -> &'static Self {
        static EMITTER: OnceLock<Emitter> = OnceLock::new();
        EMITTER.get_or_init(|| Self {
            mpsc_senders: Arc::new(DashMap::default()),
        })
    }

    pub async fn send(&self, transfer_id: &str, data: TransferPtr) -> Result<()> {
        if let Some(sender) = self.mpsc_senders.get(transfer_id) {
            match sender.send(data).await {
                Ok(_) => Ok(()),
                Err(e) => Err(anyhow!("Failed to send data: {}", e)),
            }
        } else {
            Err(anyhow!("Transfer id not found: {}", transfer_id))
        }
    }

    pub fn register<T: ToString>(&self, transfer_id: T, sender: Sender<TransferPtr>) {
        let transfer_id = transfer_id.to_string();
        self.mpsc_senders.insert(transfer_id, sender);
    }

    pub fn get(&self, transfer_id: &str) -> Option<Sender<TransferPtr>> {
        self.mpsc_senders
            .get(transfer_id)
            .map(|sender| sender.clone())
    }

    pub fn remove(&self, transfer_id: &str) -> Option<Sender<TransferPtr>> {
        self.mpsc_senders
            .remove(transfer_id)
            .map(|(_, value)| value)
    }
}
