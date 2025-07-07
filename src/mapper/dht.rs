use lazy_static::lazy_static;
use rusqlite::OptionalExtension;
use crate::bytes_util;
use crate::db::ConnWrapper;
use crate::dht::routing::{NodeId, RoutingTable};

lazy_static! {
    pub static ref DEFAULT_BOOTSTRAP_NODES: Vec<String> = vec![
        "192.168.2.242:3115".to_string(),
        "dht.libtorrent.org:25401".to_string(),
        "dht.transmissionbt.com:6881".to_string(),
        "router.bittorrent.com:6881".to_string(),
        "router.utorrent.com:6881".to_string(),
        "dht.aelitis.com:6881".to_string(),
    ];    
}

#[derive(Default)]
pub struct DHTEntity {
    /// id
    pub id: Option<u64>,

    /// 自己的 node id
    pub own_id: Option<NodeId>,

    /// 路由表
    pub routing_table: Option<RoutingTable>,

    /// 用于 bootstrap 的节点
    pub bootstrap_nodes: Option<Vec<String>>,
}

impl DHTEntity {
    pub fn init() -> Self {
        let own_id = NodeId::random();
        Self {
            id: None,
            own_id: Some(own_id.clone()),
            routing_table: Some(RoutingTable::new(own_id)),
            bootstrap_nodes: Some(DEFAULT_BOOTSTRAP_NODES.clone()),
        }
    }
}

pub trait DHTMapper {
    /// 载入 DHT 实体
    ///
    /// # Returns
    ///
    /// * `DHTEntity` - 载入的 DHT 实体
    fn load_dht_entity(&self) -> Option<DHTEntity>;

    /// 保存 DHT 实体
    ///
    /// # Parameters
    ///
    /// * `dht_entity` - 要保存的 DHT 实体
    fn store_dht_entity(&self, dht_entity: DHTEntity);
    
    /// 更新路由表
    ///
    /// # Parameters
    ///
    /// * `routing_table` - 要更新的路由表
    fn update_routing_table(&self, routing_table: &RoutingTable);
}

impl DHTMapper for ConnWrapper {
    fn load_dht_entity(&self) -> Option<DHTEntity> {
        let mut stmt = self.prepare_cached("select id, own_id, routing_table, bootstrap_nodes from dht order by id desc limit 1").unwrap();
        stmt.query_one([], |row| {
            let own_id = {
                let serial = row.get::<_, Vec<u8>>(1)?;
                bytes_util::decode(serial.as_slice())
            };
            let routing_table = {
                let serial = row.get::<_, Vec<u8>>(2)?;
                bytes_util::decode(serial.as_slice())
            };
            let bootstrap_nodes: Vec<String> = {
                let serial = row.get::<_, Vec<u8>>(3)?;
                bytes_util::decode(serial.as_slice())
            };
            Ok(DHTEntity {
                id: row.get(0)?,
                own_id: Some(own_id),
                routing_table: Some(routing_table),
                bootstrap_nodes: Some(bootstrap_nodes),
            })
        }).optional().unwrap()
    }

    fn store_dht_entity(&self, dht_entity: DHTEntity) {
        let own_id = dht_entity.own_id.unwrap();
        let routing_table = dht_entity.routing_table.unwrap();
        let bootstrap_nodes = dht_entity.bootstrap_nodes.unwrap();
        let own_id = bytes_util::encode(&own_id);
        let routing_table = bytes_util::encode(&routing_table);
        let bootstrap_nodes = bytes_util::encode(&bootstrap_nodes);
        let mut stmt = self.prepare_cached("insert into dht (own_id, routing_table, bootstrap_nodes) values (?, ?, ?)").unwrap();
        stmt.execute([&own_id, &routing_table, &bootstrap_nodes]).unwrap();   
    }
    
    fn update_routing_table(&self, routing_table: &RoutingTable) {
        let routing_table = bytes_util::encode(routing_table);
        let mut stmt = self.prepare_cached(r#"
            update dht set routing_table = ? where id = (select max(id) from dht)
        "#).unwrap();
        stmt.execute([&routing_table]).unwrap();
    }
}
