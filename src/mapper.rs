pub mod torrent;
pub mod context;
pub mod dht;

/// db 保存路径
pub static DB_SAVE_PATH: &str = "db";

/// 数据库名字
pub static DB_NAME: &str = "dorodoro-bangumi.db";

/// 初始化 sql
pub static INIT_SQL: &str = r#"
    CREATE TABLE "torrent" (
      "id" INTEGER NOT NULL,
      "info_hash" blob NOT NULL,
      "serial" blob NOT NULL,
      "status" INTEGER NOT NULL,
      "download" INTEGER NOT NULL DEFAULT 0,
      "uploaded" INTEGER NOT NULL DEFAULT 0,
      "bytefield" blob NOT NULL,
      "underway_bytefield" blob NOT NULL,
      "save_path" text NOT NULL,
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
    
    CREATE TABLE "context" (
      "id" INTEGER NOT NULL,
      "config" blob NOT NULL,
      PRIMARY KEY ("id")
    );
    
    CREATE TABLE "dht" (
      "id" INTEGER NOT NULL,
      "own_id" blob NOT NULL,
      "routing_table" blob NOT NULL,
      "bootstrap_nodes" blob NOT NULL,
      PRIMARY KEY ("id")
    );
"#;