use crate::client_base::{Auth, Client, ClientHandle};
use anyhow::Result;
use dorodoro_bangumi::control::{ControlStatus, Status};
use dorodoro_bangumi::default_logger;
use tracing::{info, Level};
use dorodoro_bangumi::controller::task::Task;

mod client_base;

default_logger!(Level::DEBUG);

async fn client() -> Result<Client> {
    let addr = "127.0.0.1:3300".parse()?;
    let auth = Auth {
        username: "admin".to_string(),
        password: "admin".to_string(),
    };
    ClientHandle::new(addr, auth).await
}

/// 测试未知的code是否能按照预期报错
#[tokio::test]
async fn test_client_base() {
    let code = 10001;

    let mut client = client().await.unwrap();
    let rf = client.request(code, "/").await.unwrap();
    let ret = rf.await;

    assert_eq!(ret.code, code);
    assert_ne!(ret.status, ControlStatus::Ok as Status);
    assert_eq!(ret.body, format!("No handler for code {code}").as_bytes());
}

/// 测试添加任务是否成功
#[tokio::test]
async fn test_add_task() {
    let code = 1003;
    let task = Task {
        task_name: Some("test".to_string()),
        download_path: Some("./download".to_string()),
        file_path: "./tests/resources/test6.torrent".to_string()
    };

    let mut client = client().await.unwrap();
    let rf = client.request(code, task).await.unwrap();
    let ret = rf.await;

    info!("ret: {:?}", ret);
}
