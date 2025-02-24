//! Axum JSONRPC handlers.
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, Utf8Bytes},
        WebSocketUpgrade,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Extension, Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use jsonrpc_core::{MetaIoHandler, Metadata};

use crate::{
    pub_sub::Session,
    stream::{serve_stream_sink, StreamMsg, StreamServerConfig},
};

/// Axum handler for HTTP POST JSONRPC requests.
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

/// Axum handler for JSONRPC over WebSocket.
///
/// This supports regular jsonrpc calls and notifications, as well as pub/sub
/// with [`mod@crate::pub_sub`].
pub async fn handle_jsonrpc_ws<T: Metadata + From<Session>>(
    Extension(io): Extension<Arc<MetaIoHandler<T>>>,
    Extension(config): Extension<StreamServerConfig>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        let (socket_write, socket_read) = socket.split();
        let write = socket_write.with(|msg: StreamMsg<Utf8Bytes>| async move {
            Ok::<_, axum::Error>(match msg {
                StreamMsg::Str(msg) => Message::Text(msg),
                StreamMsg::Ping => Message::Ping(b"ping"[..].into()),
                StreamMsg::Pong => Message::Pong(b""[..].into()),
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

/// Returns an axum Router that handles JSONRPC requests at the specified
/// `path`. Both HTTP and WebSocket are supported.
///
/// Subscription added via [`mod@crate::pub_sub`] is supported on WebSocket
/// connections.
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
