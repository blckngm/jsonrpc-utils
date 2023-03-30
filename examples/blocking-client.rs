//! Client using macros.
//!
//! Run the `server-macros` example first, then run this.
//!
//! ```command
//! $ cargo run --features=client --example client
//! ```

use anyhow::Result;
use jsonrpc_utils::{rpc_client, BlockingHttpClient};

struct MyRpcClient {
    inner: BlockingHttpClient,
}

#[rpc_client]
impl MyRpcClient {
    fn sleep(&self, secs: u64) -> Result<u64>;
    fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
    #[rpc(name = "@ping")]
    fn ping(&self) -> Result<String>;
}

pub fn main() {
    let client = MyRpcClient {
        inner: BlockingHttpClient::new("http://127.0.0.1:3000/rpc".into()),
    };
    dbg!(client.sleep(1).unwrap());
    dbg!(client.add((3, 4), 5).unwrap());
    dbg!(client.ping().unwrap());
}
