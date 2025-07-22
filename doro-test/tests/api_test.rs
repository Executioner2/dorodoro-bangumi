use doro::control::{ControlStatus, Status};
use doro_test::client_auth;
use doro_util::default_logger;
use tracing::Level;

default_logger!(Level::DEBUG);

/// 测试未知的code是否能按照预期报错
#[tokio::test]
async fn test_client_base() {
    let code = 10001;

    let mut client = client_auth::client().await.unwrap();
    let rf = client.request(code, "/").await.unwrap();
    let ret = rf.await;

    assert_eq!(ret.code, code);
    assert_ne!(ret.status, ControlStatus::Ok as Status);
    assert_eq!(ret.body, format!("No handler for code {code}").as_bytes());
}
