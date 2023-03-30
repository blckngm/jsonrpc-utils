//! Client using macros.
//!
//! Run the `server-macros` example first, then run this:
//!
//! ```command
//! $ cargo run --features=client --example client
//! ```

use anyhow::Result;
use jsonrpc_utils::{rpc_client, HttpClient};

struct MyRpcClient {
    inner: HttpClient,
}

#[rpc_client]
impl MyRpcClient {
    async fn sleep(&self, secs: u64) -> Result<u64>;
    async fn value(&self) -> Result<u64>;
    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
    #[rpc(name = "@ping")]
    async fn ping(&self) -> Result<String>;
}

#[tokio::main]
pub async fn main() {
    let client = MyRpcClient {
        inner: HttpClient::new("http://127.0.0.1:3000/rpc".into()),
    };
    dbg!(client.sleep(1).await.unwrap());
    dbg!(client.value().await.unwrap());
    dbg!(client.add((3, 4), 5).await.unwrap());
    dbg!(client.ping().await.unwrap());
}
