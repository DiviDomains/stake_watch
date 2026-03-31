use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use tracing::info;

// ---------------------------------------------------------------------------
// AppConfig -- loaded from a TOML file
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub backend: BackendConfig,
    #[serde(default)]
    pub fork_detection: ForkDetectionConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    pub db_path: String,
    #[serde(default = "default_staking_supply")]
    pub network_staking_supply: u64,
    #[serde(default = "default_alert_multiplier")]
    pub alert_multiplier: u32,
    #[serde(default = "default_alert_check_interval")]
    pub alert_check_interval_secs: u64,
    #[serde(default = "default_max_watches")]
    pub max_watches_per_user: u32,
}

fn default_staking_supply() -> u64 {
    3_000_000
}
fn default_alert_multiplier() -> u32 {
    3
}
fn default_alert_check_interval() -> u64 {
    300
}
fn default_max_watches() -> u32 {
    20
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackendConfig {
    #[serde(rename = "type")]
    pub backend_type: BackendType,
    pub rpc_url: String,
    pub explorer_url: String,
    pub socketio: Option<SocketIoConfig>,
    pub polling: Option<PollingConfig>,
    pub rpc_auth: Option<RpcAuthConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    #[serde(rename = "socketio")]
    SocketIo,
    Polling,
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendType::SocketIo => write!(f, "socketio"),
            BackendType::Polling => write!(f, "polling"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SocketIoConfig {
    pub url: String,
    pub path: String,
    pub network_filter: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PollingConfig {
    #[serde(default = "default_polling_interval")]
    pub interval_secs: u64,
}

fn default_polling_interval() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcAuthConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ForkDetectionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_fork_check_interval")]
    pub check_interval_secs: u64,
    #[serde(default)]
    pub endpoints: Vec<ForkEndpointConfig>,
}

fn default_fork_check_interval() -> u64 {
    120
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForkEndpointConfig {
    pub name: String,
    pub rpc_url: String,
}

// ---------------------------------------------------------------------------
// AppConfig loading
// ---------------------------------------------------------------------------

impl AppConfig {
    /// Load configuration from a TOML file at the given path.
    pub fn load(path: &str) -> Result<Self> {
        let path = Path::new(path);
        anyhow::ensure!(path.exists(), "Config file not found: {}", path.display());

        let contents =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

        let config: AppConfig =
            toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;

        config.validate()?;

        info!(
            backend = %config.backend.backend_type,
            rpc_url = %config.backend.rpc_url,
            db_path = %config.general.db_path,
            "Configuration loaded"
        );

        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.backend.rpc_url.is_empty(),
            "backend.rpc_url must not be empty"
        );

        if self.backend.backend_type == BackendType::SocketIo {
            anyhow::ensure!(
                self.backend.socketio.is_some(),
                "backend.socketio section required when type = \"socketio\""
            );
        }

        if self.backend.backend_type == BackendType::Polling {
            // polling section is optional; defaults are fine
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Secrets -- loaded from environment variables
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Secrets {
    pub telegram_bot_token: String,
    pub admin_telegram_ids: Vec<i64>,
    pub rpc_username: Option<String>,
    pub rpc_password: Option<String>,
    pub chainz_api_key: Option<String>,
}

impl Secrets {
    /// Load secrets from environment variables (expects dotenvy to have already
    /// been called so .env is in the environment).
    pub fn load() -> Result<Self> {
        let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .context("TELEGRAM_BOT_TOKEN env var is required")?;

        let admin_ids_raw = std::env::var("ADMIN_TELEGRAM_IDS").unwrap_or_default();

        let admin_telegram_ids: Vec<i64> = if admin_ids_raw.is_empty() {
            Vec::new()
        } else {
            admin_ids_raw
                .split(',')
                .map(|s| {
                    s.trim()
                        .parse::<i64>()
                        .with_context(|| format!("Invalid admin telegram id: '{s}'"))
                })
                .collect::<Result<Vec<_>>>()?
        };

        let rpc_username = std::env::var("RPC_USERNAME").ok().filter(|s| !s.is_empty());
        let rpc_password = std::env::var("RPC_PASSWORD").ok().filter(|s| !s.is_empty());
        let chainz_api_key = std::env::var("CHAINZ_API_KEY")
            .ok()
            .filter(|s| !s.is_empty());

        info!(
            admin_count = admin_telegram_ids.len(),
            has_rpc_auth = rpc_username.is_some(),
            has_chainz_key = chainz_api_key.is_some(),
            "Secrets loaded"
        );

        Ok(Self {
            telegram_bot_token,
            admin_telegram_ids,
            rpc_username,
            rpc_password,
            chainz_api_key,
        })
    }

    /// Returns true if the given telegram user id is an admin.
    pub fn is_admin(&self, telegram_id: i64) -> bool {
        self.admin_telegram_ids.contains(&telegram_id)
    }
}
