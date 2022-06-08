use std::{sync::atomic::AtomicU64, time::Duration};

use futures_core::Stream;
use futures_util::{Sink, SinkExt, StreamExt};
use jsonrpc_core::{MetaIoHandler, Metadata};
use tokio::{sync::mpsc::channel, time::Instant};

use crate::pubsub::Session;

#[derive(Clone)]
pub struct StreamServerConfig {
    pub(crate) channel_size: usize,
    pub(crate) pipeline_size: usize,
    pub(crate) keep_alive: bool,
    pub(crate) keep_alive_duration: Duration,
    pub(crate) ping_interval: Duration,
}

impl Default for StreamServerConfig {
    fn default() -> Self {
        Self {
            channel_size: 8,
            pipeline_size: 1,
            keep_alive: false,
            keep_alive_duration: Duration::from_secs(60),
            ping_interval: Duration::from_secs(19),
        }
    }
}

impl StreamServerConfig {
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
}

pub enum StreamMsg {
    Str(String),
    Ping,
    Pong,
}

/// Serve JSON-RPC requests over a bidirectional stream (Stream + Sink).
///
/// # Keepalive
///
/// TODO: document keepalive mechanism.
pub async fn serve_stream_sink<E, T: Metadata + From<Session>>(
    rpc: &MetaIoHandler<T>,
    mut sink: impl Sink<StreamMsg> + Unpin,
    stream: impl Stream<Item = Result<StreamMsg, E>> + Unpin,
    config: StreamServerConfig,
) {
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
            let msg = if let Ok(msg) = message_or_err {
                msg
            } else {
                return Err(());
            };
            let msg = match msg {
                StreamMsg::Str(msg) => msg,
                _ => return Ok(None),
            };
            Ok(rpc.handle_request(&msg, session.clone().into()).await)
        })
        .buffer_unordered(config.pipeline_size);
    loop {
        tokio::select! {
            result = result_stream.next() => {
                match result {
                    Some(Ok(opt_result)) => {
                        if let Some(result) = opt_result {
                            if sink.send(StreamMsg::Str(result)).await.is_err() {
                                break;
                            }
                        }
                        if config.keep_alive {
                            dead_timer
                                .as_mut()
                                .reset(Instant::now() + config.keep_alive_duration);
                        }
                    }
                    _ => break,
                }
            }
            // This will never be None.
            Some(msg) = rx.recv() => {
                if sink.send(StreamMsg::Str(msg)).await.is_err() {
                    break;
                }
            }
            _ = &mut dead_timer, if config.keep_alive => {
                break;
            }
            _ = ping_interval.tick(), if config.keep_alive => {
                if sink.send(StreamMsg::Ping).await.is_err() {
                    break;
                }
            }
        }
    }
}
