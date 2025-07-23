//! 数据库存储

#[cfg(test)]
mod tests;

use rusqlite::Connection;
use std::collections::VecDeque;
use std::fs;
use std::fs::DirBuilder;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use anyhow::Result;
use doro_util::sync::MutexExt;

type Pool = Arc<Mutex<VecDeque<Connection>>>;

pub struct ConnWrapper {
    connection: Option<Connection>,
    pool: Pool,
    _permit: OwnedSemaphorePermit,
}

impl Deref for ConnWrapper {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.connection
            .as_ref()
            .expect("Connection always exists when in use")
    }
}

impl DerefMut for ConnWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.connection
            .as_mut()
            .expect("Connection always exists when in use")
    }
}

impl ConnWrapper {
    fn new(connection: Connection, pool: Pool, _permit: OwnedSemaphorePermit) -> Self {
        Self {
            connection: Some(connection),
            pool,
            _permit,
        }
    }
}

impl Drop for ConnWrapper {
    fn drop(&mut self) {
        // 归还连接到连接池
        let mut pool = self.pool.lock_pe();
        pool.push_back(self.connection.take().unwrap());
    }
}

pub struct Db {
    pool: Pool,
    filepath: Arc<PathBuf>,
    semaphore: Arc<Semaphore>,
}

impl Db {
    pub fn new(path: &str, db_name: &str, init_sql: &str, pool_limit: usize) -> Result<Self> {
        DirBuilder::new().recursive(true).create(path)?;
        let filepath = [path, db_name].iter().collect::<PathBuf>();
        Self::init(&filepath, init_sql)?;
        Ok(Self {
            pool: Arc::new(Mutex::new(VecDeque::with_capacity(pool_limit))),
            filepath: Arc::new(filepath),
            semaphore: Arc::new(Semaphore::new(pool_limit)),
        })
    }

    fn init(filepath: &PathBuf, init_sql: &str) -> Result<()> {
        if fs::exists(&filepath)? {
            return Ok(());
        }

        let conn = Connection::open(filepath)?;
        conn.execute_batch(init_sql)?;

        Ok(())
    }

    pub async fn get_conn(&self) -> Result<ConnWrapper> {
        let permit = self.semaphore.clone().acquire_owned().await?;
        let conn = match self.pool.lock_pe().pop_front() {
            None => Connection::open(&*self.filepath)?,
            Some(conn) => conn,
        };
        Ok(ConnWrapper::new(conn, self.pool.clone(), permit))
    }
}
