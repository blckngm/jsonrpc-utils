use std::thread::available_parallelism;

use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::ServerBuilder;
use serde_json::Value;

fn main() {
    let mut rpc = MetaIoHandler::<()>::with_compatibility(jsonrpc_core::Compatibility::V2);
    rpc.add_sync_method("@ping", |_| Ok(Value::String("pong".into())));

    ServerBuilder::new(rpc)
        .threads(available_parallelism().unwrap().into())
        .start_http(&"127.0.0.1:3001".parse().unwrap())
        .unwrap()
        .wait();
}
