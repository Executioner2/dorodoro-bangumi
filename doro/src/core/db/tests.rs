use crate::core::db::Db;
use crate::torrent::{Parse, Torrent};
use anyhow::Result;
use bincode::config;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

static INIT_SQL: &str = r#"
CREATE TABLE "torrent" (
  "id" INTEGER NOT NULL,
  "info_hash" TEXT NOT NULL,
  "serial" blob,
  "status" INTEGER NOT NULL,
  "download" INTEGER NOT NULL DEFAULT 0,
  "uploaded" INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX "info_hash_idx"
ON "torrent" (
  "info_hash"
);

CREATE INDEX "status_idx"
ON "torrent" (
  "status"
);
"#;

/// 测试创建 db
#[tokio::test]
async fn test_create_db() -> Result<()> {
    let db = Db::new("db", "test.db", INIT_SQL, 10)?;
    let conn = db.get_conn().await?;
    let mut stmt = conn.prepare("select count(*) from torrent")?;
    let res: u32 = stmt.query_row([], |row| row.get(0))?;
    info!("res: {}", res);
    Ok(())
}

/// 测试插入一条 torrent 记录
#[tokio::test]
async fn test_insert_and_query_torrent() -> Result<()> {
    let db = Db::new("db", "test.db", INIT_SQL, 10)?;
    let conn = db.get_conn().await?;

    let torrent = Torrent::parse_torrent("./tests/resources/test6.torrent")?;
    let info_hash = hex::encode(torrent.info_hash);
    let serial = bincode::encode_to_vec(&torrent, config::standard())?;

    let mut stmt = conn.prepare_cached("replace into torrent(info_hash, serial, status, download, uploaded) values (?1, ?2, ?3, ?4, ?5)")?;
    stmt.execute((&info_hash, serial, 0, 0, 0))?;

    let mut stmt = conn.prepare_cached("select serial from torrent where info_hash = ?1")?;
    let serial: Vec<u8> = stmt.query_row([&info_hash], |row| row.get(0))?;
    let res: Torrent = bincode::decode_from_slice(&serial, config::standard())?.0;

    assert_eq!(res, torrent);

    Ok(())
}

/// 多线程下，获取链接
#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn test_get_conn() -> Result<()> {
    let db = Arc::new(Db::new("db", "test.db", INIT_SQL, 10)?);
    let mut handles = Vec::with_capacity(15);
    for i in 0..15 {
        let d = db.clone();
        handles.push(tokio::spawn(async move {
            let conn = d.get_conn().await.unwrap();
            info!("第 {i} 个 task 获得了链接");
            tokio::time::sleep(Duration::from_secs(3)).await;
            let mut stmt = conn.prepare_cached("select count(*) from torrent").unwrap();
            let res: u32 = stmt.query_row([], |row| row.get(0)).unwrap();
            info!("第 {i} 个 task 的查询结果: {res}");
        }))
    }

    for handle in handles {
        handle.await?;
    }

    match db.pool.lock() {
        Ok(pool) => {
            info!("当前连接池的长度: {}", pool.len());
            assert_eq!(pool.len(), 10);
        }
        Err(e) => {
            error!("当前连接池为空: {}", e);
        }
    }

    Ok(())
}
