use crate::command::CommandHandler;
use crate::peer_manager::gasket::Gasket;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum Command {
    DiscoverPeerAddr(DiscoverPeerAddr),
}

impl<'a> CommandHandler<'a> for Command {
    type Target = &'a Gasket;

    async fn handle(self, context: Self::Target) {
        match self {
            Command::DiscoverPeerAddr(cmd) => cmd.handle(context).await,
        }
    }
}

/// 发现了 peer addr
#[derive(Debug)]
pub struct DiscoverPeerAddr {
    pub peers: Vec<SocketAddr>,
}
impl<'a> CommandHandler<'a> for DiscoverPeerAddr {
    type Target = &'a Gasket;

    async fn handle(self, context: Self::Target) {
        for addr in self.peers {
            context.start_peer(addr).await    
        }
    }
}
