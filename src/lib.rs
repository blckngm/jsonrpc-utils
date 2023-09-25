//! Alternative pub/sub, server, client and macros for jsonrpc-core.
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub extern crate jsonrpc_core;
pub extern crate serde_json;

#[cfg(feature = "axum")]
pub mod axum_utils;
#[cfg(feature = "server")]
pub mod pub_sub;
#[cfg(feature = "server")]
pub mod stream;

mod client;
pub use client::*;

#[cfg(feature = "macros")]
/// Using a rust trait to define RPC methods.
///
/// # Example
///
/// ```rust
/// #[rpc]
/// #[async_trait]
/// trait MyRpc {
///    async fn sleep(&self, x: u64) -> Result<u64>;
///    // Tailing optional parameters are optional.
///    async fn value(&self, x: Option<u64>) -> Result<u64>;
///    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
///    // Override rpc method name.
///    #[rpc(name = "@ping")]
///    fn ping(&self) -> Result<String>;
///
///    type S: Stream<Item = PublishMsg<u64>> + Send + 'static;
///    #[rpc(pub_sub(notify = "subscription", unsubscribe = "unsubscribe"))]
///    fn subscribe(&self, interval: u64) -> Result<Self::S>;
/// }
///
/// #[async_trait]
/// impl MyRpc for MyRpcImpl {...}
///
/// let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
/// add_my_rpc_methods(&mut rpc, RpcImpl);
/// ```
///
/// # Error handling
///
/// The error type must implement `Into<jsonrpc_core::Error>`.
///
/// # OpenRPC doc generation
///
/// Using `#[rpc(openrpc)]` on e.g. `trait MyRpc` will generate a `my_rpc_doc`
/// function, which will return an OpenRPC document for the RPC methods as a
/// `serde_json::Value`.
///
/// All parameters and results must implement the
/// [JsonSchema](https://docs.rs/schemars/latest/schemars/trait.JsonSchema.html)
/// trait.
///
/// Pub/sub methods won't be included in the generated doc for now.
///
/// Full example:
/// [openrpc.rs](https://github.com/sopium/jsonrpc-utils/blob/master/examples/openrpc.rs)
pub use jsonrpc_utils_macros::rpc;

#[cfg(feature = "macros")]
/// Implement RPC client methods automatically.
///
/// # Example
///
/// ```rust
/// struct MyRpcClient {
///    // This field must have an `rpc` method:
///    // pub async fn rpc(&self, method: &str, params: &RawValue) -> Result<Value>
///    inner: jsonrpc_utils::HttpClient,
/// }
///
/// #[rpc_client]
/// impl MyRpcClient {
///     async fn sleep(&self, secs: u64) -> Result<u64>;
///     async fn value(&self) -> Result<u64>;
///     async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
///     // Override rpc method name.
///     #[rpc(name = "@ping")]
///     async fn ping(&self) -> Result<String>;
/// }
/// ```
///
/// # Blocking Client
///
/// ```rust
/// struct MyRpcClient {
///    // This field must have an `rpc` method:
///    // pub fn rpc(&self, method: &str, params: &RawValue) -> Result<Value>
///     inner: jsonrpc_utils::BlockingHttpClient,
/// }
///
/// #[rpc_client]
/// impl MyRpcClient {
///     fn sleep(&self, secs: u64) -> Result<u64>;
///     fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
///     #[rpc(name = "@ping")]
///     fn ping(&self) -> Result<String>;
/// }
/// ```
pub use jsonrpc_utils_macros::rpc_client;
