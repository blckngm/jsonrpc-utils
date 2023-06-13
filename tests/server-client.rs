#![cfg(feature = "client")]

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use jsonrpc_core::{MetaIoHandler, Result};
use jsonrpc_utils::{
    axum_utils::jsonrpc_router, rpc, rpc_client, stream::StreamServerConfig, HttpClient,
};
use serde_json::value::RawValue;

#[rpc]
#[async_trait]
trait MyRpc {
    async fn sleep(&self, x: u64) -> Result<u64>;
    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
    #[rpc(name = "@ping")]
    fn ping(&self) -> Result<String>;
}

#[derive(Clone)]
struct RpcImpl;

#[async_trait]
impl MyRpc for RpcImpl {
    async fn sleep(&self, x: u64) -> Result<u64> {
        tokio::time::sleep(Duration::from_secs(x)).await;
        Ok(x)
    }

    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32> {
        Ok(x + y + z)
    }

    fn ping(&self) -> Result<String> {
        Ok("pong".into())
    }
}

struct MyRpcClient {
    inner: HttpClient,
}

#[rpc_client]
impl MyRpcClient {
    async fn sleep(&self, secs: u64) -> anyhow::Result<u64>;
    async fn add(&self, (x, y): (i32, i32), z: i32) -> anyhow::Result<i32>;
    #[rpc(name = "@ping")]
    async fn ping(&self) -> anyhow::Result<String>;
}

#[tokio::test]
async fn test_server_client() {
    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    add_my_rpc_methods(&mut rpc, RpcImpl);
    let rpc = Arc::new(rpc);
    let stream_config = StreamServerConfig::default().with_keep_alive(true);
    let app = jsonrpc_router("/rpc", rpc, stream_config);
    let listener = tokio::net::TcpListener::bind(&"[::1]:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = MyRpcClient {
        inner: HttpClient::new(format!("http://{}/rpc", server_addr)),
    };
    client.sleep(1).await.unwrap();
    assert_eq!(client.add((3, 4), 5).await.unwrap(), 12);
    assert_eq!(client.ping().await.unwrap(), "pong");

    // Test [] params.
    assert_eq!(
        client
            .inner
            .rpc("@ping", &RawValue::from_string("[]".into()).unwrap())
            .await
            .unwrap(),
        "pong"
    );
}
