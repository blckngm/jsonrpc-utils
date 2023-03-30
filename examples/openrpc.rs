use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use jsonrpc_core::{MetaIoHandler, Result};
use jsonrpc_utils::{axum_utils::jsonrpc_router, rpc, stream::StreamServerConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

#[derive(Serialize, Deserialize, JsonSchema)]
struct MyStruct0 {
    z: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct MyStruct {
    x: u32,
    y: Vec<u64>,
    z: MyStruct0,
}

#[rpc(openrpc)]
#[async_trait]
trait MyRpc {
    /// Sleep.
    async fn sleep(&self, x: u64) -> Result<u64> {
        tokio::time::sleep(Duration::from_secs(x)).await;
        Ok(x)
    }
    /// Add some numbers.
    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32> {
        Ok(x + y + z)
    }
    fn echo_my_struct(&self, my_struct: MyStruct) -> Result<MyStruct> {
        Ok(my_struct)
    }
    #[rpc(name = "@ping")]
    fn ping(&self) -> Result<String> {
        Ok("pong".into())
    }
}

#[derive(Clone)]
struct MyRpcImpl;

#[async_trait]
impl MyRpc for MyRpcImpl {}

#[tokio::main]
async fn main() {
    let doc = my_rpc_doc();

    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    add_my_rpc_methods(&mut rpc, MyRpcImpl);
    rpc.add_method("rpc.discover", move |_| {
        let doc = doc.clone();
        async move { Ok(doc) }
    });
    let rpc = Arc::new(rpc);

    let stream_config = StreamServerConfig::default().with_keep_alive(true);
    let app = jsonrpc_router("/", rpc, stream_config).layer(CorsLayer::permissive());

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
