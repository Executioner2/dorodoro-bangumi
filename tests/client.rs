use crate::client_base::ClientHandle;
use anyhow::Result;

mod client_base;

#[tokio::test]
async fn test_client_base() -> Result<()> {
    let addr = "127.0.0.1:8080".parse()?;
    let mut client = ClientHandle::new(addr).await?;
    let _response = client.request(10001, "/").await?;
    Ok(())
}