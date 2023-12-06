//! Example pub/sub server with broadcast.

use std::{sync::Arc, time::Duration};

use futures_util::StreamExt;
use jsonrpc_core::{MetaIoHandler, Params};
use jsonrpc_utils::{
    axum_utils::jsonrpc_router,
    pub_sub::{add_pub_sub, PublishMsg},
    stream::StreamServerConfig,
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

#[tokio::main]
async fn main() {
    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    let (tx, _) = broadcast::channel(8);
    tokio::spawn({
        let tx = tx.clone();
        async move {
            for i in 0u64.. {
                // Error can be ignored.
                //
                // It is recommended to broadcast already serialized
                // `PublishMsg`. This way it only need to serialized once.
                drop(tx.send(PublishMsg::result(&i)));
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });
    add_pub_sub(
        &mut rpc,
        "subscribe",
        "subscription",
        "unsubscribe",
        move |_params: Params| {
            Ok(BroadcastStream::new(tx.subscribe()).map(|result| {
                result.unwrap_or_else(|_| {
                    PublishMsg::error(&jsonrpc_core::Error {
                        code: jsonrpc_core::ErrorCode::ServerError(-32000),
                        message: "lagged".into(),
                        data: None,
                    })
                })
            }))
        },
    );
    let rpc = Arc::new(rpc);
    let config = StreamServerConfig::default()
        .with_channel_size(4)
        .with_pipeline_size(4)
        .with_keep_alive(true);
    let app = jsonrpc_router("/rpc", rpc.clone(), config);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
