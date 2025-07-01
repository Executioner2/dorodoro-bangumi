use crate::emitter::Emitter;
use crate::emitter::transfer::TransferPtr;
use anyhow::Result;
use tokio::sync::mpsc::{channel, Receiver};
use crate::config::CHANNEL_BUFFER;

/// 命令处理器
pub trait CommandHandler<'a, Return = ()> {
    type Target;

    fn handle(self, ctx: Self::Target) -> impl Future<Output = Return> + Send;
}

/// 通信令牌    
/// 用于在不同的线程间传递信息
#[derive(Clone)]
pub struct ChannelToken {
    /// 消息接收方 id
    id: String,
    
    /// 发送器
    emitter: Emitter
}

impl ChannelToken {
    pub fn new(id: String, emitter: Emitter) -> Self {
        Self { id, emitter }
    }
    
    pub fn register<T: ToString>(id: T, mut emitter: Emitter) -> (Self, Receiver<TransferPtr>) {
        let (send, recv) = channel(CHANNEL_BUFFER);
        emitter.register(id.to_string(), send);
        (Self::new(id.to_string(), emitter), recv)
    }

    pub async fn send<T: Into<TransferPtr>>(&self, cmd: T) -> Result<()> {
        self.emitter.send(&self.id, cmd.into()).await
    }
    
    pub fn update_id(&mut self, id: String) {
        self.id = id;
    }
}