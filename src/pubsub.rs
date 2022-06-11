use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use futures_core::Stream;
use futures_util::StreamExt;
use jsonrpc_core::{serde::Serialize, MetaIoHandler, Metadata, Params, Value};
use rand::{thread_rng, Rng};
use tokio::sync::mpsc::Sender;

/// Transports intend to support pub/sub should provide `Session`s as metadata.
///
/// See websocket implementation for an example.
#[derive(Clone)]
pub struct Session {
    pub raw_tx: Sender<String>,
    pub id: u64,
}

impl Metadata for Session {}

fn generate_id() -> String {
    let id: [u8; 16] = thread_rng().gen();
    let mut id_hex_bytes = vec![0u8; 34];
    id_hex_bytes[..2].copy_from_slice(b"0x");
    hex::encode_to_slice(&id, &mut id_hex_bytes[2..]).unwrap();
    unsafe { String::from_utf8_unchecked(id_hex_bytes) }
}

/// Inner message published to subscribers.
#[derive(Clone)]
pub struct PublishMsg {
    is_err: bool,
    // Make clone cheap.
    value: Arc<str>,
}

impl PublishMsg {
    /// Create a new “result” message by serializing the value into JSON.
    pub fn result(value: &impl Serialize) -> Result<Self, jsonrpc_core::serde_json::Error> {
        Ok(Self {
            is_err: false,
            value: jsonrpc_core::serde_json::to_string(value)?.into(),
        })
    }

    /// Create a new “error” message by serializing the JSONRPC error object.
    pub fn error(err: &jsonrpc_core::Error) -> Result<Self, jsonrpc_core::serde_json::Error> {
        Ok(Self {
            is_err: true,
            value: jsonrpc_core::serde_json::to_string(err)?.into(),
        })
    }

    /// Create a new “result” message.
    ///
    /// `value` must be valid JSON.
    pub fn result_raw_json(value: impl Into<Arc<str>>) -> Self {
        Self {
            is_err: false,
            value: value.into(),
        }
    }

    /// Create a new “error” message.
    ///
    /// `value` must be valid JSON.
    pub fn error_raw_json(value: impl Into<Arc<str>>) -> Self {
        Self {
            is_err: true,
            value: value.into(),
        }
    }
}

/// Implement this trait to define actual pub/sub logic.
///
/// # Streams
///
/// Stream wrappers from tokio-stream can be used, e.g. `BroadcastStream`.
///
/// Or use the async-stream crate to implement streams with async-await. See the example server.
pub trait PubSub {
    type Stream: Stream<Item = PublishMsg> + Send;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error>;
}

impl<F: Fn(Params) -> Result<S, jsonrpc_core::Error>, S: Stream<Item = PublishMsg> + Send> PubSub
    for F
{
    type Stream = S;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error> {
        (self)(params)
    }
}

impl<T: PubSub> PubSub for Arc<T> {
    type Stream = T::Stream;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error> {
        T::subscribe(&*self, params)
    }
}

/// Add subscribe and unsubscribe methods to the jsonrpc handler.
///
/// `notify_method` should have already been escaped for JSON string.
pub fn add_pubsub(
    io: &mut MetaIoHandler<Option<Session>>,
    subscribe_method: &str,
    notify_method: Arc<str>,
    unsubscribe_method: &str,
    pubsub: impl PubSub + Clone + Send + Sync + 'static,
) {
    let subscriptions0 = Arc::new(Mutex::new(HashMap::new()));
    let subscriptions = subscriptions0.clone();
    io.add_method_with_meta(
        subscribe_method,
        move |params: Params, session: Option<Session>| {
            let subscriptions = subscriptions.clone();
            let pubsub = pubsub.clone();
            let notify_method = notify_method.clone();
            async move {
                let session = session.ok_or_else(jsonrpc_core::Error::method_not_found)?;
                let session_id = session.id;
                let id = generate_id();
                let stream = pubsub.subscribe(params)?;
                let stream = terminate_after_one_error(stream);
                let handle = tokio::spawn({
                    let id = id.clone();
                    let subscriptions = subscriptions.clone();
                    async move {
                        tokio::pin!(stream);
                        loop {
                            tokio::select! {
                                msg = stream.next() => {
                                    match msg {
                                        Some(msg) => {
                                            let msg = format_msg(&id, &notify_method, msg);
                                            if session.raw_tx.send(msg).await.is_err() {
                                                break;
                                            }
                                        }
                                        None => break,
                                    }
                                }
                                _ = session.raw_tx.closed() => {
                                    break;
                                }
                            }
                        }
                        subscriptions.lock().unwrap().remove(&(session_id, id));
                    }
                });
                subscriptions
                    .lock()
                    .unwrap()
                    .insert((session_id, id.clone()), handle);
                Ok(Value::String(id))
            }
        },
    );
    io.add_method_with_meta(
        unsubscribe_method,
        move |params: Params, session: Option<Session>| {
            let subscriptions = subscriptions0.clone();
            async move {
                let (id,): (String,) = params.parse()?;
                let session_id = if let Some(session) = session {
                    session.id
                } else {
                    return Ok(Value::Bool(false));
                };
                let result =
                    if let Some(handle) = subscriptions.lock().unwrap().remove(&(session_id, id)) {
                        handle.abort();
                        true
                    } else {
                        false
                    };
                Ok(Value::Bool(result))
            }
        },
    );
}

fn format_msg(id: &str, method: &str, msg: PublishMsg) -> String {
    match msg.is_err {
        false => format!(
            r#"{{"jsonrpc":"2.0","method":"{}","params":{{"subscription":"{}","result":{}}}}}"#,
            method, id, msg.value,
        ),
        true => format!(
            r#"{{"jsonrpc":"2.0","method":"{}","params":{{"subscription":"{}","error":{}}}}}"#,
            method, id, msg.value,
        ),
    }
}

pin_project_lite::pin_project! {
    struct TerminateAfterOneError<S> {
        #[pin]
        inner: S,
        has_error: bool,
    }
}

impl<S> Stream for TerminateAfterOneError<S>
where
    S: Stream<Item = PublishMsg>,
{
    type Item = PublishMsg;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.has_error {
            return None.into();
        }
        let proj = self.project();
        match futures_core::ready!(proj.inner.poll_next(cx)) {
            None => None.into(),
            Some(msg) => {
                if msg.is_err {
                    *proj.has_error = true;
                }
                Some(msg).into()
            }
        }
    }
}

fn terminate_after_one_error<S>(s: S) -> TerminateAfterOneError<S> {
    TerminateAfterOneError {
        inner: s,
        has_error: false,
    }
}

#[cfg(test)]
mod tests {
    use async_stream::stream;
    use jsonrpc_core::{Call, Id, MethodCall, Output, Version};
    use tokio::sync::mpsc::channel;

    use super::*;

    #[test]
    fn test_id() {
        let id = generate_id();
        assert!(std::str::from_utf8(id.as_bytes()).is_ok());
    }

    #[tokio::test]
    async fn test_pubsub() {
        let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
        add_pubsub(&mut rpc, "sub", "notify".into(), "unsub", |_params| {
            Ok(stream! {
                yield PublishMsg::result(&1).unwrap();
                yield PublishMsg::result(&1).unwrap();
            })
        });
        let (raw_tx, mut rx) = channel(1);
        let response = rpc
            .handle_call(
                Call::MethodCall(MethodCall {
                    jsonrpc: Some(Version::V2),
                    method: "sub".into(),
                    params: Params::None,
                    id: Id::Num(1),
                }),
                Some(Session {
                    raw_tx: raw_tx.clone(),
                    id: 1,
                }),
            )
            .await
            .unwrap();
        let sub_id = match response {
            Output::Success(s) => s.result,
            _ => unreachable!(),
        };

        assert!(rx.recv().await.is_some());

        // Unsubscribe with a different id should fail.
        let response = rpc
            .handle_call(
                Call::MethodCall(MethodCall {
                    jsonrpc: Some(Version::V2),
                    method: "unsub".into(),
                    params: Params::Array(vec![sub_id.clone()]),
                    id: Id::Num(2),
                }),
                Some(Session {
                    raw_tx: raw_tx.clone(),
                    id: 2,
                }),
            )
            .await
            .unwrap();
        let result = match response {
            Output::Success(s) => s.result,
            _ => unreachable!(),
        };
        assert!(!result.as_bool().unwrap());

        // Unsubscribe with correct id should succeed.
        let response = rpc
            .handle_call(
                Call::MethodCall(MethodCall {
                    jsonrpc: Some(Version::V2),
                    method: "unsub".into(),
                    params: Params::Array(vec![sub_id.clone()]),
                    id: Id::Num(3),
                }),
                Some(Session { raw_tx, id: 1 }),
            )
            .await
            .unwrap();
        let result = match response {
            Output::Success(s) => s.result,
            _ => unreachable!(),
        };
        assert!(result.as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_terminate_after_one_error() {
        let s = terminate_after_one_error(futures_util::stream::iter([
            PublishMsg::result_raw_json(""),
            PublishMsg::error_raw_json(""),
            PublishMsg::result_raw_json(""),
        ]));
        assert_eq!(s.count().await, 2);
    }
}
