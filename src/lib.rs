use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{ws::Message, WebSocketUpgrade, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Extension, Json, Router,
};
use jsonrpc_core::{MetaIoHandler, Metadata, Params, Request as JsonrpcRequest, Value};
use rand::{thread_rng, Rng};
use tokio::sync::{
    broadcast,
    mpsc::{channel, Sender},
};

/// Axum handler to handle HTTP jsonrpc request.
pub async fn handle_jsonrpc<T: Default + Metadata>(
    Extension(io): Extension<MetaIoHandler<T>>,
    req: Result<Json<JsonrpcRequest>, JsonRejection>,
) -> Response {
    match req {
        Ok(Json(req)) => {
            if let Some(r) = io.handle_rpc_request(req, T::default()).await {
                Json(r).into_response()
            } else {
                StatusCode::NO_CONTENT.into_response()
            }
        }
        Err(_) => {
            Json(
                jsonrpc_core::Failure {
                    jsonrpc: Some(jsonrpc_core::Version::V2),
                    error: jsonrpc_core::Error::parse_error(),
                    id: jsonrpc_core::Id::Null,
                }
            ).into_response()
        }
    }
}

/// Directly send messages to e.g. websocket client and implements `Metadata`.
#[derive(Clone)]
pub struct Session {
    pub raw_tx: Sender<String>,
}

impl Metadata for Session {}

/// Axum handler for jsonrpc over websocket.
///
/// This supports regular jsonrpc calls and notifications, as well as
/// `subscribe` and `unsubscribe` added via [`add_subscribe_and_unsubscribe`].
pub async fn handle_jsonrpc_ws<T: Metadata + From<Session>>(
    Extension(io): Extension<MetaIoHandler<T>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        // TODO: make channel buffer configurable.
        let (tx, mut rx) = channel(8);
        loop {
            tokio::select! {
                Some(Ok(msg)) = socket.recv() => {
                    match msg {
                        Message::Text(msg) => {
                            if let Some(r) = io.handle_request(&msg, Session { raw_tx: tx.clone() }.into()).await {
                                if socket.send(Message::Text(r)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Message::Binary(msg) => {
                            match String::from_utf8(msg) {
                                Ok(msg) => if let Some(r) = io.handle_request(&msg, Session { raw_tx: tx.clone() }.into()).await {
                                    if socket.send(Message::Text(r)).await.is_err() {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        Message::Ping(_) => {}
                        Message::Pong(_) => {}
                        Message::Close(_) => break,
                    }        
                }
                Some(msg) = rx.recv() => {
                    if socket.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
                else => break,
            };
        }
    })
}

fn random_id() -> String {
    let id: [u8; 24] = thread_rng().gen();
    let mut id_hex_bytes = vec![0u8; 50];
    id_hex_bytes[..2].copy_from_slice(b"0x");
    hex::encode_to_slice(&id, &mut id_hex_bytes[2..]).unwrap();
    unsafe { String::from_utf8_unchecked(id_hex_bytes) }
}

/// Topic based pub/sub.
pub trait PubSub {
    fn subscribe(&self, topic: &str) -> Option<broadcast::Receiver<String>>;
}

impl PubSub for HashMap<String, broadcast::Sender<String>> {
    fn subscribe(&self, topic: &str) -> Option<broadcast::Receiver<String>> {
        self.get(topic).map(|tx| tx.subscribe())
    }
}

impl<T: PubSub> PubSub for Arc<T> {
    fn subscribe(&self, topic: &str) -> Option<broadcast::Receiver<String>> {
        T::subscribe(&*self, topic)
    }
}

impl<'a, T: PubSub> PubSub for &'a T {
    fn subscribe(&self, topic: &str) -> Option<broadcast::Receiver<String>> {
        T::subscribe(self, topic)
    }
}

/// Add subscribe and unsubscribe methods to the jsonrpc handler.
pub fn add_subscribe_and_unsubscribe(
    io: &mut MetaIoHandler<Option<Session>>,
    pubsub: impl PubSub + Clone + Send + Sync + 'static,
    subscribe_method: &str,
    unsubscribe_method: &str,
) {
    let subscriptions0 = Arc::new(Mutex::new(HashMap::new()));
    let subscriptions = subscriptions0.clone();
    io.add_method_with_meta(subscribe_method, move |params: Params, session: Option<Session>| {
        let subscriptions = subscriptions.clone();
        let pubsub = pubsub.clone();
        async move {
            let (topic,): (String,) = params.parse()?;
            let session = session.ok_or_else(jsonrpc_core::Error::method_not_found)?;
            let mut rx = pubsub
                .subscribe(&topic)
                .ok_or_else(|| jsonrpc_core::Error::invalid_params("unknown topic"))?
                .resubscribe();
            let id = random_id();
            let handle = tokio::spawn({
                let id = id.clone();
                let subscriptions = subscriptions.clone();
                async move {
                    while let Ok(msg) = rx.recv().await {
                        if session
                            .raw_tx
                            .send(format!(r##"{{"jsonrpc":"2.0","method":"subscribe","params":{{"subscription":"{}","result":{}}}}}"##, id, msg))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    subscriptions.lock().unwrap().remove(&id);
                }
            });
            subscriptions.lock().unwrap().insert(id.clone(), handle);
            Ok(Value::String(id))
        }
    });
    // Note that a subscription can be cancelled by any client. But this
    // shouldn't be a problem because subscription id is random and
    // un-guessable.
    io.add_method(unsubscribe_method, move |params: Params| {
        let subscriptions = subscriptions0.clone();
        async move {
            let (id,): (String,) = params.parse()?;
            let result = if let Some(handle) = subscriptions.lock().unwrap().remove(&id) {
                handle.abort();
                true
            } else {
                false
            };
            Ok(Value::Bool(result))
        }
    });
}

/// Returns an axum Router that handles jsonrpc requests at the specified
/// `path`. Both HTTP and websocket are supported. Subscription is supported if
/// websocket is used. Use [`add_subscribe_and_unsubscribe`] to add subscription
/// handling to the jsonrpc handler.
pub fn jsonrpc_router(path: &str, rpc: MetaIoHandler<Option<Session>>) -> Router {
    Router::new()
        .route(path, post(handle_jsonrpc::<Option<Session>>).get(handle_jsonrpc_ws::<Option<Session>>))
        .layer(Extension(rpc))
}

#[cfg(test)]
mod tests {
    use axum::http::{header, Request};
    use jsonrpc_core::{Compatibility, Params, Value};

    use super::*;

    #[tokio::test]
    async fn test_simple() {
        use tower::ServiceExt as _;

        let mut io = MetaIoHandler::with_compatibility(Compatibility::Both);
        io.add_method("add", |params: Params| async move {
            let (x, y): (i32, i32) = params.parse()?;
            Ok(Value::from(x + y))
        });
        let router = jsonrpc_router("/rpc", io);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/rpc")
                    .method("POST")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(r#"{"jsonrpc":"2.0","id":1,"method":"add","params":[2,3]}"#.into())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_random_id() {
        let id = random_id();
        assert!(std::str::from_utf8(id.as_bytes()).is_ok());
    }
}
