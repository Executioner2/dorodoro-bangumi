//! torrent 数据持久化

use doro_util::bytes_util;
use crate::db::ConnWrapper;
use crate::task_handler::gasket::PieceStatus;
use crate::torrent::TorrentArc;
use anyhow::{Error, Result, anyhow};
use bytes::BytesMut;
use dashmap::DashMap;
use rusqlite::params;
use std::mem;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::warn;

/// 种子状态
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TorrentStatus {
    /// 下载
    Download,

    /// 上传
    Upload,

    /// 暂停
    Pasue,

    /// 完成
    Finished,
}

impl TryFrom<u8> for TorrentStatus {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        if value <= 8 {
            Ok(unsafe { mem::transmute(value) })
        } else {
            Err(anyhow!("invalid torrent status: {}", value))
        }
    }
}

#[derive(Default)]
pub struct TorrentEntity {
    /// id
    pub id: Option<u64>,

    /// torrent 的 info hash
    pub info_hash: Option<Vec<u8>>,

    /// 序列化后的 torrent 实例
    pub serail: Option<TorrentArc>,

    /// 任务状态
    pub status: Option<TorrentStatus>,

    /// 下载量
    pub download: Option<u64>,

    /// 上传量
    pub uploaded: Option<u64>,

    /// 已完成的分块
    pub bytefield: Option<BytesMut>,

    /// 正在进行中的分块
    pub underway_bytefield: Option<Vec<(u32, PieceStatus)>>,

    /// 保存路径
    pub save_path: Option<PathBuf>,
}

pub trait TorrentMapper {
    /// 保存 bytefield
    ///  
    /// # Arguments
    ///
    /// * `bytefield`: 下载完成的分块信息
    /// * `info_hash`: torrent 的 info hash
    ///
    /// returns: ()
    fn update_bytefield(
        &self,
        bytefield: &[u8],
        ub: Arc<DashMap<u32, PieceStatus>>,
        info_hash: &[u8],
    ) -> Result<usize>;

    /// 从数据库中恢复状态
    ///
    /// # Arguments
    ///
    /// * `info_hash`: torrent 的 info hash
    ///
    /// returns: TorrentEntity
    fn recover_from_db(&self, info_hash: &[u8]) -> Result<TorrentEntity>;

    /// 保存任务进度
    ///
    /// # Arguments
    ///
    /// * `entity`: 种子视图
    ///
    /// returns: ()
    fn save_progress(&self, entity: TorrentEntity) -> Result<usize>;

    /// 添加下载任务
    ///
    /// # Arguments
    ///
    /// * `torrent`: 种子实例
    /// * `save_path`: 保存路径
    ///
    /// returns: bool `true`: 添加成功 `false`: 添加失败
    fn add_torrent(&mut self, torrent: &TorrentArc, save_path: &PathBuf) -> Result<bool>;

    /// 列出所有种子
    ///
    /// returns: Vec<TorrentArc> 种子列表
    fn list_torrent(&self) -> Result<Vec<TorrentEntity>>;
}

impl TorrentMapper for ConnWrapper {
    /// 保存 bytefield
    fn update_bytefield(
        &self,
        bytefield: &[u8],
        ub: Arc<DashMap<u32, PieceStatus>>,
        info_hash: &[u8],
    ) -> Result<usize> {
        let ub = ub
            .iter()
            .map(|item| (item.key().clone(), (*item.value()).clone()))
            .collect::<Vec<(u32, PieceStatus)>>();
        let mut stmt = self.prepare_cached(
            "update torrent set bytefield = ?1, underway_bytefield = ?2 where info_hash = ?3",
        )?;
        stmt.execute((bytefield, bytes_util::encode(&ub), info_hash))
            .map_err(Into::into)
    }

    /// 从数据库中恢复状态
    fn recover_from_db(&self, info_hash: &[u8]) -> Result<TorrentEntity> {
        let mut stmt = self.prepare_cached("select download, uploaded, save_path, bytefield, underway_bytefield, status from torrent where info_hash = ?1")?;
        stmt.query_row([&info_hash], |row| {
            let ub: Vec<(u32, PieceStatus)> = {
                let serial: Vec<u8> = row.get(4)?;
                bytes_util::decode(&serial)
            };
            Ok(TorrentEntity {
                download: Some(row.get(0)?),
                uploaded: Some(row.get(1)?),
                save_path: Some(PathBuf::from(row.get::<_, String>(2)?)),
                bytefield: Some(BytesMut::from(row.get::<_, Vec<u8>>(3)?.as_slice())),
                underway_bytefield: Some(ub),
                status: Some(TorrentStatus::try_from(row.get::<_, u8>(5)?).unwrap()),
                ..Default::default()
            })
        })
        .map_err(Into::into)
    }

    /// 保存当前进度
    fn save_progress(&self, entity: TorrentEntity) -> Result<usize> {
        let mut stmt = self.prepare_cached("update torrent set download = ?1, uploaded = ?2, bytefield = ?3, underway_bytefield = ?4, status = ?5 where info_hash = ?6")?;
        stmt.execute(params![
            entity.download,
            entity.uploaded,
            entity.bytefield.unwrap().as_ref(),
            bytes_util::encode(&entity.underway_bytefield.unwrap()),
            entity.status.map(|x| x as usize),
            entity.info_hash
        ])
        .map_err(Into::into)
    }

    /// 添加下载任务
    fn add_torrent(&mut self, torrent: &TorrentArc, save_path: &PathBuf) -> Result<bool> {
        let tx = self.transaction()?;

        let count: u32 = tx
            .prepare_cached("select count(*) from torrent where info_hash = ?1")?
            .query_row(params![torrent.info_hash], |row| row.get(0))?;
        if count > 0 {
            warn!("重复添加的 torrent");
            tx.commit()?;
            return Ok(false);
        }

        let serial = bytes_util::encode(&torrent.inner());
        let bytefield = vec![0u8; torrent.bitfield_len()];
        let underway_bytefield: Vec<(u32, PieceStatus)> = vec![];
        let underway_bytefield = bytes_util::encode(&underway_bytefield);
        tx.prepare_cached("insert into torrent(info_hash, serial, status, bytefield, underway_bytefield, save_path) values (?1, ?2, ?3, ?4, ?5, ?6)")?
            .execute(params![
            torrent.info_hash,
            &serial,
            TorrentStatus::Download as usize,
            bytefield,
            underway_bytefield,
            save_path.to_str(),
        ])?;

        tx.commit()?;

        Ok(true)
    }

    fn list_torrent(&self) -> Result<Vec<TorrentEntity>> {
        let mut stmt = self.prepare_cached("select serial, status from torrent")?;
        let mut rows = stmt.query([])?;
        let mut list = vec![];
        while let Some(row) = rows.next()? {
            let serial: Vec<u8> = row.get(0)?;
            let status = TorrentStatus::try_from(row.get::<_, u8>(1)?)?;
            let torrent = bytes_util::decode(serial.as_slice());
            list.push(TorrentEntity {
                serail: Some(TorrentArc::new(torrent)),
                status: Some(status),
                ..Default::default()
            });
        }
        Ok(list)
    }
}
