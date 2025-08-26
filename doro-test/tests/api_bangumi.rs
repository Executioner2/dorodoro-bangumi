use doro::entity::{Group, Quarter, Resource, Source};
use doro::router::ret::Ret;
use doro_test::client_util;
use doro_util::{default_logger, hashmap};
use serde::{Deserialize, Serialize};
use tracing::{Level, info};

default_logger!(Level::DEBUG);

#[derive(Serialize, Deserialize, Debug)]
struct ListBangumiReq {
    /// 要是用的爬取器
    crawler: String,

    /// 按季度查询
    quarter: Option<Quarter>,
}

/// 测试获取首页的番剧资源
#[tokio::test]
async fn test_list_resource() {
    let code = 1201;
    let req = ListBangumiReq {
        crawler: "mikan".to_string(),
        quarter: None,
    };

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, req).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<Vec<Resource>>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试按季度查询番剧资源
#[tokio::test]
async fn test_list_resource_from_quarter() {
    let code = 1201;
    let req = ListBangumiReq {
        crawler: "mikan".to_string(),
        quarter: Some(Quarter {
            group: "2025".to_string(),
            name: "夏".to_string(),
        }),
    };

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, req).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<Vec<Resource>>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试获取季度列表
#[tokio::test]
async fn test_list_quarter() {
    let code = 1202;
    let req = hashmap!("crawler" => "mikan");

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, req).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<Vec<Quarter>>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试获取资源分组列表
#[tokio::test]
async fn test_list_source_group() {
    let code = 1203;
    let req = hashmap!(
        "crawler" => "mikan",
        "resource_id" => "3713"
    );

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, req).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<Vec<Group>>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试获取订阅源列表
#[tokio::test]
async fn test_list_subscribe_sources() {
    let code = 1204;
    let req = hashmap!(
        "crawler" => "mikan",
        "resource_id" => "3713"
    );

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, req).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<Vec<Vec<Source>>>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试搜索资源
#[tokio::test]
async fn test_search_resource() {
    let code = 1205;
    let req = hashmap!(
        "crawler" => "mikan",
        "keyword" => "物语"
    );

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, req).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<(Vec<Resource>, Vec<Source>)>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}
