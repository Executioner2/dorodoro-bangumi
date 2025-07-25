use crate::config::DATABASE_CONN_LIMIT;
use crate::core::config::Config;
use crate::core::context::Context;
use crate::core::runtime::Runnable;
use crate::core::task_handler::TaskHandler;
use crate::core::tcp_server::TcpServer;
use crate::core::udp_server::UdpServer;
use crate::db::Db;
use crate::dht::DHT;
use crate::dht::routing::{NodeId, RoutingTable};
use crate::mapper;
use tracing::{info, trace};

#[rustfmt::skip]
pub async fn start() {
    info!("dorodoro-bangumi 启动中...");

    // 初始化通用资源
    trace!("初始化全局上下文");
    let db = Db::new(mapper::DB_SAVE_PATH, mapper::DB_NAME, mapper::INIT_SQL, DATABASE_CONN_LIMIT).unwrap();
    let context = load_context(db).await;

    trace!("启动 tcp server");
    let tcp_server = TcpServer::new();
    let tcp_server_handle = tokio::spawn(Box::pin(tcp_server.run()));

    trace!("启动 udp server");
    let udp_server = UdpServer::new().await.unwrap();
    let udp_server_handle = tokio::spawn(Box::pin(udp_server.clone().run()));

    trace!("启动 dht");
    let (routing_table, bootstrap_nodes) = load_routing_table(&context).await;
    let dht_server = DHT::new(udp_server, routing_table, bootstrap_nodes);
    let dht_server_handle = tokio::spawn(Box::pin(dht_server.run()));

    trace!("初始化任务处理器");
    TaskHandler::init().await;

    info!("dorodoro bangumi 运行中...");
    dht_server_handle.await.unwrap();
    udp_server_handle.await.unwrap();
    tcp_server_handle.await.unwrap();

    info!("资源已安全关闭，程序退出");
}

pub async fn load_context(db: Db) -> Context {
    use crate::mapper::context::{ContextEntity, ContextMapper};
    let conn = db.get_conn().await.unwrap();
    let cen = conn.load_context().unwrap();
    let not_init = cen.is_none();

    let ce = cen.unwrap_or(ContextEntity::init());
    let config = ce.config.unwrap_or(Config::new());

    if not_init {
        let ce = ContextEntity {
            config: Some(config.clone()),
            ..Default::default()
        };
        conn.store_context(ce).unwrap();
    }

    Context::init(db, config);
    Context::global().clone()
}

pub async fn load_routing_table(context: &Context) -> (RoutingTable, Vec<String>) {
    use crate::mapper::dht::{DEFAULT_BOOTSTRAP_NODES, DHTEntity, DHTMapper};
    let conn = context.get_conn().await.unwrap();
    let dhte = conn.load_dht_entity().unwrap();
    let not_init = dhte.is_none();

    let dhte = dhte.unwrap_or(DHTEntity::init());
    let own_id = dhte.own_id.unwrap_or(NodeId::random());
    let routing_table = dhte
        .routing_table
        .unwrap_or(RoutingTable::new(own_id.clone()));
    let bootstrap_nodes = dhte
        .bootstrap_nodes
        .unwrap_or(DEFAULT_BOOTSTRAP_NODES.clone());

    if not_init {
        let dhte = DHTEntity {
            own_id: Some(own_id.clone()),
            routing_table: Some(routing_table.clone()),
            bootstrap_nodes: Some(bootstrap_nodes.clone()),
            ..Default::default()
        };
        conn.store_dht_entity(dhte).unwrap();
    }

    (routing_table, bootstrap_nodes)
}
