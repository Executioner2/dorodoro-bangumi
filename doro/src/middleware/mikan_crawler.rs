use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use ctor::ctor;
use doro_util::hashmap;
use doro_util::xml::{self, AttributeExt, AttributesExt};
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::Serialize;

use crate::entity::{Group, Quarter, Resource, Source};
use crate::{BangumiCrawler, Crawler};

#[cfg(test)]
mod tests;

const BASE_URL: &str = "https://mikanani.me";

const QUARTERS: [&str; 4] = ["秋", "夏", "春", "冬"];

#[derive(Default, Debug)]
struct ResourceBuilder {
    id: Option<String>,
    name: Option<String>,
    link: Option<String>,
    last_update: Option<DateTime<Utc>>,
    image_url: Option<String>,
}

impl ResourceBuilder {
    fn build(self) -> Option<Resource> {
        Some(Resource {
            id: self.id?,
            name: self.name?,
            link: self.link?,
            last_update: self.last_update,
            image_url: self.image_url?,
            extend: HashMap::default(),
        })
    }
}

#[derive(Default, Debug)]
struct GroupBuilder {
    id: Option<String>,
    name: Option<String>,
}

impl GroupBuilder {
    fn build(self) -> Option<Group> {
        Some(Group {
            id: self.id?,
            name: self.name?,
        })
    }
}

#[derive(Default, Debug)]
struct SourceBuilder {
    group: Option<Group>,
    name: Option<String>,
    update_date: Option<DateTime<Utc>>,
    torrent_url: Option<String>,
    magnet_url: Option<String>,
    detail_url: Option<String>,
    size: Option<String>,
}

impl SourceBuilder {
    fn build(self) -> Option<Source> {
        Some(Source {
            group: self.group,
            name: self.name?,
            update_date: self.update_date?,
            torrent_url: self.torrent_url?,
            magnet_url: self.magnet_url?,
            detail_url: self.detail_url?,
            size: self.size?,
        })
    }
}

#[allow(dead_code)]
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
enum SearchTableItem {
    /// 番剧名
    Name = 1,

    /// 大小
    Size = 2,

    /// 更新时间
    UpdateDate = 3,

    /// 下载地址
    TorrentUrl = 4,

    /// 在线播放
    OnlinePlay = 5,
}

/// 初始化注册蜜柑爬取器
#[ctor]
fn init_mikan_crawler() {
    let mikan = MikanCrawler::new();
    BangumiCrawler::global().register_crawler("mikan", Arc::new(mikan));
}

/// 蜜柑爬取器
#[derive(Default)]
pub struct MikanCrawler {}

impl MikanCrawler {
    pub fn new() -> Self {
        Self::default()
    }

    /// 请求获取资源数据
    async fn fetch_data<T: Serialize + ?Sized>(&self, url: &str, params: &T) -> Result<Bytes> {
        let client = reqwest::Client::new();
        let data = client
            .get(format!("{BASE_URL}{url}"))
            .query(params)
            .send()
            .await?
            .bytes()
            .await?;
        Ok(data)
    }

    /// 解析日期字符串
    fn parse_date(date_str: &str) -> Option<DateTime<Utc>> {
        date_str.split(" ").next().and_then(|date_str| {
            NaiveDate::parse_from_str(date_str, "%Y/%m/%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|datetime| Utc.from_utc_datetime(&datetime))
        })
    }

    /// 从首页解析资源
    fn parse_resource_from_home(reader: &mut Reader<&[u8]>) -> Result<Option<Resource>> {
        let mut resource = ResourceBuilder::default();
        let mut parse_date = false;

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let mut attrs = attrs.to_hashmap()?;
                    let class = attrs.try_get_attribute("class");
                    match tag.name().as_ref() {
                        b"span" if matches!(class.as_ref(), Some(c) if c.contains(b"js-expand_bangumi")) =>
                        {
                            resource.image_url = attrs
                                .try_get_attribute("data-src")
                                .and_then(|a| String::from_utf8(a.value.to_vec()).ok());
                            resource.id = attrs.get_attribute_value("data-bangumiid");
                        }
                        b"a" if matches!(class.as_ref(), Some(c) if c.contains(b"an-text")) => {
                            resource.name = attrs.get_attribute_value("title");
                            resource.link = attrs.get_attribute_value("href");
                        }
                        b"div" if matches!(class.as_ref(), Some(c) if c.contains(b"date-text")) => {
                            parse_date = true;
                        }
                        _ => {}
                    }
                }
                Event::Text(text) if parse_date => {
                    let date_str = text.html_content()?;
                    resource.last_update = MikanCrawler::parse_date(&date_str);
                    parse_date = false;
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(resource.build())
    }

    /// 从首页解析资源集合
    fn parse_resources_from_home(
        reader: &mut Reader<&[u8]>, source: &[u8],
    ) -> Result<Vec<Resource>> {
        let mut ret = Vec::new();

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    if tag.name().as_ref() == b"li" {
                        let range = reader.read_to_end(tag.name())?;
                        let mut reader =
                            Reader::from_reader(&source[range.start as usize..range.end as usize]);
                        reader.config_mut().trim_text(true);
                        reader.config_mut().check_end_names = false;
                        if let Some(resource) = Self::parse_resource_from_home(&mut reader)? {
                            ret.push(resource);
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(ret)
    }

    /// 从首页解析资源集合分组
    fn parse_resources_group_from_home(
        reader: &mut Reader<&[u8]>, source: &[u8],
    ) -> Result<Vec<Resource>> {
        let mut ret = Vec::new();

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let mut attrs = attrs.to_hashmap()?;
                    let class = attrs.try_get_attribute("class");
                    if tag.name().as_ref() == b"ul"
                        && matches!(class.as_ref(), Some(c) if c.contains(b"an-ul"))
                    {
                        let range = reader.read_to_end(tag.name())?;
                        let data = &source[range.start as usize..range.end as usize];
                        let mut reader = Reader::from_reader(data);
                        reader.config_mut().trim_text(true);
                        reader.config_mut().check_end_names = false;
                        ret.extend(Self::parse_resources_from_home(&mut reader, data)?);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(ret)
    }

    /// 解析字幕组项
    fn parse_source_group_item(reader: &mut Reader<&[u8]>) -> Result<Option<Group>> {
        let mut group = GroupBuilder::default();

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let mut attrs = attrs.to_hashmap()?;
                    let class = attrs.try_get_attribute("class");
                    match tag.name().as_ref() {
                        b"div" if matches!(class.as_ref(), Some(c) if c.contains(b"tag-res-name")) =>
                        {
                            group.name = attrs.get_attribute_value("title");
                        }
                        b"div" if matches!(class.as_ref(), Some(c) if c.contains(b"tag-sub")) => {
                            group.id = attrs.get_attribute_value("data-subtitlegroupid");
                        }
                        _ => {}
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(group.build())
    }

    /// 解析来源字幕组
    fn parse_source_group(reader: &mut Reader<&[u8]>, source: &[u8]) -> Result<Vec<Group>> {
        let mut ret = Vec::new();

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let class = attrs.try_get_attribute("class")?;
                    if tag.name().as_ref() == b"li"
                        && matches!(class.as_ref(), Some(c) if c.contains(b"js-expand_bangumi-subgroup"))
                    {
                        let range = reader.read_to_end(tag.name())?;
                        let mut reader =
                            Reader::from_reader(&source[range.start as usize..range.end as usize]);
                        reader.config_mut().trim_text(true);
                        reader.config_mut().check_end_names = false;
                        if let Some(group) = Self::parse_source_group_item(&mut reader)? {
                            ret.push(group);
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(ret)
    }

    /// 解析来源
    fn parse_source(reader: &mut Reader<&[u8]>, group: &Group) -> Result<Option<Source>> {
        let mut source = SourceBuilder::default();
        let mut parse_date = false;
        let mut name_buf: Option<String> = None;

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let mut attrs = attrs.to_hashmap()?;
                    let class = attrs.try_get_attribute("class");
                    match tag.name().as_ref() {
                        b"a" if matches!(class.as_ref(), Some(c) if c.contains("magnet-link-wrap")) =>
                        {
                            source.detail_url = attrs.get_attribute_value("href");
                            name_buf = Some(String::new());
                        }
                        b"a" if matches!(class.as_ref(), Some(c) if c.contains("magnet-link")) => {
                            source.magnet_url = attrs.get_attribute_value("data-clipboard-text");
                        }
                        b"div" if matches!(class.as_ref(), Some(c) if c.contains("res-date")) => {
                            parse_date = true;
                        }
                        b"a" => {
                            if let Some(href) = attrs.get_attribute_value("href") {
                                if href.ends_with(".torrent") {
                                    source.torrent_url = Some(href);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::End(tag) if tag.name().as_ref() == b"a" && name_buf.is_some() => {
                    let name = name_buf.take().unwrap();
                    if let Some(pos) = name.rfind("[") {
                        let size = name[pos..].trim_start_matches('[').trim_end_matches(']');
                        source.size = Some(size.to_string())
                    } else {
                        source.size = Some("".to_string())
                    }
                    source.name = Some(name.trim().to_string());
                }
                Event::Text(text) if name_buf.is_some() => {
                    name_buf.as_mut().unwrap().push(' ');
                    name_buf.as_mut().unwrap().push_str(&text.decode()?);
                }
                Event::GeneralRef(text) if name_buf.is_some() => {
                    let entity_str = String::from_utf8_lossy(text.as_ref());
                    if let Some(decoded) = xml::decode_html_entity(&entity_str) {
                        name_buf.as_mut().unwrap().push_str(&decoded);
                    } else {
                        name_buf.as_mut().unwrap().push_str(&entity_str);
                    }
                }
                Event::Text(text) if parse_date => {
                    let date_str = text.html_content()?;
                    source.update_date = MikanCrawler::parse_date(&date_str);
                    parse_date = false;
                }
                Event::Eof => break,
                _ => {}
            }
        }

        source.group = Some(group.clone());
        Ok(source.build())
    }

    /// 解析来源集合
    fn parse_sources(
        reader: &mut Reader<&[u8]>, source: &[u8], group: &Group,
    ) -> Result<Vec<Source>> {
        let mut ret = Vec::new();
        let mut find_group = false;

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let class = attrs.try_get_attribute("class")?;
                    match tag.name().as_ref() {
                        b"ul" if matches!(class.as_ref(), Some(c) if c.contains(b"res-detail-ul")) =>
                        {
                            find_group = true;
                        }
                        b"li" if find_group => {
                            let range = reader.read_to_end(tag.name())?;
                            let mut reader = Reader::from_reader(
                                &source[range.start as usize..range.end as usize],
                            );
                            reader.config_mut().trim_text(true);
                            reader.config_mut().check_end_names = false;
                            if let Some(source) = Self::parse_source(&mut reader, group)? {
                                ret.push(source);
                            }
                        }
                        _ => {
                            find_group = false;
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(ret)
    }

    /// 解析来源
    fn parse_sources_group(
        reader: &mut Reader<&[u8]>, source: &[u8], groups: Vec<Group>,
    ) -> Result<Vec<Vec<Source>>> {
        let mut ret = Vec::new();
        let mut cur = 0;

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    if let Some(class) = attrs.try_get_attribute("class")? {
                        if class.contains(b"res-mid-frame") {
                            let range = reader.read_to_end(tag.name())?;
                            let group_data = &source[range.start as usize..range.end as usize];
                            let mut reader = Reader::from_reader(group_data);
                            reader.config_mut().trim_text(true);
                            reader.config_mut().check_end_names = false;
                            let sources =
                                Self::parse_sources(&mut reader, group_data, &groups[cur])?;
                            ret.push(sources);
                            cur += 1;
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(ret)
    }

    /// 从搜索结果中解析资源
    fn parse_resource_from_search(reader: &mut Reader<&[u8]>) -> Result<Option<Resource>> {
        let mut resource = ResourceBuilder::default();

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let mut attrs = attrs.to_hashmap()?;
                    let class = attrs.try_get_attribute("class");
                    match tag.name().as_ref() {
                        b"a" => {
                            resource.link = attrs.get_attribute_value("href");
                            if let Some(link) = resource.link.as_ref() {
                                if let Some(idx) = link.rfind("/") {
                                    resource.id = Some(link[idx + 1..].to_string());
                                }
                            }
                        }
                        b"span" => {
                            resource.image_url = attrs
                                .try_get_attribute("data-src")
                                .and_then(|a| String::from_utf8(a.value.to_vec()).ok());
                        }
                        b"div" if matches!(class.as_ref(), Some(c) if c.contains(b"an-text")) => {
                            resource.name = attrs.get_attribute_value("title");
                        }
                        _ => {}
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(resource.build())
    }

    /// 解析从搜索来的资源列表
    fn parse_resources_from_search(
        reader: &mut Reader<&[u8]>, source: &[u8],
    ) -> Result<Vec<Resource>> {
        let mut ret = Vec::new();

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    if tag.name().as_ref() == b"li" {
                        let range = reader.read_to_end(tag.name())?;
                        let data = &source[range.start as usize..range.end as usize];
                        let mut reader = Reader::from_reader(data);
                        reader.config_mut().trim_text(true);
                        reader.config_mut().check_end_names = false;
                        if let Some(resource) = Self::parse_resource_from_search(&mut reader)? {
                            ret.push(resource);
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(ret)
    }

    /// 解析搜索结果中的来源
    fn parse_source_from_search(reader: &mut Reader<&[u8]>) -> Result<Option<Source>> {
        let mut source = SourceBuilder::default();
        let mut name_buf: Option<String> = None;
        let mut num = 0;

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    let mut attrs = attrs.to_hashmap()?;
                    let class = attrs.try_get_attribute("class");
                    match tag.name().as_ref() {
                        b"a" if matches!(class.as_ref(), Some(c) if c.contains("magnet-link-wrap")) =>
                        {
                            source.detail_url = attrs.get_attribute_value("href");
                            name_buf = Some(String::new());
                        }
                        b"a" if matches!(class.as_ref(), Some(c) if c.contains("magnet-link")) => {
                            source.magnet_url = attrs.get_attribute_value("data-clipboard-text");
                        }
                        b"a" => {
                            if let Some(href) = attrs.get_attribute_value("href") {
                                if href.ends_with(".torrent") {
                                    source.torrent_url = Some(href);
                                }
                            }
                        }
                        b"td" => {
                            num += 1;
                        }
                        _ => {}
                    }
                }
                Event::End(tag) if tag.name().as_ref() == b"a" && name_buf.is_some() => {
                    let name = name_buf.take().unwrap();
                    source.name = Some(name.trim().to_string());
                }
                Event::Text(text) if name_buf.is_some() => {
                    name_buf.as_mut().unwrap().push(' ');
                    name_buf.as_mut().unwrap().push_str(&text.decode()?);
                }
                Event::GeneralRef(text) if name_buf.is_some() => {
                    let entity_str = String::from_utf8_lossy(text.as_ref());
                    if let Some(decoded) = xml::decode_html_entity(&entity_str) {
                        name_buf.as_mut().unwrap().push_str(&decoded);
                    } else {
                        name_buf.as_mut().unwrap().push_str(&entity_str);
                    }
                }
                Event::Text(text) if num == SearchTableItem::Size as usize => {
                    source.size = Some(text.html_content()?.to_string());
                }
                Event::Text(text) if num == SearchTableItem::UpdateDate as usize => {
                    let date_str = &text.html_content()?;
                    source.update_date = NaiveDateTime::parse_from_str(date_str, "%Y/%m/%d %H:%M")
                        .ok()
                        .map(|datetime| Utc.from_utc_datetime(&datetime));
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(source.build())
    }

    /// 解析搜索结果中的来源集合
    fn parse_sources_from_search(reader: &mut Reader<&[u8]>, source: &[u8]) -> Result<Vec<Source>> {
        let mut ret = Vec::new();

        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    if tag.name().as_ref() == b"tr" {
                        let mut attrs = tag.attributes();
                        if let Some(class) = attrs.try_get_attribute("class")? {
                            if class.contains("js-search-results-row") {
                                let range = reader.read_to_end(tag.name())?;
                                let data = &source[range.start as usize..range.end as usize];
                                let mut reader = Reader::from_reader(data);
                                reader.config_mut().trim_text(true);
                                reader.config_mut().check_end_names = false;
                                if let Some(source) = Self::parse_source_from_search(&mut reader)? {
                                    ret.push(source);
                                }
                            }
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(ret)
    }
}

#[async_trait]
impl Crawler for MikanCrawler {
    async fn list_quarter(&self) -> Result<Vec<Quarter>> {
        fn append_quarter(list: &mut Vec<Quarter>, year: i32, quarters: &[&str]) {
            quarters.iter().for_each(|q| {
                list.push(Quarter {
                    group: year.to_string(),
                    name: q.to_string(),
                });
            });
        }

        let begin = 2014; // 14 年，mikan 上才开始有完整的四个季度的资源

        let now = Local::now();
        let now_year = now.year();
        let now_month = now.month();
        let now_quarters = now_month.div_ceil(3) as usize;

        // 12 年只有春季、13 年只有秋季
        let mut ret = Vec::with_capacity((now_year - begin + 1 + 2) as usize);

        append_quarter(&mut ret, now_year, &QUARTERS[(4 - now_quarters)..4]);
        for year in (begin..now_year).rev() {
            append_quarter(&mut ret, year, &QUARTERS);
        }
        append_quarter(&mut ret, 2013, &QUARTERS[0..1]);
        append_quarter(&mut ret, 2012, &QUARTERS[2..3]);

        Ok(ret)
    }

    async fn list_resource(&self) -> Result<Vec<Resource>> {
        let params: HashMap<String, String> = hashmap!();
        let data = self.fetch_data("", &params).await?;

        let mut reader = Reader::from_reader(data.as_ref());
        reader.config_mut().trim_text(true);
        reader.config_mut().check_end_names = false;

        let mut sk_body_range = None;
        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    if matches!(attrs.try_get_attribute("id")?, Some(c) if c.contains("sk-body")) {
                        sk_body_range = Some(reader.read_to_end(tag.name())?);
                        break;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        if sk_body_range.is_some() {
            let range = sk_body_range.unwrap();
            let data = &data[range.start as usize..range.end as usize];
            let mut reader = Reader::from_reader(data);
            reader.config_mut().trim_text(true);
            reader.config_mut().check_end_names = false;
            return Self::parse_resources_group_from_home(&mut reader, data);
        }

        Ok(vec![])
    }

    async fn list_resource_from_quarter(&self, quarter: Quarter) -> Result<Vec<Resource>> {
        let params = hashmap!(
            "year" => quarter.group,
            "seasonStr" => quarter.name,
        );

        let data = self
            .fetch_data("/Home/BangumiCoverFlowByDayOfWeek", &params)
            .await?;

        let mut reader = Reader::from_reader(data.as_ref());
        reader.config_mut().trim_text(true);
        reader.config_mut().check_end_names = false;

        Self::parse_resources_group_from_home(&mut reader, &data)
    }

    async fn list_source_group(&self, resource_id: &str) -> Result<Vec<Group>> {
        let params = hashmap!(
            "bangumiId" => resource_id,
            "showSubscribed" => "false",
        );

        let data = self.fetch_data("/Home/ExpandBangumi", &params).await?;
        let mut reader = Reader::from_reader(data.as_ref());
        reader.config_mut().trim_text(true);
        reader.config_mut().check_end_names = false;

        let groups = Self::parse_source_group(&mut reader, &data)?;
        Ok(groups)
    }

    async fn list_subscribe_sources(&self, resource_id: &str) -> Result<Vec<Vec<Source>>> {
        let params = hashmap!(
            "bangumiId" => resource_id,
            "showSubscribed" => "false",
        );

        let data = self.fetch_data("/Home/ExpandBangumi", &params).await?;
        let mut reader = Reader::from_reader(data.as_ref());
        reader.config_mut().trim_text(true);
        reader.config_mut().check_end_names = false;

        let mut groups_range = None;
        while groups_range.is_none() {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let mut attrs = tag.html_attributes();
                    if let Some(class) = attrs.try_get_attribute("class")? {
                        if class.contains(b"res-ul") && tag.name().as_ref() == b"ul" {
                            groups_range = Some(reader.read_to_end(tag.name())?);
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        if groups_range.is_none() {
            return Ok(Vec::new());
        }
        let groups_range = groups_range.unwrap();

        let group_data = &data[groups_range.start as usize..groups_range.end as usize];
        let mut group_reader = Reader::from_reader(group_data);
        group_reader.config_mut().trim_text(true);
        group_reader.config_mut().check_end_names = false;
        let groups = Self::parse_source_group(&mut group_reader, group_data)?;

        Self::parse_sources_group(&mut reader, &data, groups)
    }

    async fn search_resource(&self, keyword: &str) -> Result<(Vec<Resource>, Vec<Source>)> {
        let params = hashmap!("searchstr" => keyword);
        let data = self.fetch_data("/Home/Search", &params).await?;

        let mut reader = Reader::from_reader(data.as_ref());
        reader.config_mut().trim_text(true);
        reader.config_mut().check_end_names = false;

        let mut resource_range = None;
        while resource_range.is_none() {
            match reader.read_event()? {
                Event::Start(tag) => {
                    if tag.name().as_ref() == b"ul" {
                        let mut attrs = tag.html_attributes();
                        if let Some(class) = attrs.try_get_attribute("class")? {
                            if class.contains("an-ul") {
                                resource_range = Some(reader.read_to_end(tag.name())?);
                            }
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        let mut resources = vec![];
        if let Some(range) = resource_range {
            let data = &data[range.start as usize..range.end as usize];
            let mut reader = Reader::from_reader(data);
            reader.config_mut().trim_text(true);
            reader.config_mut().check_end_names = false;
            resources = Self::parse_resources_from_search(&mut reader, data)?;
        }

        let sources = Self::parse_sources_from_search(&mut reader, data.as_ref())?;

        Ok((resources, sources))
    }
}
