use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use teloxide::prelude::*;
use tracing::{error, info};

use stake_watch::{
    block_processor, bot, config, db, fork_detector, monitor, notifier, rpc, stake_analyzer, webapp,
};

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "stake_watch",
    about = "Telegram bot for monitoring blockchain staking",
    version
)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    /// Path to the .env file for secrets.
    #[arg(short, long, default_value = ".env")]
    env: String,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Initialize tracing (respects RUST_LOG env var)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 2. Parse CLI args
    let cli = Cli::parse();

    // 3. Load .env file (non-fatal if missing -- env vars may be set externally)
    match dotenvy::from_filename(&cli.env) {
        Ok(path) => info!(path = %path.display(), "Loaded environment file"),
        Err(e) => {
            if cli.env == ".env" {
                info!("No .env file found, using existing environment variables");
            } else {
                return Err(e).with_context(|| format!("Failed to load env file: {}", cli.env));
            }
        }
    }

    // 4. Load configuration
    let config = config::AppConfig::load(&cli.config)?;

    // 5. Load secrets from environment
    let secrets = config::Secrets::load()?;

    // 6. Initialize database
    let db_pool = db::init_db(&config.general.db_path)?;

    // 7. Create RPC client
    let rpc_client: Arc<dyn rpc::RpcClient> = Arc::from(rpc::create_rpc_client(
        &config.backend,
        &secrets,
        &config.chain,
    ));

    // 8. Log startup info
    match rpc_client.get_block_count().await {
        Ok(height) => {
            info!(
                height = height,
                backend = %config.backend.backend_type,
                rpc_url = %config.backend.rpc_url,
                "Connected to blockchain"
            );
        }
        Err(e) => {
            error!(
                error = %e,
                backend = %config.backend.backend_type,
                "Failed to connect to blockchain on startup -- will retry"
            );
        }
    }

    info!(
        db_path = %config.general.db_path,
        fork_detection = config.fork_detection.enabled,
        max_watches = config.general.max_watches_per_user,
        "Stake Watch starting"
    );

    // 9. Set up graceful shutdown
    let shutdown = tokio::signal::ctrl_c();

    // 10. Spawn concurrent tasks
    //
    // Each subsystem runs as an independent tokio task. The main function
    // waits for a shutdown signal (SIGINT / ctrl-c) and then drops the
    // tasks.

    let config = Arc::new(config);
    let secrets = Arc::new(secrets);

    // Create Telegram bot
    let bot = Bot::new(&secrets.telegram_bot_token);

    // Create notifier
    let notifier = Arc::new(notifier::Notifier::new(
        bot.clone(),
        db_pool.clone(),
        config.backend.explorer_url.clone(),
        config.chain.ticker.clone(),
    ));

    // Create broadcast channel for SSE block feed
    let (sse_block_tx, _sse_block_rx) = tokio::sync::broadcast::channel::<String>(64);

    // Create block monitor channel
    let (block_tx, block_rx) = tokio::sync::mpsc::channel::<String>(256);

    // Start block monitor
    let mut block_monitor = monitor::create_monitor(&config.backend, rpc_client.clone());
    let monitor_tx = block_tx.clone();
    let monitor_handle = tokio::spawn(async move {
        if let Err(e) = block_monitor.start(monitor_tx).await {
            error!(error = %e, "Block monitor failed");
        }
    });

    // Start block processor
    // Create a forwarding channel that tees block hashes to the SSE broadcast
    let (fwd_tx, fwd_rx) = tokio::sync::mpsc::channel::<String>(256);
    let sse_tx_for_fwd = sse_block_tx.clone();
    let fwd_handle = tokio::spawn(async move {
        let mut rx = block_rx;
        while let Some(hash) = rx.recv().await {
            let _ = sse_tx_for_fwd.send(hash.clone());
            if fwd_tx.send(hash).await.is_err() {
                break;
            }
        }
    });

    let processor = block_processor::BlockProcessor::new(
        rpc_client.clone(),
        db_pool.clone(),
        notifier.clone(),
        config.chain.has_lottery,
        config.chain.has_vaults,
        config.chain.excluded_addresses.clone(),
    );
    let processor_handle = tokio::spawn(async move {
        processor.run(fwd_rx).await;
    });

    // Start Telegram bot
    let bot_state = Arc::new(bot::BotState::new(
        db_pool.clone(),
        rpc_client.clone(),
        (*config).clone(),
        (*secrets).clone(),
    ));
    let bot_clone = bot.clone();
    let bot_handle = tokio::spawn(async move {
        bot::run_bot(bot_clone, bot_state).await;
    });

    // Start stake alert loop
    let stake_analyzer = stake_analyzer::StakeAnalyzer::new(
        rpc_client.clone(),
        db_pool.clone(),
        notifier.clone(),
        config.general.clone(),
        config.chain.clone(),
    );
    let alert_handle = tokio::spawn(async move {
        stake_analyzer.run_alert_loop().await;
    });

    // Start fork detector (if enabled)
    let fork_handle = if config.fork_detection.enabled {
        let detector = fork_detector::ForkDetector::new(
            db_pool.clone(),
            notifier.clone(),
            config.fork_detection.clone(),
            secrets.admin_telegram_ids.clone(),
        );
        Some(tokio::spawn(async move {
            detector.run().await;
        }))
    } else {
        info!("Fork detection disabled");
        None
    };

    // Start web application server
    let webapp_state = Arc::new(webapp::WebAppState {
        db: db_pool.clone(),
        rpc: rpc_client.clone(),
        config: config.clone(),
        secrets: secrets.clone(),
        explorer_url: config.backend.explorer_url.clone(),
        block_tx: Some(sse_block_tx),
    });
    let webapp_router = webapp::router(webapp_state);
    let webapp_port = std::env::var("WEBAPP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(18095);
    let webapp_addr = std::net::SocketAddr::from(([0, 0, 0, 0], webapp_port));
    let webapp_listener = tokio::net::TcpListener::bind(webapp_addr)
        .await
        .with_context(|| format!("binding webapp to {webapp_addr}"))?;
    info!(port = webapp_port, "Web application server starting");
    let webapp_handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(webapp_listener, webapp_router).await {
            error!(error = %e, "Web application server failed");
        }
    });

    info!("All systems initialized -- press Ctrl+C to shut down");

    // Wait for shutdown signal
    match shutdown.await {
        Ok(()) => info!("Received shutdown signal"),
        Err(e) => error!(error = %e, "Error waiting for shutdown signal"),
    }

    info!("Stake Watch shutting down");

    // Abort tasks
    monitor_handle.abort();
    fwd_handle.abort();
    processor_handle.abort();
    bot_handle.abort();
    alert_handle.abort();
    webapp_handle.abort();
    if let Some(h) = fork_handle {
        h.abort();
    }

    Ok(())
}
