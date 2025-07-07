use crate::config::Config;
use crate::db::ConnWrapper;
use rusqlite::OptionalExtension;
use crate::bytes_util;

#[derive(Default)]
pub struct ContextEntity {
    /// id
    pub id: Option<u64>,
    
    /// config
    pub config: Option<Config>
}

impl ContextEntity {
    pub fn init() -> Self {
        ContextEntity {
            id: None,
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
        let mut stmt = self.prepare_cached("select id, config from context order by id desc limit 1").unwrap();
        stmt.query_one([], |row| {
            let config = {
                let serial: Vec<u8> = row.get(1)?;
                bytes_util::decode(serial.as_slice())
            };
            Ok(ContextEntity {
                id: Some(row.get(0)?),
                config: Some(config)
            })
        }).optional().unwrap()
    }

    fn store_context(&self, entity: ContextEntity) {
        let config = entity.config.unwrap();
        let serial_config = bytes_util::encode(&config);
        let mut stmt = self.prepare_cached("insert into context (config) values (?)").unwrap();
        stmt.execute([&serial_config]).unwrap();
    }
}