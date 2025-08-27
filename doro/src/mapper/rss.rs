use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::db::ConnWrapper;

#[derive(Default)]
pub struct RSSEntity {
    /// id
    pub id: Option<u64>,

    /// title
    pub title: Option<String>,

    /// 订阅地址
    pub url: Option<String>,

    /// 订阅内容的 hash 值
    pub hash: Option<String>,

    /// 最后更新时间（单位秒时间戳）
    pub last_update: Option<u64>,

    /// 保存路径
    pub save_path: Option<String>,
}

#[derive(Default)]
pub struct RSSMarkReadEntity {
    /// id
    pub id: Option<u64>,

    /// rss id
    pub rss_id: Option<u64>,

    /// guid
    pub guid: Option<String>,
}

pub trait RSSMapper {
    /// 根据 url 获取 rss
    ///
    /// # Arguments
    ///
    /// * `url` - 订阅源的 url
    ///
    /// # Returns
    ///
    /// * `Option<RSSEntity>` - 订阅源的实体，如果没有找到，返回 None
    fn get_rss_by_url(&self, url: &str) -> Result<Option<RSSEntity>>;

    /// 添加订阅源
    ///
    /// # Arguments
    ///
    /// * `re` - RSSEntity 结构体
    /// * `guids` - 订阅源的 guid 列表
    ///
    /// # Returns
    ///
    /// * `bool` - 是否添加成功。一般都是成功，除非 url 已存在
    fn add_subscribe(&mut self, re: RSSEntity) -> Result<bool>;

    /// 列出近期未更新订阅源
    ///
    /// # Arguments
    ///
    /// * `limit` - 最后一次更新时间
    ///
    /// # Returns
    ///
    /// * `Vec<RSSEntity>` - 上一次更新时间小于 last_update 的订阅源列表
    fn list_subscribe_not_update(&self, last_update: u64) -> Result<Vec<RSSEntity>>;

    /// 列出指定 url 的所有已读的 guid
    ///
    /// # Arguments
    ///
    /// * `url` - 订阅源的 url
    ///
    /// # Returns
    ///
    /// * `Vec<String>` - 已读的 guid 列表
    fn list_mark_read_guid(&self, url: &str) -> Result<Vec<String>>;

    /// 更新订阅源
    ///
    /// # Arguments
    ///
    /// * `id` - 订阅源的 id
    /// * `last_update` - 最后一次更新时间
    /// * `hash` - 订阅源的 hash 值
    ///
    /// # Returns
    ///
    /// * `bool` - 是否更新成功
    fn update_subscribe(&self, id: u64, last_update: u64, hash: &str) -> Result<usize>;

    /// 标记已读
    ///
    /// # Arguments
    ///
    /// * `rss_id` - 订阅源的 id
    /// * `guid` - 订阅源的 guid
    ///
    /// # Returns
    ///
    /// * `bool` - 是否标记成功
    fn mark_read(&mut self, rss_id: u64, guid: &str) -> Result<bool>;

    /// 标记未读
    ///
    /// # Arguments
    ///
    /// * `rss_id` - 订阅源的 id
    /// * `guid` - 订阅源的 guid
    ///
    /// # Returns
    ///
    /// * `bool` - 是否标记成功
    fn mark_not_read(&mut self, rss_id: u64, guid: &str) -> Result<usize>;
}

impl RSSMapper for ConnWrapper {
    fn get_rss_by_url(&self, url: &str) -> Result<Option<RSSEntity>> {
        let mut stmt = self.prepare_cached(
            "select id, title, url, hash, last_update, save_path from rss where url = ? limit 1",
        )?;
        stmt.query_row(params![url], |row| {
            Ok(RSSEntity {
                id: row.get(0)?,
                title: row.get(1)?,
                url: row.get(2)?,
                hash: row.get(3)?,
                last_update: row.get(4)?,
                save_path: row.get(5)?,
            })
        })
        .optional()
        .map_err(Into::into)
    }

    fn add_subscribe(&mut self, re: RSSEntity) -> Result<bool> {
        let title = re.title.unwrap();
        let url = re.url.unwrap();
        let hash = re.hash.unwrap();
        let last_update = re.last_update.unwrap();
        let save_path = re.save_path;

        let tx = self.transaction()?;

        // 先查询有没有这个 url
        let count: i64 = tx
            .prepare_cached("select count(*) from rss where url = ?")?
            .query_row([&url], |row| row.get(0))?;

        if count > 0 {
            tx.commit()?;
            return Ok(false);
        }

        // 插入订阅源
        tx.prepare_cached("insert into rss (title, url, hash, last_update, save_path) values (?, ?, ?, ?, ?)")?
            .execute(params![&title, &url, &hash, &last_update, &save_path])?;

        tx.commit()?;
        Ok(true)
    }

    fn list_subscribe_not_update(&self, last_update: u64) -> Result<Vec<RSSEntity>> {
        let mut stmt = self.prepare_cached(
            "select id, title, url, hash, last_update, save_path from rss where last_update < ?",
        )?;
        let mut rows = stmt.query(params![&last_update])?;
        let mut result = Vec::new();

        while let Some(row) = rows.next()? {
            result.push(RSSEntity {
                id: row.get(0)?,
                title: row.get(1)?,
                url: row.get(2)?,
                hash: row.get(3)?,
                last_update: row.get(4)?,
                save_path: row.get(5)?,
            });
        }

        Ok(result)
    }

    fn list_mark_read_guid(&self, url: &str) -> Result<Vec<String>> {
        let mut stmt = self.prepare_cached(
            r#"
            select guid from rss_mark_read where rss_id = (select id from rss where url = ? limit 1)
        "#,
        )?;
        let mut res = Vec::new();
        let mut rows = stmt.query(params![url])?;
        while let Some(row) = rows.next()? {
            res.push(row.get(0)?)
        }
        Ok(res)
    }

    fn update_subscribe(&self, id: u64, last_update: u64, hash: &str) -> Result<usize> {
        let mut stmt = self.prepare_cached(
            r#"
            update rss set last_update = ?, hash = ? where id = ?
        "#,
        )?;
        stmt.execute(params![&last_update, &hash, &id])
            .map_err(Into::into)
    }

    fn mark_read(&mut self, rss_id: u64, guid: &str) -> Result<bool> {
        let tx = self.transaction()?;

        // 检查是否已经标记了未读
        let count: i64 = tx
            .prepare_cached("select count(*) from rss_mark_read where rss_id = ? and guid = ?")?
            .query_row(params![&rss_id, &guid], |row| row.get(0))?;

        if count > 0 {
            tx.commit()?;
            return Ok(false);
        }

        // 插入标记
        tx.prepare_cached("insert into rss_mark_read (rss_id, guid) values (?, ?)")?
            .execute(params![&rss_id, &guid])?;

        tx.commit()?;
        Ok(true)
    }

    fn mark_not_read(&mut self, rss_id: u64, guid: &str) -> Result<usize> {
        let mut stmt =
            self.prepare_cached("delete from rss_mark_read where rss_id = ? and guid = ?")?;
        stmt.execute(params![&rss_id, &guid]).map_err(Into::into)
    }
}
