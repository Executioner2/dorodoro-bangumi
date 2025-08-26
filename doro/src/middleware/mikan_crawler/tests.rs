use doro_util::default_logger;
use tracing::{Level, info};

use crate::entity::Quarter;
use crate::mikan_crawler::MikanCrawler;
use crate::{BangumiCrawler, Crawler};

default_logger!(Level::TRACE);

/// 测试获取季度列表
#[tokio::test]
async fn test_list_quarter() {
    let mikan = MikanCrawler::new();
    let ret = mikan.list_quarter().await;

    assert!(ret.is_ok());

    let ret = ret.unwrap();

    assert_eq!(
        ret.last(),
        Some(&Quarter {
            group: "2012".to_string(),
            name: "春".to_string()
        })
    );

    assert_eq!(ret[ret.len() - 2], Quarter {
        group: "2013".to_string(),
        name: "秋".to_string()
    });
}

/// 测试获取资源列表
#[ignore]
#[tokio::test]
async fn test_list_resource() {
    let mikan = BangumiCrawler::global().get_crawler("mikan").unwrap();
    let ret = mikan.list_resource().await;

    assert!(ret.is_ok());

    info!("ret: {ret:#?}");
}

/// 测试获取季度资源列表
#[ignore]
#[tokio::test]
async fn test_list_resource_from_quarter() {
    let mikan = BangumiCrawler::global().get_crawler("mikan").unwrap();
    let ret = mikan
        .list_resource_from_quarter(Quarter {
            group: "2025".to_string(),
            name: "夏".to_string(),
        })
        .await;

    assert!(ret.is_ok());

    info!("ret: {ret:#?}");
}

/// 测试获取资源字幕组
#[ignore]
#[tokio::test]
async fn test_list_source_group() {
    let mikan = BangumiCrawler::global().get_crawler("mikan").unwrap();
    let ret = mikan
        .list_source_group("3706")
        .await;

    assert!(ret.is_ok());

    info!("ret: {ret:#?}");
}

// 测试获取资源订阅源列表
#[ignore]
#[tokio::test]
async fn test_list_subscribe_sources() {
    let mikan = BangumiCrawler::global().get_crawler("mikan").unwrap();
    let ret = mikan
        .list_subscribe_sources("3706")
        .await;

    assert!(ret.is_ok());

    info!("ret: {ret:#?}");
}

/// 测试搜索资源
#[ignore]
#[tokio::test]
async fn test_search_resource() {
    let mikan = BangumiCrawler::global().get_crawler("mikan").unwrap();
    let ret = mikan.search_resource("吊带袜").await;

    assert!(ret.is_ok());

    info!("ret: {ret:#?}");
}