use doro::api::rss_api::RSSFeed;
use doro_util::default_logger;
use tracing::{info, Level};
use doro::router::ret::Ret;
use doro_test::client_util;

default_logger!(Level::DEBUG);

/// 测试添加 rss 订阅
#[tokio::test]
async fn test_add_rss_feed() {
    let code = 1101;
    let rss_feed = RSSFeed {
        name: Some("章鱼哔的原罪".to_string()),
        url: "https://mikanani.me/RSS/Bangumi?bangumiId=3649&subgroupid=370".to_string(),
    };

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, rss_feed).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let ret: Result<Ret<bool>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(ret.is_ok());
    assert_eq!(ret.as_ref().unwrap().status_code, 0);

    let data = ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}
