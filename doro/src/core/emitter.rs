//! 命令（信号）发射器。统一管理所有运行时实例的 channel sender 端。

pub mod constant;
#[cfg(test)]
mod test;
pub mod transfer;

use dashmap::DashMap;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use transfer::TransferPtr;

#[derive(Eq, PartialEq, Hash)]
pub enum EmitterType {
    Scheduler,
    PeerManager,
}

#[derive(Clone)]
pub struct Emitter {
    mpsc_senders: Arc<DashMap<String, Sender<TransferPtr>>>,
}

unsafe impl Send for Emitter {}
unsafe impl Sync for Emitter {}

impl Emitter {
    pub fn new() -> Self {
        Self {
            mpsc_senders: Arc::new(DashMap::default()),
        }
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

    pub fn register<T: ToString>(&mut self, transfer_id: T, sender: Sender<TransferPtr>) {
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
