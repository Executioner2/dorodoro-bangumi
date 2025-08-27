use std::path::PathBuf;

use doro::config::{ClientAuth, ConfigInner};
use doro::router::ret::Ret;
use doro_test::client_util;
use doro_util::default_logger;
use tracing::{Level, info};

default_logger!(Level::DEBUG);

/// 测试还原默认设置
#[tokio::test]
async fn test_restore_default_config() {
    let code = 1301;

    let mut client = client_util::client().await.unwrap();
    let rf = client.request::<()>(code, None).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<ConfigInner>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {data:#?}");
}

/// 测试获取系统配置
#[tokio::test]
async fn test_get_config() {
    let code = 1302;

    let mut client = client_util::client().await.unwrap();
    let rf = client.request::<()>(code, None).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<ConfigInner>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {data:#?}");
}

/// 测试更新系统配置
#[ignore]
#[tokio::test]
async fn test_update_config() {
    let code = 1303;
    let mut config = ConfigInner::default();
    config.set_default_download_dir(PathBuf::from("/tmp/test"));

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, Some(config)).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<bool>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {data:#?}");
}

/// 测试更新客户端认证信息
#[ignore]
#[tokio::test]
async fn test_update_auth() {
    let code = 1304;
    let auth = ClientAuth::new("test_user", "test_password");

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, Some(auth)).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<bool>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {data:#?}");
}
