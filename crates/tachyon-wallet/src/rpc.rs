//! Direct, bounded Zakura JSON-RPC. This wallet does not use lightwalletd.

use anyhow::{Context, Result, ensure};
use reqwest::{Url, blocking::Client};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, time::Duration};

pub struct Rpc {
    client: Client,
    url: Url,
    cookie: Option<PathBuf>,
}

impl Rpc {
    pub fn new(url: &str, cookie: Option<PathBuf>) -> Result<Self> {
        let url = Url::parse(url)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none(),
            "use an HTTP(S) RPC URL without embedded credentials; use --cookie-file for authentication"
        );
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            url,
            cookie,
        })
    }

    pub fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let mut request = self
            .client
            .post(self.url.clone())
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}));
        if let Some(path) = &self.cookie {
            let cookie = std::fs::read_to_string(path).context("cannot read RPC cookie")?;
            let (user, password) = cookie
                .trim()
                .split_once(':')
                .context("invalid RPC cookie")?;
            request = request.basic_auth(user, Some(password));
        }
        let response = request
            .send()
            .with_context(|| format!("{method}: transport failed"))?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "{method}: RPC response exceeds 16 MiB"
        );
        let envelope: Value = serde_json::from_slice(&bytes).context("invalid RPC JSON")?;
        if !envelope["error"].is_null() {
            anyhow::bail!("{method}: Zakura RPC {}", envelope["error"]);
        }
        ensure!(status.is_success(), "{method}: HTTP {status}");
        ensure!(envelope["id"] == 1, "RPC response ID mismatch");
        Ok(serde_json::from_value(
            envelope
                .get("result")
                .context("missing RPC result")?
                .clone(),
        )?)
    }

    pub fn hash(&self, height: u32) -> Result<String> {
        self.call("getblockhash", json!([height]))
    }
    pub fn height(&self) -> Result<u32> {
        self.call("getblockcount", json!([]))
    }
    pub fn block(&self, height: u32) -> Result<crate::chain::Block> {
        let mut block: crate::chain::Block =
            self.call("gettachyonblock", json!([height.to_string()]))?;
        let ordinary: Value = self.call("getblock", json!([block.hash, 1]))?;
        ensure!(ordinary["hash"] == block.hash, "getblock identity mismatch");
        block.transactions = serde_json::from_value(ordinary["tx"].clone())?;
        for stamp in &block.stamps {
            ensure!(
                block.transactions.get(stamp.transaction_index) == Some(&stamp.txid),
                "stamp transaction position mismatch"
            );
        }
        Ok(block)
    }

    pub fn submit(&self, txid: &str, raw: &str) -> Result<()> {
        let accepted: String = self.call("sendrawtransaction", json!([raw])).context(
            "broadcast outcome may be ambiguous; input remains reserved; retry with submit",
        )?;
        ensure!(
            accepted == txid,
            "node returned a different transaction ID; input remains reserved"
        );
        Ok(())
    }
}
