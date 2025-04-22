//! 命令（信号）发射器。统一管理所有运行时实例的 channel sender 端。

pub mod constant;
mod error;
#[cfg(test)]
mod test;
pub mod transfer;

use crate::core::emitter::error::Error;
use crate::core::emitter::error::Error::RepeatRegister;
use ahash::RandomState;
use error::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc::Sender;
use transfer::TransferPtr;

#[derive(Eq, PartialEq, Hash)]
pub enum EmitterType {
    Scheduler,
    PeerManager,
}

pub struct Emitter {
    mpsc_senders: Arc<RwLock<HashMap<String, Sender<TransferPtr>, RandomState>>>,
}

unsafe impl Send for Emitter {}
unsafe impl Sync for Emitter {}

impl Clone for Emitter {
    fn clone(&self) -> Self {
        Self {
            mpsc_senders: self.mpsc_senders.clone(),
        }
    }
}

impl Emitter {
    pub fn new() -> Self {
        Self {
            mpsc_senders: Arc::new(RwLock::new(HashMap::default())),
        }
    }

    pub async fn send(&self, transfer_id: &str, data: TransferPtr) -> Result<()> {
        if let Some(sender) = self.mpsc_senders.read().await.get(transfer_id) {
            match sender.send(data).await {
                Ok(_) => Ok(()),
                Err(e) => Err(Error::SendError(e)),
            }
        } else {
            Err(Error::NotFindEmitterType)
        }
    }

    pub async fn register<T: ToString>(
        &mut self,
        transfer_id: T,
        sender: Sender<TransferPtr>,
    ) -> Result<()> {
        let transfer_id = transfer_id.to_string();
        let mut lock = self.mpsc_senders.write().await;
        if lock.contains_key(&transfer_id) {
            return Err(RepeatRegister);
        }
        lock.insert(transfer_id, sender);
        Ok(())
    }

    pub async fn get(&self, transfer_id: &str) -> Option<Sender<TransferPtr>> {
        self.mpsc_senders
            .read()
            .await
            .get(transfer_id)
            .map(|sender| sender.clone())
    }
    
    pub async fn remove(&self, transfer_id: &str) -> Option<Sender<TransferPtr>> {
        self.mpsc_senders.write().await.remove(transfer_id)
    }
}
