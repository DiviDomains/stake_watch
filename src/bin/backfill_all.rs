// One-shot tool: backfill historical stake events for every distinct
// address in the `watched_addresses` table. Reuses the same code path as
// the bot's add-watch flow, so detection logic stays in one place.
//
// Usage:
//   backfill_all --config /opt/stake-watch/config/config.toml --env /opt/stake-watch/.env
//
// Safe to run while the main bot is up: SQLite WAL mode allows concurrent
// reads, and `record_stake_event` is `INSERT OR IGNORE` keyed on
// `(txid, address)`.

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use tracing::{error, info, warn};

use stake_watch::{config, db, rpc, stake_analyzer::StakeAnalyzer};

#[derive(Parser, Debug)]
#[command(
    name = "backfill_all",
    about = "Backfill stake events for all watched addresses"
)]
struct Cli {
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    #[arg(short, long, default_value = ".env")]
    env: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if let Err(e) = dotenvy::from_filename(&cli.env) {
        if cli.env != ".env" {
            return Err(e).with_context(|| format!("Failed to load env file: {}", cli.env));
        }
    }

    let app_config = config::AppConfig::load(&cli.config)?;
    let secrets = config::Secrets::load()?;
    let db_pool = db::init_db(&app_config.general.db_path)?;
    let rpc_client: Arc<dyn rpc::RpcClient> = Arc::from(rpc::create_rpc_client(
        &app_config.backend,
        &secrets,
        &app_config.chain,
    ));

    let tip = rpc_client
        .get_block_count()
        .await
        .context("getting current chain tip")?;
    info!(tip, "Connected to chain");

    let addresses: Vec<String> = {
        let conn = db_pool
            .lock()
            .map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
        let mut stmt =
            conn.prepare("SELECT DISTINCT address FROM watched_addresses ORDER BY address")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    info!(count = addresses.len(), "Backfilling distinct addresses");

    let has_vaults = app_config.chain.has_vaults;
    let excluded = app_config.chain.excluded_addresses.clone();

    let mut ok = 0u32;
    let mut failed = 0u32;
    for addr in &addresses {
        info!(address = %addr, "Backfilling");
        match StakeAnalyzer::backfill_stakes(&rpc_client, &db_pool, addr, has_vaults, &excluded)
            .await
        {
            Ok(()) => {
                ok += 1;
            }
            Err(e) => {
                warn!(address = %addr, error = %e, "Backfill failed");
                failed += 1;
            }
        }
    }

    info!(ok, failed, total = addresses.len(), "Backfill complete");
    if failed > 0 {
        error!(
            failed,
            "Some addresses failed to backfill -- see warnings above"
        );
        std::process::exit(1);
    }
    Ok(())
}
