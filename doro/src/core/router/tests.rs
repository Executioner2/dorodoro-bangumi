use anyhow::Result;
use bytes::Bytes;
use doro_macro::route;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::register_route;
use crate::router::ret::Ret;

#[derive(Deserialize, Serialize, Debug)]
struct Info {
    length: u64,
    author: Option<String>,
    created_at: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
struct TorrentSource {
    magent: String,
    name: String,
    info: Option<Info>,
}

#[derive(Serialize)]
struct R {
    code: u32,
}

#[route(code = 0)]
async fn fun0(torrent: TorrentSource) -> Result<Ret<bool>> {
    info!("torrent source: {torrent:#?}");
    let ret = Ret::ok(true);
    Ok(ret)
}

#[route(code = 1)]
async fn fun1() -> Result<Ret<R>> {
    Ok(Ret::ok(R { code: 1 }))
}

#[route(code = 2)]
async fn fun2(
    #[param] magent: String, #[param] name: String, #[body] info: Info,
) -> Result<Ret<R>> {
    info!("magent: {magent}, name: {name}\tinfo: {info:#?}");
    Ok(Ret::ok(R { code: 2 }))
}

#[route(code = 3)]
async fn fun3(args: Vec<String>) -> Result<Ret<R>> {
    info!("args: {:#?}", args);
    Ok(Ret::ok(R { code: 3 }))
}

#[route(code = 4)]
async fn fun4(#[param] magent: String, #[body] info: Info) -> Result<Ret<R>> {
    info!("magent: {magent}, \tinfo: {info:#?}");
    Ok(Ret::ok(R { code: 4 }))
}

#[route(code = 5)]
async fn fun5(#[param] magent: String, info: Option<Info>) -> Result<Ret<R>> {
    info!("magent: {magent}, \tinfo: {info:#?}");
    Ok(Ret::ok(R { code: 5 }))
}

// 处理请求
async fn handle_request(code: u32, body: Option<Bytes>) {
    let result = super::handle_request(code, body).await;
    match result {
        Ok(response) => {
            info!("Response: {}", String::from_utf8_lossy(&response));
        }
        Err(e) => error!("Error: {}", e),
    }
}

#[tokio::test]
#[ignore]
#[cfg_attr(miri, ignore)]
#[rustfmt::skip]
async fn test_main() {
    let request = TorrentSource {
        magent: "magent".to_string(),
        name: "name".to_string(),
        info: Some(Info {
            length: 100,
            author: Some("Executioner2".to_string()),
            created_at: None,
        }),
    };
    handle_request(0, Some(Bytes::from_owner(serde_json::to_vec(&request).unwrap()))).await;
    handle_request(1, None).await;
    handle_request(2, Some(Bytes::from_owner(serde_json::to_vec(&request).unwrap()))).await;
    handle_request(4, Some(Bytes::from_owner(serde_json::to_vec(&request).unwrap()))).await;

    let request = vec!["arg1".to_string(), "arg2".to_string()];
    handle_request(3, Some(Bytes::from_owner(serde_json::to_vec(&request).unwrap()))).await;

    
    let request = TorrentSource {
        magent: "magent".to_string(),
        name: "name".to_string(),
        info: None,
    };
    handle_request(5, Some(Bytes::from_owner(serde_json::to_vec(&request).unwrap()))).await;
}
