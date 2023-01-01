#![cfg(feature = "client")]

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use jsonrpc_core::{MetaIoHandler, Result};
use jsonrpc_utils::{
    axum_utils::jsonrpc_router, rpc, rpc_client, stream::StreamServerConfig, HttpClient,
};

#[rpc]
#[async_trait]
trait MyRpc {
    async fn sleep(&self, x: u64) -> Result<u64>;
    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
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
    async fn ping(&self) -> anyhow::Result<String>;
}

#[tokio::test]
async fn test_server_client() {
    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    add_my_rpc_methods(&mut rpc, RpcImpl);
    let rpc = Arc::new(rpc);
    let stream_config = StreamServerConfig::default().with_keep_alive(true);
    let app = jsonrpc_router("/rpc", rpc, stream_config);
    let server = axum::Server::bind(&"[::1]:0".parse().unwrap()).serve(app.into_make_service());
    let server_addr = server.local_addr();
    tokio::spawn(async move {
        server.await.unwrap();
    });

    let client = MyRpcClient {
        inner: HttpClient::new(format!("http://{}/rpc", server_addr)),
    };
    client.sleep(1).await.unwrap();
    assert_eq!(client.add((3, 4), 5).await.unwrap(), 12);
    assert_eq!(client.ping().await.unwrap(), "pong");
}
