use doro::control::{ControlStatus, Status};
use doro::router::Code;
use crate::client_base::{Auth, Client, ClientHandle, Ret};

/// 创建一个 client 实例
pub async fn client() -> anyhow::Result<Client> {
    let addr = "127.0.0.1:3300".parse()?;
    let auth = Auth {
        username: "admin".to_string(),
        password: "admin".to_string(),
    };
    ClientHandle::new(addr, auth).await
}

pub fn verification_result(code: &Code, ret: &Ret) {
    assert_eq!(ret.code, *code);
    assert_eq!(ret.status, ControlStatus::Ok as Status);
}