use crate::config::DATABASE_CONN_LIMIT;
use crate::core::config::Config;
use crate::core::context::Context;
use crate::core::emitter::Emitter;
use crate::core::peer_manager::PeerManager;
use crate::core::runtime::Runnable;
use crate::core::scheduler::Scheduler;
use crate::core::tcp_server::TcpServer;
use crate::core::udp_server::UdpServer;
use crate::db::Db;
use crate::mapper;
use tracing::{info, trace};
use crate::dht::DHT;
use crate::dht::routing::{NodeId, RoutingTable};

pub async fn start() {
    info!("dorodoro-bangumi 启动中...");

    // 初始化通用资源
    trace!("初始化全局上下文");
    let db = Db::new(mapper::DB_SAVE_PATH, mapper::DB_NAME, mapper::INIT_SQL, DATABASE_CONN_LIMIT).unwrap();
    let context = load_context(db).await;

    // 命令发射器
    let emitter = Emitter::new();

    trace!("启动 tcp server");
    let tcp_server = TcpServer::new(context.clone(), emitter.clone());
    let tcp_server_handle = tokio::spawn(tcp_server.run());

    trace!("启动 udp server");
    let udp_server = UdpServer::new(context.clone()).await.unwrap();
    let udp_server_handle = tokio::spawn(udp_server.clone().run());
    
    trace!("启动 dht");
    let (routing_table, bootstrap_nodes) = load_routing_table(&context).await;
    let dht_server = DHT::new(emitter.clone(), context.clone(), udp_server, routing_table, bootstrap_nodes);
    let dht_server_handle = tokio::spawn(dht_server.run());

    trace!("启动 peer 管理器");
    let peer_manager = PeerManager::new(context.clone(), emitter.clone());
    let peer_manager_handle = tokio::spawn(peer_manager.run());

    trace!("启动调度器");
    let scheduler = Scheduler::new(context, emitter);
    scheduler.run().await;

    info!("等待资源关闭中...");
    peer_manager_handle.await.unwrap();
    dht_server_handle.await.unwrap();
    udp_server_handle.await.unwrap();
    tcp_server_handle.await.unwrap();

    info!("资源已安全关闭，程序退出");
}

pub async fn load_context(db: Db) -> Context {
    use crate::mapper::context::{ContextEntity, ContextMapper};
    let conn = db.get_conn().await.unwrap();
    let cen = conn.load_context();
    let not_init = cen.is_none();
    
    let ce = cen.unwrap_or(ContextEntity::init());
    let config = ce.config.unwrap_or(Config::new());
    
    if not_init {
        let ce = ContextEntity {
            config: Some(config.clone()),
            ..Default::default()
        };
        conn.store_context(ce);  
    }

    Context::new(db, config)
}

pub async fn load_routing_table(context: &Context) -> (RoutingTable, Vec<String>) {
    use crate::mapper::dht::{DHTEntity, DHTMapper, DEFAULT_BOOTSTRAP_NODES};
    let conn = context.get_conn().await.unwrap();
    let dhte = conn.load_dht_entity();
    let not_init = dhte.is_none();
    
    let dhte = dhte.unwrap_or(DHTEntity::init());
    let own_id = dhte.own_id.unwrap_or(NodeId::random());
    let routing_table = dhte.routing_table.unwrap_or(RoutingTable::new(own_id.clone()));
    let bootstrap_nodes = dhte.bootstrap_nodes.unwrap_or(DEFAULT_BOOTSTRAP_NODES.clone());
    
    if not_init {
        let dhte = DHTEntity {
            own_id: Some(own_id.clone()),
            routing_table: Some(routing_table.clone()),
            bootstrap_nodes: Some(bootstrap_nodes.clone()),
            ..Default::default()
        };
        conn.store_dht_entity(dhte);
    }

    (routing_table, bootstrap_nodes)
}
