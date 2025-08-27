use anyhow::Result;

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
        let mut rows = stmt.query_and_then([], |row| {
            let serial: String = row.get(1)?;
            let x = serde_json::from_str(&serial)?;
            let config = Config::from_inner(x);
            Ok(ContextEntity {
                id: Some(row.get(0)?),
                config: Some(config),
            })
        })?;

        rows.next().transpose()
    }

    fn store_context(&self, entity: ContextEntity) -> Result<usize> {
        let config = entity.config.unwrap();
        let config_json = serde_json::to_string(config.inner())?;
        let mut stmt = self.prepare_cached("insert into context (config) values (?)")?;
        stmt.execute([&config_json]).map_err(Into::into)
    }
}
