use super::error::Result;
use crate::core::db::Db;
use crate::torrent::{Parse, Torrent};
use bincode::config;

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
    let db = Db::new("db", "dorodoro-bangumi.db", INIT_SQL, 10)?;
    let conn = db.get_conn().await?;
    let mut stmt = conn.prepare("select count(*) from torrent")?;
    let res: u32 = stmt.query_row([], |row| row.get(0))?;
    println!("res: {}", res);
    Ok(())
}

/// 测试插入一条 torrent 记录
#[tokio::test]
async fn test_insert_and_query_torrent() -> Result<()> {
    let db = Db::new("db", "dorodoro-bangumi.db", INIT_SQL, 10)?;
    let conn = db.get_conn().await?;

    let torrent = Torrent::parse_torrent("./tests/resources/test6.torrent").unwrap();
    let info_hash = hex::encode(&torrent.info_hash);
    let serial = bincode::encode_to_vec(&torrent, config::standard()).unwrap();

    let mut stmt = conn.prepare_cached("replace into torrent(info_hash, serial, status, download, uploaded) values (?1, ?2, ?3, ?4, ?5)")?;
    stmt.execute((&info_hash, serial, 0, 0, 0))?;

    let mut stmt = conn.prepare_cached("select serial from torrent where info_hash = ?1")?;
    let serial: Vec<u8> = stmt.query_row([&info_hash], |row| row.get(0))?;
    let res: Torrent = bincode::decode_from_slice(&serial, config::standard())
        .unwrap()
        .0;

    assert_eq!(res, torrent);

    Ok(())
}
