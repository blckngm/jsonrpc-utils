use std::{pin::Pin, time::Duration};

use futures_core::Stream;
use jsonrpc_core::{MetaIoHandler, Params};
use jsonrpc_utils::{
    axum::{jsonrpc_router, WebSocketConfig},
    pubsub::{add_subscribe_and_unsubscribe, PubSub},
};

struct Publisher {}

impl PubSub for Publisher {
    type Stream = Pin<Box<dyn Stream<Item = String> + Send>>;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error> {
        let (interval,): (u64,) = params.parse()?;
        if interval > 0 {
            Ok(Box::pin(async_stream::stream! {
                loop {
                    tokio::time::sleep(Duration::from_secs(interval)).await;
                    yield "interval".into();
                }
            }))
        } else {
            Err(jsonrpc_core::Error::invalid_params("unknown topic"))
        }
    }
}

#[tokio::main]
async fn main() {
    let publisher: &'static Publisher = Box::leak(Box::new(Publisher {}));
    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    rpc.add_method("sleep", |params: Params| async move {
        let (x,): (u64,) = params.parse()?;
        tokio::time::sleep(Duration::from_secs(x)).await;
        Ok(x.into())
    });
    add_subscribe_and_unsubscribe(
        &mut rpc,
        publisher,
        "subscribe",
        "subscription".into(),
        "unsubscribe",
    );
    let config = WebSocketConfig::default()
        .with_channel_size(4)
        .with_pipeline_size(4);
    let app = jsonrpc_router("/rpc", rpc, config);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
