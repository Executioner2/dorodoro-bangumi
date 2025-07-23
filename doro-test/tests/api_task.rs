use doro::api::task_api::{Task, TorrentRet, TorrentSource};
use doro_test::client_util;
use doro_util::default_logger;
use tracing::{Level, info};
use doro::router::ret::Ret;

default_logger!(Level::DEBUG);

/// 测试解析 torrent 链接，支持磁力链接和文件哈希值
#[tokio::test]
async fn test_parse_torrent_link() {
    let code = 1001;

    todo!("code: {}", code)
}

/// 测试解析本地 torrent 文件
#[tokio::test]
async fn test_parse_torrent_file() {
    let code = 1002;
    let file_path = "./tests/resources/test6.torrent";

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, file_path).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let torrent_ret: Result<Ret<TorrentRet>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(torrent_ret.is_ok());
    assert_eq!(torrent_ret.as_ref().unwrap().status_code, 0);

    let data = torrent_ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试添加任务是否成功
#[tokio::test]
async fn test_add_task() {
    let code = 1003;
    let task = Task {
        task_name: Some("test".to_string()),
        download_path: Some("./download".to_string()),
        source: TorrentSource::LocalFile("./tests/resources/test7.torrent".to_string()),
    };

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, task).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let torrent_ret: Result<Ret<TorrentRet>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(torrent_ret.is_ok());
    assert_eq!(torrent_ret.as_ref().unwrap().status_code, 0);

    let data = torrent_ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}
