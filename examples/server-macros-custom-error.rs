//! Example server with macros.

use std::{fmt::Display, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures_core::{stream::BoxStream, Stream};
use jsonrpc_core::{ErrorCode, MetaIoHandler};
use jsonrpc_utils::{
    axum_utils::jsonrpc_router, pub_sub::PublishMsg, rpc, stream::StreamServerConfig,
};
use serde_json::Value;

struct MyError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl<E: Display> From<E> for MyError {
    fn from(e: E) -> Self {
        Self {
            code: ErrorCode::InternalError.code(),
            message: e.to_string(),
            data: None,
        }
    }
}

impl From<MyError> for jsonrpc_core::Error {
    fn from(e: MyError) -> Self {
        Self {
            code: e.code.into(),
            message: e.message,
            data: e.data,
        }
    }
}

type Result<T, E = MyError> = std::result::Result<T, E>;

#[rpc]
#[async_trait]
trait MyRpc {
    async fn sleep(&self, x: u64) -> Result<u64>;
    async fn value(&self, x: Option<u64>) -> Result<u64>;
    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
    #[rpc(name = "@ping")]
    fn ping(&self) -> Result<String>;

    type S: Stream<Item = PublishMsg<u64>> + Send + 'static;
    #[rpc(pub_sub(notify = "subscription", unsubscribe = "unsubscribe"))]
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
            Err(MyError {
                code: ErrorCode::InvalidParams.code(),
                message: "invalid interval".into(),
                data: None,
            })
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
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
