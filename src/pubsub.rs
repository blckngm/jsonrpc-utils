use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use futures_core::Stream;
use futures_util::StreamExt;
use jsonrpc_core::{MetaIoHandler, Metadata, Params, Value};
use rand::{thread_rng, Rng};
use tokio::sync::mpsc::Sender;

/// Directly send messages to e.g. websocket client and implements `Metadata`.
#[derive(Clone)]
pub struct Session {
    pub raw_tx: Sender<Option<String>>,
    pub id: u64,
}

impl Metadata for Session {}

fn generate_id(session_id: u64) -> String {
    let mut id = [0u8; 24];
    id[..8].copy_from_slice(&session_id.to_be_bytes());
    thread_rng().fill(&mut id[8..]);
    let mut id_hex_bytes = vec![0u8; 50];
    id_hex_bytes[..2].copy_from_slice(b"0x");
    hex::encode_to_slice(&id, &mut id_hex_bytes[2..]).unwrap();
    unsafe { String::from_utf8_unchecked(id_hex_bytes) }
}

fn extract_session_id(id: &[u8]) -> Option<u64> {
    if id.len() < 18 {
        return None;
    }
    let mut out = [0u8; 8];
    if hex::decode_to_slice(&id[2..18], &mut out).is_err() {
        return None;
    }
    Some(u64::from_be_bytes(out))
}

/// Topic based pub/sub.
pub trait PubSub {
    type Stream: Stream<Item = String> + Unpin + Send;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error>;
}

impl<T: PubSub> PubSub for Arc<T> {
    type Stream = T::Stream;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error> {
        T::subscribe(&*self, params)
    }
}

impl<'a, T: PubSub> PubSub for &'a T {
    type Stream = T::Stream;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error> {
        T::subscribe(self, params)
    }
}

/// Add subscribe and unsubscribe methods to the jsonrpc handler.
pub fn add_subscribe_and_unsubscribe(
    io: &mut MetaIoHandler<Option<Session>>,
    pubsub: impl PubSub + Clone + Send + Sync + 'static,
    subscribe_method: &str,
    notify_method: Arc<str>,
    unsubscribe_method: &str,
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
                let id = generate_id(session.id);
                let mut stream = pubsub.subscribe(params)?;
                let handle = tokio::spawn({
                    let id = id.clone();
                    let subscriptions = subscriptions.clone();
                    async move {
                        while let Some(msg) = stream.next().await {
                            let msg = format!(r#"{{"jsonrpc":"2.0","method":"{}","params":{{"subscription":"{}","result":{}}}}}"#,
                                notify_method,
                                id,
                                msg,
                            );
                            if session.raw_tx.send(Some(msg)).await.is_err() {
                                subscriptions.lock().unwrap().remove(&id);
                                return;
                            }
                        }
                        // Stream closed.
                        drop(session.raw_tx.send(None).await);
                        subscriptions.lock().unwrap().remove(&id);
                    }
                });
                subscriptions.lock().unwrap().insert(id.clone(), handle);
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
                let session_id = match extract_session_id(id.as_bytes()) {
                    Some(session_id) => session_id,
                    None => return Ok(Value::Bool(false)),
                };
                if session.map(|s| s.id) != Some(session_id) {
                    return Ok(Value::Bool(false));
                }
                let result = if let Some(handle) = subscriptions.lock().unwrap().remove(&id) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id() {
        let id = generate_id(1);
        assert!(std::str::from_utf8(id.as_bytes()).is_ok());
        assert_eq!(extract_session_id(id.as_bytes()), Some(1));
    }
}
