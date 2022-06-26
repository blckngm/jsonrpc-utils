//! Example HTTP, WebSocket and TCP JSON-RPC server.

use std::{sync::Arc, time::Duration};

use futures_util::{SinkExt, TryStreamExt};
use jsonrpc_core::{MetaIoHandler, Params};
use jsonrpc_utils::{
    axum::jsonrpc_router,
    pub_sub::{add_pub_sub, PublishMsg},
    stream::{serve_stream_sink, StreamMsg, StreamServerConfig},
};
use tokio::net::TcpListener;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec, LinesCodecError};

#[tokio::main]
async fn main() {
    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    rpc.add_method("sleep", |params: Params| async move {
        let (x,): (u64,) = params.parse()?;
        tokio::time::sleep(Duration::from_secs(x)).await;
        Ok(x.into())
    });
    add_pub_sub(
        &mut rpc,
        "subscribe",
        "subscription".into(),
        "unsubscribe",
        |params: Params| {
            let (interval,): (u64,) = params.parse()?;
            if interval > 0 {
                Ok(async_stream::stream! {
                    for i in 0..10 {
                        tokio::time::sleep(Duration::from_secs(interval)).await;
                        yield PublishMsg::result(&i);
                    }
                    yield PublishMsg::error(&jsonrpc_core::Error {
                        code: jsonrpc_core::ErrorCode::ServerError(-32000),
                        message: "ended".into(),
                        data: None,
                    });
                })
            } else {
                Err(jsonrpc_core::Error::invalid_params("invalid interval"))
            }
        },
    );
    let rpc = Arc::new(rpc);
    let stream_config = StreamServerConfig::default()
        .with_channel_size(4)
        .with_pipeline_size(4);

    // HTTP and WS server.
    let ws_config = stream_config.clone().with_keep_alive(true);
    let app = jsonrpc_router("/rpc", rpc.clone(), ws_config);
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
            let r = FramedRead::new(r, codec.clone()).map_ok(StreamMsg::Str);
            let w = FramedWrite::new(w, codec).with(|msg| async move {
                Ok::<_, LinesCodecError>(match msg {
                    StreamMsg::Str(msg) => msg,
                    _ => "".into(),
                })
            });
            tokio::pin!(w);
            drop(serve_stream_sink(&rpc, w, r, stream_config).await);
        });
    }
}
