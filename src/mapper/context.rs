use bincode::config;
use rusqlite::OptionalExtension;
use crate::config::Config;
use crate::db::ConnWrapper;
use crate::dht::node_id;
use crate::dht::node_id::NodeId;

#[derive(Default)]
pub struct ContextEntity {
    /// id
    pub id: Option<u64>,
    
    /// node_id
    pub node_id: Option<NodeId>,
    
    /// config
    pub config: Option<Config>
}

impl ContextEntity {
    pub fn init() -> Self {
        ContextEntity {
            id: None,
            node_id: Some(node_id::generate_node_id()),
            config: Some(Config::new())
        }
    }
}

pub trait ContextMapper {
    /// 载入全局上下文
    /// 
    /// # Returns
    /// 
    /// * `ContextEntity` - 全局上下文
    fn load_context(&self) -> Option<ContextEntity>;
    
    /// 存储全局上下文
    /// 
    /// # Arguments
    /// 
    /// * `context` - 全局上下文
    fn store_context(&self, entity: ContextEntity);
}

impl ContextMapper for ConnWrapper {
    fn load_context(&self) -> Option<ContextEntity> {
        let mut stmt = self.prepare_cached("select id, node_id, config from context order by id desc limit 1").unwrap();
        stmt.query_one([], |row| {
            let node_id = {
                let serial = row.get::<_, Vec<u8>>(1)?;
                bincode::decode_from_slice(serial.as_slice(), config::standard()).unwrap().0
            };
            let config = {
                let serial = row.get::<_, Vec<u8>>(2)?;
                bincode::decode_from_slice(serial.as_slice(), config::standard()).unwrap().0
            };
            Ok(ContextEntity {
                id: Some(row.get::<_, u64>(0)?),
                node_id: Some(node_id),
                config: Some(config)
            })
        }).optional().unwrap()
    }

    fn store_context(&self, entity: ContextEntity) {
        let mut stmt = self.prepare_cached("insert into context (node_id, config) values (?,?)").unwrap();
        let node_id = entity.node_id.unwrap();
        let config = entity.config.unwrap();
        let serial = bincode::encode_to_vec(&node_id, config::standard()).unwrap();
        let serial_config = bincode::encode_to_vec(&config, config::standard()).unwrap();
        stmt.execute([&serial, &serial_config]).unwrap();
    }
}