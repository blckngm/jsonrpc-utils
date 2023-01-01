//! Alternative pub/sub, server and client implementations for jsonrpc-core.
#![cfg_attr(doc, feature(doc_auto_cfg))]

pub extern crate jsonrpc_core;
pub extern crate serde_json;

pub mod axum_utils;
pub mod pub_sub;
pub mod stream;

mod client;
pub use client::*;

#[cfg(feature = "macros")]
pub use jsonrpc_utils_macros::pub_sub;
/// Generate function to add server stub trait implementation to a `MetaIoHandler`.
///
/// ```no_run
/// # use jsonrpc_core::{Result, MetaIoHandler};
/// # use jsonrpc_utils::{rpc, pub_sub, pub_sub::{PublishMsg, Session}};
/// # use async_trait::async_trait;
/// # use futures_core::Stream;
/// #[rpc]
/// #[async_trait]
/// trait MyRpc {
///     // Tailing `Option` parameters are optional.
///     async fn sleep(&self, x: Option<u64>) -> Result<u64>;
///     async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
///     // Non-async function is supported, but implementation should not block the executor.
///     fn ping(&self) -> Result<String>;
///
///     type S: Stream<Item = PublishMsg<u64>> + Send + 'static;
///     #[pub_sub(notify = "subscription", unsubscribe = "unsubscribe")]
///     fn subscribe(&self, interval: u64) -> Result<Self::S>;
/// }
///
/// #[derive(Clone)]
/// struct RpcImpl;
///
/// #[async_trait]
/// impl MyRpc for RpcImpl {
///     async fn sleep(&self, x: Option<u64>) -> Result<u64> { todo!() }
///     async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32> { todo!() }
///     fn ping(&self) -> Result<String> { Ok("pong".into()) }
///
///     type S = futures_util::stream::BoxStream<'static, PublishMsg<u64>>;
///     fn subscribe(&self, interval: u64) -> Result<Self::S> { todo!() }
/// }
///
/// # let mut rpc: MetaIoHandler<Option<Session>> = None.unwrap();
/// add_my_rpc_methods(&mut rpc, RpcImpl);
/// ```
#[cfg(feature = "macros")]
pub use jsonrpc_utils_macros::rpc;
/// Implement JSONRPC client methods.
/// ```no_run
/// use anyhow::{Result, Context};
/// use jsonrpc_utils::rpc_client;
///
/// struct MyRpcClient {
///     // You can use any clients that have a compatible `rpc` method.
///     inner: jsonrpc_utils::HttpClient,
/// }
///
/// // The `Result` type here is `anyhow::Result`, but it is possible to use other error types.
/// #[rpc_client]
/// impl MyRpcClient {
///     async fn sleep(&self, secs: u64) -> Result<u64>;
///     async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
///     async fn ping(&self) -> Result<String>;
/// }
///
/// struct MyRpcClient1 {
///     inner: jsonrpc_utils::BlockingHttpClient,
/// }
///
/// // Blocking client is also supported.
/// #[rpc_client]
/// impl MyRpcClient1 {
///     fn sleep(&self, secs: u64) -> Result<u64>;
///     fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
///     fn ping(&self) -> Result<String>;
/// }
/// ```
#[cfg(feature = "macros")]
pub use jsonrpc_utils_macros::rpc_client;
