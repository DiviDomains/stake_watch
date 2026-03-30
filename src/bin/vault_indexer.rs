use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tracing::{error, info};

use stake_watch::config;
use stake_watch::rpc;
use stake_watch::vault_indexer::{api, db as vault_db, scanner};

#[derive(Parser)]
#[command(name = "vault_indexer", about = "Divi vault UTXO indexer")]
struct Cli {
    /// Path to the TOML configuration file
    #[arg(short, long, default_value = "config/config.toml")]
    config: String,

    /// Path to the .env file for secrets
    #[arg(short, long, default_value = ".env")]
    env: String,

    /// Port for the REST API
    #[arg(short, long, default_value = "18095")]
    port: u16,

    /// How many blocks back to scan on first run (0 = full chain)
    #[arg(long, default_value = "200000")]
    scan_depth: u64,

    /// Only scan, don't start the API server or monitor
    #[arg(long)]
    scan_only: bool,
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

    // Load .env file (non-fatal if missing and using default)
    match dotenvy::from_filename(&cli.env) {
        Ok(_) => {}
        Err(_) if cli.env == ".env" => {}
        Err(e) => return Err(e.into()),
    }

    // Load config (reuse bot's config for RPC settings)
    let cfg = config::AppConfig::load(&cli.config)?;
    let secrets = config::Secrets::load()?;

    // Init vault DB (separate from bot DB)
    let db = vault_db::init_db("./data/vault_indexer.db")?;

    // Create RPC client (reuse from bot)
    let rpc_client: Arc<dyn rpc::RpcClient> =
        Arc::from(rpc::create_rpc_client(&cfg.backend, &secrets));

    // Get chain height
    let chain_height = rpc_client.get_block_count().await?;
    let db_height = vault_db::get_last_scanned_height(&db);
    info!(chain_height, db_height, "Vault indexer starting");

    // Calculate scan range
    let start_height = if db_height > 0 {
        db_height + 1
    } else if cli.scan_depth == 0 {
        1 // Full chain scan
    } else {
        chain_height.saturating_sub(cli.scan_depth)
    };

    // Initial scan
    if start_height <= chain_height {
        let s = scanner::Scanner::new(rpc_client.clone(), db.clone());
        info!(
            start = start_height,
            end = chain_height,
            blocks = chain_height - start_height + 1,
            "Starting chain scan"
        );
        s.scan_range(start_height, chain_height).await?;
        info!("Initial scan complete");
    }

    if cli.scan_only {
        info!("Scan-only mode, exiting");
        return Ok(());
    }

    // Start polling monitor for real-time blocks
    let scan = Arc::new(scanner::Scanner::new(rpc_client.clone(), db.clone()));
    let poll_scanner = scan.clone();
    let rpc_for_poll = rpc_client.clone();
    let poll_interval = cfg
        .backend
        .polling
        .as_ref()
        .map(|p| p.interval_secs)
        .unwrap_or(60);

    tokio::spawn(async move {
        info!(interval_secs = poll_interval, "Starting polling monitor");
        let mut last_height = vault_db::get_last_scanned_height(poll_scanner.db());
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(poll_interval)).await;
            match rpc_for_poll.get_block_count().await {
                Ok(current) => {
                    if current > last_height {
                        for h in (last_height + 1)..=current {
                            if let Err(e) = poll_scanner.process_block(h).await {
                                error!(height = h, error = %e, "Error processing block");
                            }
                        }
                        last_height = current;
                    }
                }
                Err(e) => error!(error = %e, "Failed to get block count"),
            }
        }
    });

    // Start API server
    let state = Arc::new(api::AppState { db: db.clone() });
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cli.port)).await?;
    info!(port = cli.port, "Vault Indexer API server starting");

    axum::serve(listener, app).await?;

    Ok(())
}
