Alternative pub/sub and server implementations for jsonrpc-core. Jsonrpc-derive
other than pub/sub is still supported.

# Pub/Sub

Implement the `PubSub` trait or use a closure. Return a `Stream` or an error
according to the params:

```rust
pub trait PubSub {
    type Stream: Stream<Item = PublishMsg> + Send;

    fn subscribe(&self, params: Params) -> Result<Self::Stream, jsonrpc_core::Error>;
}

pub struct PublishMsg { ... }

impl PublishMsg {
    /// Create a new “result” message by serializing the value into JSON.
    pub fn result(value: &impl Serialize) -> Result<Self, jsonrpc_core::serde_json::Error>;
    /// Create a new “error” message by serializing the JSONRPC error object.
    pub fn error(err: &jsonrpc_core::Error) -> Result<Self, jsonrpc_core::serde_json::Error>;
}
```

Any result or error messages will be sent to the subscriber along with the
subscription id, which is generated automatically. The stream is dropped when
the subscription is cancelled or the subscriber is disconnected.

The `async-stream` crate can be used to create streams with async-await. Or you
can use tokio-stream `ReceiverStream`/`BroadcastStream` and futures-util
`StreamExt`/`TryStreamExt` combinators.

TODO: broadcast best practices.

# Servers

Axum HTTP POST and WebSocket router is provided. HTTP POST and WebSocket can be
served on the same port and path. Http concerns e.g. CORS can be handled by
axum/tower-http middlewares.

A generic server for any `Stream` and `Sink` is also provided as
`serve_stream_sink`. TCP, TLS and unix socket servers can be easily implemented
with e.g. tokio-util `FramedRead`/`FramedWrite`.

# Example

See `examples/server.rs`.
