use crate::client_base::{Auth, Client, ClientHandle};

pub async fn client() -> anyhow::Result<Client> {
    let addr = "127.0.0.1:3300".parse()?;
    let auth = Auth {
        username: "admin".to_string(),
        password: "admin".to_string(),
    };
    ClientHandle::new(addr, auth).await
}