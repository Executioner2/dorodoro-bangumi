use bincode::config;
use dashmap::DashMap;
use dorodoro_bangumi::peer_manager::gasket::PieceStatus;
use rusqlite::Connection;
use tracing::{info, Level};
use dorodoro_bangumi::db::Db;
use dorodoro_bangumi::{default_logger, mapper};
use dorodoro_bangumi::config::DATABASE_CONN_LIMIT;
use dorodoro_bangumi::mapper::dht::DHTMapper;

default_logger!(Level::DEBUG);

#[test]
fn test_db() {
    let filepath = "db/dorodoro-bangumi.db";
    let conn = Connection::open(filepath).unwrap();
    let mut stmt = conn.prepare_cached("select underway_bytefield from torrent where id = 1").unwrap();
    let ub = stmt.query_row([], |row| {
        let ub: Vec<(u32, PieceStatus)> = bincode::decode_from_slice(
            row.get::<_, Vec<u8>>(0)?.as_slice(),
            config::standard(),
        ).unwrap().0;
        Ok(ub)
    }).unwrap();

    let ub: DashMap<u32, PieceStatus> = ub.into_iter().collect();
    info!("ub: {:?}", ub);
}

#[tokio::test]
async fn test_load_dht() {
    let db = Db::new(mapper::DB_SAVE_PATH, mapper::DB_NAME, mapper::INIT_SQL, DATABASE_CONN_LIMIT).unwrap();
    let conn = db.get_conn().await.unwrap();
    let de = conn.load_dht_entity().unwrap().unwrap();
    info!("{:?}", de.routing_table);
}