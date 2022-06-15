use std::sync::Arc;

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

use crate::{
    pubsub::Session,
    stream::{serve_stream_sink, StreamMsg, StreamServerConfig},
};

/// Axum handler to handle HTTP jsonrpc request.
pub async fn handle_jsonrpc<T: Default + Metadata>(
    Extension(io): Extension<Arc<MetaIoHandler<T>>>,
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
        ([(axum::http::header::CONTENT_TYPE, "application/json")], r).into_response()
    } else {
        StatusCode::NO_CONTENT.into_response()
    }
}

/// Axum handler for jsonrpc over websocket.
///
/// This supports regular jsonrpc calls and notifications, as well as
/// `subscribe` and `unsubscribe` added via [`add_subscribe_and_unsubscribe`].
pub async fn handle_jsonrpc_ws<T: Metadata + From<Session>>(
    Extension(io): Extension<Arc<MetaIoHandler<T>>>,
    Extension(config): Extension<StreamServerConfig>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        let (socket_write, socket_read) = socket.split();
        let write = socket_write.with(|msg: StreamMsg| async move {
            Ok::<_, axum::Error>(match msg {
                StreamMsg::Str(msg) => Message::Text(msg),
                StreamMsg::Ping => Message::Ping(b"ping".to_vec()),
                StreamMsg::Pong => Message::Pong(vec![]),
            })
        });
        let read = socket_read.filter_map(|msg| async move {
            match msg {
                Ok(Message::Text(t)) => Some(Ok(StreamMsg::Str(t))),
                Ok(Message::Pong(_)) => Some(Ok(StreamMsg::Pong)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        });
        tokio::pin!(write);
        tokio::pin!(read);
        drop(serve_stream_sink(&io, write, read, config).await);
    })
}

/// Returns an axum Router that handles jsonrpc requests at the specified
/// `path`. Both HTTP and websocket are supported.
///
/// Subscription added via [`add_subscribe_and_unsubscribe`] is supported on
/// websocket connections.
pub fn jsonrpc_router(
    path: &str,
    rpc: Arc<MetaIoHandler<Option<Session>>>,
    websocket_config: StreamServerConfig,
) -> Router {
    Router::new()
        .route(
            path,
            post(handle_jsonrpc::<Option<Session>>).get(handle_jsonrpc_ws::<Option<Session>>),
        )
        .layer(Extension(rpc))
        .layer(Extension(websocket_config))
}
