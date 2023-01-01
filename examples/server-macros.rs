//! Example server with macros.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures_core::{stream::BoxStream, Stream};
use jsonrpc_core::{MetaIoHandler, Result};
use jsonrpc_utils::{
    axum_utils::jsonrpc_router, pub_sub, pub_sub::PublishMsg, rpc, stream::StreamServerConfig,
};

#[rpc]
#[async_trait]
trait MyRpc {
    async fn sleep(&self, x: u64) -> Result<u64>;
    async fn value(&self, x: Option<u64>) -> Result<u64>;
    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
    fn ping(&self) -> Result<String>;

    type S: Stream<Item = PublishMsg<u64>> + Send + 'static;
    #[pub_sub(notify = "subscription", unsubscribe = "unsubscribe")]
    fn subscribe(&self, interval: u64) -> Result<Self::S>;
}

#[derive(Clone)]
struct RpcImpl;

#[async_trait]
impl MyRpc for RpcImpl {
    async fn sleep(&self, x: u64) -> Result<u64> {
        tokio::time::sleep(Duration::from_secs(x)).await;
        Ok(x)
    }

    async fn value(&self, x: Option<u64>) -> Result<u64> {
        Ok(x.unwrap_or_default())
    }

    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32> {
        Ok(x + y + z)
    }

    fn ping(&self) -> Result<String> {
        Ok("pong".into())
    }

    type S = BoxStream<'static, PublishMsg<u64>>;
    fn subscribe(&self, interval: u64) -> Result<Self::S> {
        if interval > 0 {
            Ok(Box::pin(async_stream::stream! {
                for i in 0..10 {
                    tokio::time::sleep(Duration::from_secs(interval)).await;
                    yield PublishMsg::result(&i);
                }
                yield PublishMsg::error(&jsonrpc_core::Error {
                    code: jsonrpc_core::ErrorCode::ServerError(-32000),
                    message: "ended".into(),
                    data: None,
                });
            }))
        } else {
            Err(jsonrpc_core::Error::invalid_params("invalid interval"))
        }
    }
}

#[tokio::main]
async fn main() {
    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    add_my_rpc_methods(&mut rpc, RpcImpl);
    let rpc = Arc::new(rpc);
    let stream_config = StreamServerConfig::default().with_keep_alive(true);

    let app = jsonrpc_router("/rpc", rpc, stream_config);
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
