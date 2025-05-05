use crate::core::config::Config;
use crate::core::context::Context;
use crate::core::emitter::Emitter;
use crate::core::peer_manager::PeerManager;
use crate::core::runtime::Runnable;
use crate::core::scheduler::Scheduler;
use crate::core::tcp_server::TcpServer;
use tracing::{info, trace};
use crate::db::Db;

static INIT_SQL: &str = r#"
"#;

pub struct Bootstrap {}

impl Bootstrap {
    pub async fn start() {
        info!("dorodoro-bangumi 启动中...");

        // 初始化通用资源
        trace!("初始化全局上下文");
        let config = Config::new();
        let db = Db::new("db", "dorodoro-bangumi.db", INIT_SQL, config.database_conn_limit()).unwrap();
        let context = Context::new(db, config);

        // 命令发射器
        let emitter = Emitter::new();

        trace!("启动 tcp server");
        let tcp_server = TcpServer::new(context.clone(), emitter.clone());
        let tcp_server_handle = tokio::spawn(tcp_server.run());

        trace!("启动 peer 管理器");
        let peer_manager = PeerManager::new(context.clone(), emitter.clone());
        let peer_manager_handle = tokio::spawn(peer_manager.run());

        trace!("启动调度器");
        let scheduler = Scheduler::new(context, emitter);
        scheduler.run().await;

        info!("等待资源关闭中...");
        peer_manager_handle.await.unwrap();
        tcp_server_handle.await.unwrap();

        info!("资源已安全关闭，程序退出");
    }
}
