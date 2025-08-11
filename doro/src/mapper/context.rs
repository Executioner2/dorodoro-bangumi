use anyhow::Result;
use doro_util::bytes_util;
use rusqlite::OptionalExtension;

use crate::config::Config;
use crate::db::ConnWrapper;

#[derive(Default)]
pub struct ContextEntity {
    /// id
    pub id: Option<u64>,

    /// config
    pub config: Option<Config>,
}

impl ContextEntity {
    pub fn init() -> Self {
        ContextEntity {
            id: None,
            config: Some(Config::new()),
        }
    }
}

pub trait ContextMapper {
    /// 载入全局上下文
    ///
    /// # Returns
    ///
    /// * `ContextEntity` - 全局上下文
    fn load_context(&self) -> Result<Option<ContextEntity>>;

    /// 存储全局上下文
    ///
    /// # Arguments
    ///
    /// * `context` - 全局上下文
    fn store_context(&self, entity: ContextEntity) -> Result<usize>;
}

impl ContextMapper for ConnWrapper {
    fn load_context(&self) -> Result<Option<ContextEntity>> {
        let mut stmt =
            self.prepare_cached("select id, config from context order by id desc limit 1")?;
        stmt.query_one([], |row| {
            let config = {
                let serial: Vec<u8> = row.get(1)?;
                bytes_util::decode(serial.as_slice())
            };
            Ok(ContextEntity {
                id: Some(row.get(0)?),
                config: Some(config),
            })
        })
        .optional()
        .map_err(Into::into)
    }

    fn store_context(&self, entity: ContextEntity) -> Result<usize> {
        let config = entity.config.unwrap();
        let serial_config = bytes_util::encode(&config);
        let mut stmt = self.prepare_cached("insert into context (config) values (?)")?;
        stmt.execute([&serial_config]).map_err(Into::into)
    }
}
