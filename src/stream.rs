use std::sync::atomic::AtomicU64;

use futures_core::Stream;
use futures_util::{Sink, SinkExt, StreamExt};
use jsonrpc_core::{MetaIoHandler, Metadata};
use tokio::sync::mpsc::channel;

use crate::pubsub::Session;

#[derive(Clone)]
pub struct StreamServerConfig {
    pub(crate) channel_size: usize,
    pub(crate) pipeline_size: usize,
}

impl Default for StreamServerConfig {
    fn default() -> Self {
        Self {
            channel_size: 8,
            pipeline_size: 1,
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
}

/// Serve JSON-RPC requests over a bidirectional stream (Stream + Sink).
pub async fn serve_stream_sink<E, T: Metadata + From<Session>>(
    mut sink: impl Sink<String> + Unpin,
    stream: impl Stream<Item = Result<String, E>> + Unpin,
    rpc: &MetaIoHandler<T>,
    config: StreamServerConfig,
) {
    static SESSION_ID: AtomicU64 = AtomicU64::new(0);

    let (tx, mut rx) = channel(config.channel_size);
    let session = Session {
        id: SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        raw_tx: tx,
    };

    let mut result_stream = stream
        .map(|message_or_err| async {
            let msg = if let Ok(msg) = message_or_err {
                msg
            } else {
                return Err(());
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
                            if sink.send(result).await.is_err() {
                                break;
                            }
                        }
                    }
                    _ => break,
                }
            }
            // This will never be None.
            Some(msg) = rx.recv() => {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        }
    }
}
