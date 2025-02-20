//! JSONRPC server on any streams, e.g. TCP, unix socket.
//!
//! Use `tokio_util::codec` to convert `AsyncRead`, `AsyncWrite` to `Stream`
//! and `Sink`. Use `LinesCodec` or define you own codec.

use std::{ops::Deref, sync::atomic::AtomicU64, time::Duration};

use futures_core::{future::BoxFuture, Future, Stream};
use futures_util::{
    future::{self, Shared},
    FutureExt, Sink, SinkExt, StreamExt,
};
use jsonrpc_core::{MetaIoHandler, Metadata};
use tokio::{sync::mpsc::channel, time::Instant};

use crate::pub_sub::Session;

#[derive(Clone)]
pub struct StreamServerConfig {
    pub(crate) channel_size: usize,
    pub(crate) pipeline_size: usize,
    pub(crate) keep_alive: bool,
    pub(crate) keep_alive_duration: Duration,
    pub(crate) ping_interval: Duration,
    pub(crate) shutdown_signal: Shared<BoxFuture<'static, ()>>,
}

impl Default for StreamServerConfig {
    fn default() -> Self {
        Self {
            channel_size: 8,
            pipeline_size: 1,
            keep_alive: false,
            keep_alive_duration: Duration::from_secs(60),
            ping_interval: Duration::from_secs(19),
            shutdown_signal: future::pending().boxed().shared(),
        }
    }
}

impl StreamServerConfig {
    /// Set pub-sub channel buffer size.
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

    /// Set whether keep alive is enabled.
    ///
    /// Default is false.
    pub fn with_keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    /// Wait for `keep_alive_duration` after the last message is received, then
    /// close the connection.
    ///
    /// Default is 60 seconds.
    pub fn with_keep_alive_duration(mut self, keep_alive_duration: Duration) -> Self {
        self.keep_alive_duration = keep_alive_duration;
        self
    }

    /// Set interval to send ping messages.
    ///
    /// Default is 19 seconds.
    pub fn with_ping_interval(mut self, ping_interval: Duration) -> Self {
        self.ping_interval = ping_interval;
        self
    }

    pub fn with_shutdown<S>(mut self, shutdown: S) -> StreamServerConfig
    where
        S: Future<Output = ()> + Send + 'static,
    {
        self.shutdown_signal = shutdown.boxed().shared();
        self
    }
}

/// Request/response message for streaming JSON-RPC servers.
///
/// S should be some string-like type. For TCP it's `String`, for WebSocket
/// it's `Utf8Bytes`.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamMsg<S> {
    Str(S),
    Ping,
    Pong,
}

/// Serve JSON-RPC requests over a bidirectional stream (Stream + Sink).
///
/// # Keepalive
///
/// We will response to ping messages with pong messages. We will send out ping
/// messages at the specified interval if keepalive is enabled. If keepalive is
/// enabled and we don't receive any messages over the stream for
/// `keep_alive_duration`, we will stop serving (and this function will return).
pub async fn serve_stream_sink<E, T, S>(
    rpc: &MetaIoHandler<T>,
    mut sink: impl Sink<StreamMsg<S>, Error = E> + Unpin,
    stream: impl Stream<Item = Result<StreamMsg<S>, E>> + Unpin,
    config: StreamServerConfig,
) -> Result<(), E>
where
    T: Metadata + From<Session>,
    S: From<String> + Deref<Target = str>,
{
    static SESSION_ID: AtomicU64 = AtomicU64::new(0);

    let (tx, mut rx) = channel(config.channel_size);
    let session = Session {
        id: SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        raw_tx: tx,
    };

    let dead_timer = tokio::time::sleep(config.keep_alive_duration);
    tokio::pin!(dead_timer);
    let mut ping_interval = tokio::time::interval(config.ping_interval);
    ping_interval.reset();
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut result_stream = stream
        .map(|message_or_err| async {
            let msg = message_or_err?;
            match msg {
                StreamMsg::Str(msg) => Ok(rpc
                    .handle_request(&msg, session.clone().into())
                    .await
                    .map(|res| StreamMsg::Str(res.into()))),
                StreamMsg::Ping => Ok(Some(StreamMsg::Pong)),
                StreamMsg::Pong => Ok(None),
            }
        })
        .buffer_unordered(config.pipeline_size);
    let mut shutdown = config.shutdown_signal;
    loop {
        tokio::select! {
            biased;
            // Response/pong messages.
            result = result_stream.next() => {
                match result {
                    Some(result) => {
                        // Stop serving if the stream returns an error.
                        if let Some(s) = result? {
                            sink.send(s).await?;
                        }
                        // Reset the keepalive timer if we have received anything from the stream.
                        // Ordinary messages as well as pings and pongs will all reset the timer.
                        if config.keep_alive {
                            dead_timer
                                .as_mut()
                                .reset(Instant::now() + config.keep_alive_duration);
                        }
                    }
                    // Stop serving if the stream ends.
                    None => {
                        break;
                    }
                }
            }
            // Subscritpion response messages. This will never be None.
            Some(msg) = rx.recv() => {
                sink.send(StreamMsg::Str(msg.into())).await?;
            }
            _ = ping_interval.tick(), if config.keep_alive => {
                sink.send(StreamMsg::Ping).await?;
            }
            _ = &mut dead_timer, if config.keep_alive => {
                break;
            }
            _ = &mut shutdown => {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn test_ping_pong() {
        let rpc = MetaIoHandler::<Session>::default();

        let stream = stream::iter([Ok(StreamMsg::Ping)]);
        let mut sink: Vec<StreamMsg<String>> = Vec::new();

        let result = serve_stream_sink(&rpc, &mut sink, stream, Default::default()).await;

        assert!(result.is_ok());
        assert_eq!(sink, [StreamMsg::Pong]);
    }

    #[tokio::test]
    async fn test_subscription() {
        let mut rpc = MetaIoHandler::default();
        // Here we use add_method_with_meta instead of our add_pub_sub so that we are sure that
        // the subscription data is sent before the subscirption ok response.
        rpc.add_method_with_meta("subscribe", |_params, session: Session| async move {
            // Send a subscription response through the channel
            session
                .raw_tx
                .send("subscription_data".to_string())
                .await
                .unwrap();
            Ok(serde_json::Value::String("ok".to_string()))
        });

        let stream = async_stream::stream! {
            yield Ok(StreamMsg::Str(
                r#"{"jsonrpc":"2.0","method":"subscribe","params":[],"id":1}"#.to_string(),
            ));
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        tokio::pin!(stream);
        let mut sink: Vec<StreamMsg<String>> = Vec::new();

        let result = serve_stream_sink(&rpc, &mut sink, stream, Default::default()).await;

        assert!(result.is_ok());
        assert_eq!(sink.len(), 2);
        // Test that we receive the subscription ok response and the subscription data, and
        // in the correct order.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(match &sink[0] {
                StreamMsg::Str(s) => s,
                _ => panic!("Expected StreamMsg::Str, got: {:?}", sink[0]),
            })
            .unwrap(),
            serde_json::json!({ "jsonrpc": "2.0", "result": "ok", "id": 1 })
        );
        assert_eq!(sink[1], StreamMsg::Str("subscription_data".to_string()));
    }
}
