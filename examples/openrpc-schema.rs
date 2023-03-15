use async_trait::async_trait;
use jsonrpc_core::Result;
use jsonrpc_utils::rpc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct MyStruct {
    x: u32,
    y: Vec<u64>,
}

#[rpc(openrpc)]
#[async_trait]
trait MyRpc {
    /// Sleep.
    async fn sleep(&self, x: u64) -> Result<u64>;
    /// Add some numbers.
    async fn add(&self, (x, y): (i32, i32), z: i32) -> Result<i32>;
    fn echo_my_struct(&self, my_struct: MyStruct) -> Result<MyStruct>;
    fn ping(&self) -> Result<String>;
}

#[tokio::main]
async fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&my_rpc_schema()).unwrap()
    );
}
