use std::sync::{atomic::AtomicU64, Arc};

use axum::{
    body::Bytes,
    extract::{ws::Message, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Extension, Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use jsonrpc_core::{MetaIoHandler, Metadata};
use tokio::sync::mpsc::channel;

use crate::pubsub::Session;

/// Axum handler to handle HTTP jsonrpc request.
pub async fn handle_jsonrpc<T: Default + Metadata>(
    Extension(io): Extension<MetaIoHandler<T>>,
    req_body: Bytes,
) -> Response {
    let req = match std::str::from_utf8(req_body.as_ref()) {
        Ok(req) => req,
        Err(_) => {
            return Json(jsonrpc_core::Failure {
                jsonrpc: Some(jsonrpc_core::Version::V2),
                error: jsonrpc_core::Error::parse_error(),
                id: jsonrpc_core::Id::Null,
            })
            .into_response();
        }
    };

    if let Some(r) = io.handle_request(req, T::default()).await {
        Json(r).into_response()
    } else {
        StatusCode::NO_CONTENT.into_response()
    }
}

pub struct WebSocketConfig {
    channel_size: usize,
    pipeline_size: usize,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            channel_size: 8,
            pipeline_size: 1,
        }
    }
}

impl WebSocketConfig {
    /// Set websocket channel size.
    ///
    /// Default is 8.
    ///
    /// # Panics
    ///
    /// If channel_size is 0.
    pub fn with_channel_size(mut self, channel_size: usize) -> Self {
        assert!(channel_size > 0);
        self.channel_size = channel_size;
        self
    }

    /// Set maximum request pipelining.
    ///
    /// Up to `pipeline_size` number of requests will be handled concurrently.
    ///
    /// Default is 1, i.e. no pipelining.
    ///
    /// # Panics
    ///
    /// if `pipeline_size` is 0.
    pub fn with_pipeline_size(mut self, pipeline_size: usize) -> Self {
        assert!(pipeline_size > 0);
        self.pipeline_size = pipeline_size;
        self
    }
}

/// Axum handler for jsonrpc over websocket.
///
/// This supports regular jsonrpc calls and notifications, as well as
/// `subscribe` and `unsubscribe` added via [`add_subscribe_and_unsubscribe`].
pub async fn handle_jsonrpc_ws<T: Metadata + From<Session>>(
    Extension(io): Extension<MetaIoHandler<T>>,
    Extension(config): Extension<Arc<WebSocketConfig>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    static SESSION_ID: AtomicU64 = AtomicU64::new(0);

    ws.on_upgrade(move |socket| async move {
        let (tx, mut rx) = channel(config.channel_size);
        let session = Session {
            id: SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            raw_tx: tx,
        };
        let (mut socket_write, socket_read) = socket.split();
        let mut result_stream = socket_read
            .map(|message_or_err| async {
                let msg = if let Ok(msg) = message_or_err {
                    msg
                } else {
                    return Err(());
                };
                let msg = match msg {
                    Message::Text(msg) => msg,
                    _ => return Ok(None),
                };
                Ok(io.handle_request(&msg, session.clone().into()).await)
            })
            .buffer_unordered(config.pipeline_size);
        loop {
            tokio::select! {
                Some(Ok(option_result)) = result_stream.next() => {
                    if let Some(result) = option_result {
                        if socket_write.send(Message::Text(result)).await.is_err() {
                            break;
                        }
                    }
                }
                Some(Some(msg)) = rx.recv() => {
                    if socket_write.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
    })
}

/// Returns an axum Router that handles jsonrpc requests at the specified
/// `path`. Both HTTP and websocket are supported.
///
/// Subscription added via [`add_subscribe_and_unsubscribe`] is supported on
/// websocket connections.
pub fn jsonrpc_router(
    path: &str,
    rpc: MetaIoHandler<Option<Session>>,
    config: impl Into<Arc<WebSocketConfig>>,
) -> Router {
    Router::new()
        .route(
            path,
            post(handle_jsonrpc::<Option<Session>>).get(handle_jsonrpc_ws::<Option<Session>>),
        )
        .layer(Extension(rpc))
        .layer(Extension(config.into()))
}
