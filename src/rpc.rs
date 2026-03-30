use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, warn};

use crate::config::{BackendConfig, Secrets};

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Block {
    pub hash: String,
    pub height: u64,
    #[serde(default)]
    pub tx: Vec<String>,
    #[serde(default)]
    pub time: u64,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Transaction {
    pub txid: String,
    #[serde(default)]
    pub vin: Vec<Vin>,
    #[serde(default)]
    pub vout: Vec<Vout>,
    #[serde(default)]
    pub blockhash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vin {
    pub coinbase: Option<String>,
    pub txid: Option<String>,
    #[serde(default)]
    pub vout: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vout {
    pub value: f64,
    pub n: u32,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: ScriptPubKey,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptPubKey {
    pub addresses: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub script_type: Option<String>,
    pub asm: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddressBalance {
    pub balance: i64,
    pub received: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddressDelta {
    pub txid: String,
    pub index: u32,
    pub satoshis: i64,
    pub height: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddressValidation {
    pub isvalid: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LotteryWinners {
    pub height: u64,
    pub winners: Vec<LotteryWinner>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LotteryWinner {
    pub address: String,
    pub amount: f64,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait RpcClient: Send + Sync {
    async fn get_block_count(&self) -> Result<u64>;
    async fn get_block_hash(&self, height: u64) -> Result<String>;
    async fn get_block(&self, hash: &str) -> Result<Block>;
    async fn get_raw_transaction(&self, txid: &str) -> Result<Transaction>;
    async fn get_address_balance(&self, address: &str) -> Result<AddressBalance>;
    async fn get_address_deltas(
        &self,
        address: &str,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<Vec<AddressDelta>>;
    async fn get_lottery_block_winners(&self, hash: &str) -> Result<Option<LotteryWinners>>;
    async fn validate_address(&self, address: &str) -> Result<AddressValidation>;
}

// ---------------------------------------------------------------------------
// JSON-RPC implementation
// ---------------------------------------------------------------------------

pub struct JsonRpcClient {
    client: Client,
    rpc_url: String,
    username: Option<String>,
    password: Option<String>,
    request_id: AtomicU64,
}

impl JsonRpcClient {
    pub fn new(rpc_url: String, username: Option<String>, password: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            rpc_url,
            username,
            password,
            request_id: AtomicU64::new(1),
        }
    }

    /// Send a JSON-RPC request. Retries once on network errors.
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "1.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..2 {
            let mut req = self.client.post(&self.rpc_url).json(&body);

            if let (Some(user), Some(pass)) = (&self.username, &self.password) {
                req = req.basic_auth(user, Some(pass));
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp
                        .text()
                        .await
                        .context("reading RPC response body")?;

                    if !status.is_success() {
                        return Err(anyhow!(
                            "RPC HTTP {status} for {method}: {text}"
                        ));
                    }

                    let parsed: Value = serde_json::from_str(&text)
                        .with_context(|| format!("parsing RPC response for {method}"))?;

                    if let Some(err) = parsed.get("error") {
                        if !err.is_null() {
                            return Err(anyhow!("RPC error for {method}: {err}"));
                        }
                    }

                    return Ok(parsed["result"].clone());
                }
                Err(e) => {
                    last_err = Some(e.into());
                    if attempt == 0 {
                        warn!(method, "RPC network error, retrying...");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("RPC call failed for {method}")))
    }
}

#[async_trait]
impl RpcClient for JsonRpcClient {
    async fn get_block_count(&self) -> Result<u64> {
        let result = self.call("getblockcount", json!([])).await?;
        result
            .as_u64()
            .ok_or_else(|| anyhow!("getblockcount: expected u64"))
    }

    async fn get_block_hash(&self, height: u64) -> Result<String> {
        let result = self.call("getblockhash", json!([height])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("getblockhash: expected string"))
    }

    async fn get_block(&self, hash: &str) -> Result<Block> {
        let result = self.call("getblock", json!([hash])).await?;
        let block: Block =
            serde_json::from_value(result).context("deserializing getblock response")?;
        Ok(block)
    }

    async fn get_raw_transaction(&self, txid: &str) -> Result<Transaction> {
        // verbose = 1 to get decoded JSON
        let result = self
            .call("getrawtransaction", json!([txid, 1]))
            .await?;
        let tx: Transaction =
            serde_json::from_value(result).context("deserializing getrawtransaction")?;
        Ok(tx)
    }

    async fn get_address_balance(&self, address: &str) -> Result<AddressBalance> {
        let result = self
            .call("getaddressbalance", json!([{"addresses": [address]}]))
            .await?;
        let balance: AddressBalance =
            serde_json::from_value(result).context("deserializing getaddressbalance")?;
        Ok(balance)
    }

    async fn get_address_deltas(
        &self,
        address: &str,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<Vec<AddressDelta>> {
        let mut params = json!({"addresses": [address]});
        if let Some(s) = start {
            params["start"] = json!(s);
        }
        if let Some(e) = end {
            params["end"] = json!(e);
        }
        let result = self
            .call("getaddressdeltas", json!([params]))
            .await?;
        let deltas: Vec<AddressDelta> =
            serde_json::from_value(result).context("deserializing getaddressdeltas")?;
        Ok(deltas)
    }

    async fn get_lottery_block_winners(&self, hash: &str) -> Result<Option<LotteryWinners>> {
        match self.call("getlotteryblockwinners", json!([hash])).await {
            Ok(result) => {
                if result.is_null() {
                    return Ok(None);
                }
                let winners: LotteryWinners = serde_json::from_value(result)
                    .context("deserializing getlotteryblockwinners")?;
                Ok(Some(winners))
            }
            Err(e) => {
                // Some nodes don't support this RPC. Treat as not available.
                debug!("getlotteryblockwinners not available: {e}");
                Ok(None)
            }
        }
    }

    async fn validate_address(&self, address: &str) -> Result<AddressValidation> {
        let result = self
            .call("validateaddress", json!([address]))
            .await?;
        let validation: AddressValidation =
            serde_json::from_value(result).context("deserializing validateaddress")?;
        Ok(validation)
    }
}

// ---------------------------------------------------------------------------
// Chainz (cryptoid.info) REST implementation
// ---------------------------------------------------------------------------

pub struct ChainzClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl ChainzClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            base_url,
            api_key,
        }
    }

    /// Build a URL appending the API key if present.
    fn url(&self, query: &str) -> String {
        let sep = if query.contains('?') { "&" } else { "?" };
        if let Some(ref key) = self.api_key {
            format!("{}{sep}key={key}", query)
        } else {
            query.to_string()
        }
    }

    /// GET a plain-text endpoint.
    async fn get_text(&self, path: &str) -> Result<String> {
        let url = self.url(&format!("{}?{}", self.base_url, path));
        debug!(url = %url, "chainz GET");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("chainz HTTP request")?;
        let status = resp.status();
        let text = resp.text().await.context("reading chainz response")?;
        if !status.is_success() {
            return Err(anyhow!("chainz HTTP {status}: {text}"));
        }
        Ok(text.trim().to_string())
    }

    /// GET a JSON endpoint.
    async fn get_json(&self, path: &str) -> Result<Value> {
        let text = self.get_text(path).await?;
        let val: Value =
            serde_json::from_str(&text).with_context(|| format!("parsing chainz JSON: {text}"))?;
        Ok(val)
    }
}

#[async_trait]
impl RpcClient for ChainzClient {
    async fn get_block_count(&self) -> Result<u64> {
        let text = self.get_text("q=getblockcount").await?;
        text.parse::<u64>()
            .with_context(|| format!("parsing block count: '{text}'"))
    }

    async fn get_block_hash(&self, height: u64) -> Result<String> {
        self.get_text(&format!("q=getblockhash&height={height}"))
            .await
    }

    async fn get_block(&self, hash: &str) -> Result<Block> {
        let val = self
            .get_json(&format!("q=getblockheader&hash={hash}"))
            .await?;
        let block: Block = serde_json::from_value(val).context(
            "chainz getblockheader may not return full block data (no tx list); \
             consider using JSON-RPC backend for full block support",
        )?;
        Ok(block)
    }

    async fn get_raw_transaction(&self, txid: &str) -> Result<Transaction> {
        let val = self
            .get_json(&format!("q=txinfo&t={txid}"))
            .await?;
        let tx: Transaction =
            serde_json::from_value(val).context("deserializing chainz txinfo")?;
        Ok(tx)
    }

    async fn get_address_balance(&self, address: &str) -> Result<AddressBalance> {
        let text = self
            .get_text(&format!("q=getbalance&a={address}"))
            .await?;
        // chainz returns balance in DIVI (float), convert to satoshis
        let divi: f64 = text
            .parse()
            .with_context(|| format!("parsing chainz balance: '{text}'"))?;
        let satoshis = (divi * 100_000_000.0).round() as i64;
        Ok(AddressBalance {
            balance: satoshis,
            received: satoshis, // chainz getbalance doesn't distinguish; use same value
        })
    }

    async fn get_address_deltas(
        &self,
        _address: &str,
        _start: Option<u64>,
        _end: Option<u64>,
    ) -> Result<Vec<AddressDelta>> {
        Err(anyhow!(
            "getaddressdeltas not supported on chainz backend"
        ))
    }

    async fn get_lottery_block_winners(&self, _hash: &str) -> Result<Option<LotteryWinners>> {
        // Not supported on chainz
        Ok(None)
    }

    async fn validate_address(&self, address: &str) -> Result<AddressValidation> {
        // Offline validation: Divi addresses start with 'D' (mainnet) or 'y' (testnet),
        // are 25-34 characters, and use base58 charset.
        let base58_chars = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let is_valid = (address.starts_with('D') || address.starts_with('y'))
            && (25..=34).contains(&address.len())
            && address.chars().all(|c| base58_chars.contains(c));
        Ok(AddressValidation { isvalid: is_valid })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create an appropriate RPC client based on the backend configuration.
///
/// If the rpc_url contains "chainz.cryptoid.info", a `ChainzClient` is
/// created; otherwise a `JsonRpcClient` is used.
pub fn create_rpc_client(config: &BackendConfig, secrets: &Secrets) -> Box<dyn RpcClient> {
    if config.rpc_url.contains("chainz.cryptoid.info") {
        Box::new(ChainzClient::new(
            config.rpc_url.clone(),
            secrets.chainz_api_key.clone(),
        ))
    } else {
        let (user, pass) = if config
            .rpc_auth
            .as_ref()
            .map_or(false, |a| a.enabled)
        {
            (
                secrets.rpc_username.clone(),
                secrets.rpc_password.clone(),
            )
        } else {
            (None, None)
        };
        Box::new(JsonRpcClient::new(config.rpc_url.clone(), user, pass))
    }
}
