#[cfg(feature = "client")]
pub struct HttpClient {
    client: reqwest::Client,
    url: String,
}

#[cfg(feature = "client")]
impl HttpClient {
    pub fn new(url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
        }
    }

    pub fn with_client(url: String, client: reqwest::Client) -> Self {
        Self { client, url }
    }

    pub async fn rpc(
        &self,
        method: &str,
        params: &serde_json::value::RawValue,
    ) -> anyhow::Result<serde_json::Value> {
        let response_body = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            .body(serde_json::to_string(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": method,
                "params": params,
            }))?)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let result = serde_json::from_slice::<jsonrpc_core::Response>(&response_body[..])?;
        let result = match result {
            jsonrpc_core::Response::Single(o) => match o {
                jsonrpc_core::Output::Success(s) => s.result,
                jsonrpc_core::Output::Failure(f) => return Err(f.error.into()),
            },
            _ => anyhow::bail!("unexpected batch response"),
        };
        Ok(result)
    }
}

#[cfg(feature = "blocking-client")]
pub struct BlockingHttpClient {
    client: reqwest::blocking::Client,
    url: String,
}

#[cfg(feature = "blocking-client")]
impl BlockingHttpClient {
    pub fn new(url: String) -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            url,
        }
    }

    pub fn with_client(url: String, client: reqwest::blocking::Client) -> Self {
        Self { client, url }
    }

    pub fn rpc(
        &self,
        method: &str,
        params: &serde_json::value::RawValue,
    ) -> anyhow::Result<serde_json::Value> {
        let response_body = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            .body(serde_json::to_string(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": method,
                "params": params,
            }))?)
            .send()?
            .error_for_status()?
            .bytes()?;
        let result = serde_json::from_slice::<jsonrpc_core::Response>(&response_body[..])?;
        let result = match result {
            jsonrpc_core::Response::Single(o) => match o {
                jsonrpc_core::Output::Success(s) => s.result,
                jsonrpc_core::Output::Failure(f) => return Err(f.error.into()),
            },
            _ => anyhow::bail!("unexpected batch response"),
        };
        Ok(result)
    }
}
