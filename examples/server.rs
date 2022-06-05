use std::{collections::HashMap, time::Duration};

use jsonrpc_core::{MetaIoHandler, Params};
use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    let pubsub = Box::leak(Box::new(HashMap::new()));
    let (tx, _) = broadcast::channel(4);
    tokio::spawn({
        let tx = tx.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = tx.send("1".into());
            }
        }
    });
    pubsub.insert("one".into(), tx);
    let pubsub = &*pubsub;
    let mut rpc = MetaIoHandler::with_compatibility(jsonrpc_core::Compatibility::V2);
    rpc.add_sync_method("add", |params: Params| {
        let (x, y): (i32, i32) = params.parse()?;
        Ok((x + y).into())
    });
    jsonrpc_axum::add_subscribe_and_unsubscribe(&mut rpc, pubsub, "subscribe", "unsubscribe");
    let app = jsonrpc_axum::jsonrpc_router("/rpc", rpc);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
