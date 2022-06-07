//! Example HTTP, WebSocket and TCP JSON-RPC server.

use std::{pin::Pin, sync::Arc, time::Duration};

use futures_core::Stream;
use jsonrpc_core::{MetaIoHandler, Params};
use jsonrpc_utils::{
    axum::jsonrpc_router,
    pubsub::{add_pubsub, PubSub, PublishMsg},
    stream::{serve_stream_sink, StreamServerConfig},
};
use tokio::net::TcpListener;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

struct Publisher {}

impl PubSub for Publisher {
    type Stream = Pin<Box<dyn Stream<Item = PublishMsg> + Send>>;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error> {
        let (interval,): (u64,) = params.parse()?;
        if interval > 0 {
            Ok(Box::pin(async_stream::stream! {
                for i in 0..10 {
                    tokio::time::sleep(Duration::from_secs(interval)).await;
                    yield PublishMsg::result(&i).unwrap();
                }
                yield PublishMsg::error_raw_json("\"ended\"");
            }))
        } else {
            Err(jsonrpc_core::Error::invalid_params("invalid interval"))
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
    add_pubsub(
        &mut rpc,
        publisher,
        "subscribe",
        "subscription".into(),
        "unsubscribe",
    );
    let rpc = Arc::new(rpc);
    let stream_config = StreamServerConfig::default()
        .with_channel_size(4)
        .with_pipeline_size(4);

    // HTTP and WS server.
    let app = jsonrpc_router("/rpc", rpc.clone(), stream_config.clone());
    // You can use additional tower-http middlewares to add e.g. CORS.
    tokio::spawn(async move {
        axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
            .serve(app.into_make_service())
            .await
            .unwrap();
    });

    // TCP server with line delimited json codec.
    //
    // You can also use other transports (e.g. TLS, unix socket) and codecs
    // (e.g. netstring, JSON splitter).
    let listener = TcpListener::bind("0.0.0.0:3001").await.unwrap();
    let codec = LinesCodec::new_with_max_length(2 * 1024 * 1024);
    while let Ok((s, _)) = listener.accept().await {
        let rpc = rpc.clone();
        let stream_config = stream_config.clone();
        let codec = codec.clone();
        tokio::spawn(async move {
            let (r, w) = s.into_split();
            let r = FramedRead::new(r, codec.clone());
            let w = FramedWrite::new(w, codec);
            serve_stream_sink(w, r, &rpc, stream_config).await;
        });
    }
}
