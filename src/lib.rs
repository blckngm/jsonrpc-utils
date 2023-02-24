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
pub use jsonrpc_utils_macros::rpc;
#[cfg(feature = "macros")]
pub use jsonrpc_utils_macros::rpc_client;
