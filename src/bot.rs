use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use teloxide::utils::command::BotCommands;
use tracing::{error, info, warn};

use crate::config::{AppConfig, Secrets};
use crate::db::{self, DbPool};
use crate::rpc::RpcClient;
use crate::stake_analyzer::StakeAnalyzer;
use crate::utils::{format_duration, satoshi_to_divi, time_ago, truncate_address};

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Show available commands")]
    Help,
    #[command(description = "Watch an address for staking events")]
    Watch(String),
    #[command(description = "Stop watching an address")]
    Unwatch(String),
    #[command(description = "List your watched addresses")]
    List,
    #[command(description = "Analyze staking performance")]
    Analyze(String),
    #[command(description = "Bot status and health")]
    Status,
    #[command(description = "View alert subscriptions")]
    Alerts,
    #[command(description = "Subscribe to a blockchain alert")]
    Alert(String),
    #[command(description = "Unsubscribe from a blockchain alert")]
    Unalert(String),
    #[command(description = "Subscribe to fork detection alerts")]
    ForkWatch,
    #[command(description = "Unsubscribe from fork alerts")]
    ForkUnwatch,
    #[command(description = "View fork monitoring status")]
    ForkStatus,
    #[command(description = "Add fork monitoring endpoint (admin)")]
    AddFork(String),
    #[command(description = "Remove fork monitoring endpoint (admin)")]
    RemoveFork(String),
}

// ---------------------------------------------------------------------------
// BotState -- shared application state
// ---------------------------------------------------------------------------

pub struct BotState {
    pub db: DbPool,
    pub rpc: Arc<dyn RpcClient>,
    pub config: AppConfig,
    pub secrets: Secrets,
    pub start_time: Instant,
    pub last_block_height: AtomicU64,
    pub monitor_connected: AtomicBool,
}

impl BotState {
    pub fn new(db: DbPool, rpc: Arc<dyn RpcClient>, config: AppConfig, secrets: Secrets) -> Self {
        Self {
            db,
            rpc,
            config,
            secrets,
            start_time: Instant::now(),
            last_block_height: AtomicU64::new(0),
            monitor_connected: AtomicBool::new(false),
        }
    }

    fn is_admin(&self, telegram_id: i64) -> bool {
        self.secrets.is_admin(telegram_id)
    }
}

// ---------------------------------------------------------------------------
// Bot entry-point
// ---------------------------------------------------------------------------

/// Start the Telegram bot polling loop. This function blocks until the bot
/// is shut down via Ctrl-C or the process is terminated.
pub async fn run_bot(bot: Bot, state: Arc<BotState>) {
    let handler = Update::filter_message()
        .filter_command::<Command>()
        .endpoint(command_handler);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

// ---------------------------------------------------------------------------
// Command dispatcher
// ---------------------------------------------------------------------------

async fn command_handler(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<BotState>,
) -> ResponseResult<()> {
    let telegram_id = msg.chat.id.0;
    let username = msg.from.as_ref().and_then(|u| u.username.clone());

    // Ensure user exists and has default watches on any first interaction
    ensure_user_registered(&state, telegram_id, username.as_deref());

    let response = match cmd {
        Command::Start => handle_start(&state, telegram_id, username.as_deref()).await,
        Command::Help => handle_help(&state, telegram_id),
        Command::Watch(arg) => handle_watch(&state, telegram_id, &arg).await,
        Command::Unwatch(arg) => handle_unwatch(&state, telegram_id, &arg).await,
        Command::List => handle_list(&state, telegram_id).await,
        Command::Analyze(arg) => handle_analyze(&state, telegram_id, &arg).await,
        Command::Status => handle_status(&state).await,
        Command::Alerts => handle_alerts(&state, telegram_id).await,
        Command::Alert(arg) => handle_alert(&state, telegram_id, &arg).await,
        Command::Unalert(arg) => handle_unalert(&state, telegram_id, &arg).await,
        Command::ForkWatch => handle_fork_watch(&state, telegram_id).await,
        Command::ForkUnwatch => handle_fork_unwatch(&state, telegram_id).await,
        Command::ForkStatus => handle_fork_status(&state).await,
        Command::AddFork(arg) => handle_add_fork(&state, telegram_id, &arg).await,
        Command::RemoveFork(arg) => handle_remove_fork(&state, telegram_id, &arg).await,
    };

    match response {
        Ok(text) => {
            if let Err(e) = bot
                .send_message(msg.chat.id, &text)
                .parse_mode(ParseMode::Html)
                .await
            {
                error!(chat_id = %msg.chat.id, error = %e, "Failed to send message");
            }
        }
        Err(e) => {
            error!(chat_id = %msg.chat.id, error = %e, "Command handler error");
            let _ = bot
                .send_message(msg.chat.id, format!("Internal error: {e}"))
                .await;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// /start
// ---------------------------------------------------------------------------

/// Default watched addresses added for every new user.
const DEFAULT_WATCHES: &[(&str, &str)] = &[
    ("DPhJsztbZafDc1YeyrRqSjmKjkmLJpQpUn", "Divi Treasury"),
    ("DPujt2XAdHyRcZNB5ySZBBVKjzY2uXZGYq", "Divi Charity"),
];

/// Ensure the user is registered and has default watches.
/// Called on every command — idempotent (add_user and add_watch use INSERT OR IGNORE).
fn ensure_user_registered(state: &BotState, telegram_id: i64, username: Option<&str>) {
    let _ = db::add_user(&state.db, telegram_id, username);
    for (address, label) in DEFAULT_WATCHES {
        let _ = db::add_watch(&state.db, telegram_id, address, Some(label));
    }
}

async fn handle_start(
    state: &BotState,
    telegram_id: i64,
    username: Option<&str>,
) -> Result<String> {
    // User + defaults already ensured by ensure_user_registered
    info!(telegram_id, ?username, "User started bot");

    Ok(concat!(
        "<b>Welcome to Stake Watch!</b>\n\n",
        "I monitor Divi blockchain addresses and notify you about staking rewards, ",
        "lottery wins, and blockchain anomalies.\n\n",
        "<b>Quick start:</b>\n",
        "1. /watch &lt;address&gt; [label] - Start monitoring an address\n",
        "2. /analyze - View staking performance analysis\n",
        "3. /alerts - Set up blockchain alerts\n\n",
        "Use /help to see all available commands.",
    )
    .to_string())
}

// ---------------------------------------------------------------------------
// /help
// ---------------------------------------------------------------------------

fn handle_help(state: &BotState, telegram_id: i64) -> Result<String> {
    let mut text = String::from(
        "<b>Available Commands</b>\n\n\
         <b>Watching</b>\n\
         /watch &lt;address&gt; [label] - Watch an address\n\
         /unwatch &lt;address&gt; - Stop watching\n\
         /list - List watched addresses\n\n\
         <b>Analysis</b>\n\
         /analyze [address] - Staking performance\n\
         /status - Bot health &amp; stats\n\n\
         <b>Alerts</b>\n\
         /alerts - View subscriptions\n\
         /alert &lt;type&gt; [threshold] - Subscribe\n\
         /unalert &lt;type&gt; - Unsubscribe\n\n\
         <b>Fork Detection</b>\n\
         /forkwatch - Subscribe to fork alerts\n\
         /forkunwatch - Unsubscribe from fork alerts\n\
         /forkstatus - View fork monitoring status\n",
    );

    if state.is_admin(telegram_id) {
        text.push_str(
            "\n<b>Admin Commands</b>\n\
             /addfork &lt;name&gt; &lt;rpc_url&gt; - Add fork endpoint\n\
             /removefork &lt;name&gt; - Remove fork endpoint\n",
        );
    }

    Ok(text)
}

// ---------------------------------------------------------------------------
// /watch <address> [label]
// ---------------------------------------------------------------------------

async fn handle_watch(state: &BotState, telegram_id: i64, arg: &str) -> Result<String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Ok("Usage: /watch &lt;address&gt; [label]\n\n\
             Example: /watch D8nQRyfgS5xL7dZDC39i9s41iiCAEeq7Zk My Wallet"
            .to_string());
    }

    // Split into address and optional label
    let mut parts = arg.splitn(2, ' ');
    let address = parts.next().unwrap().trim();
    let label = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());

    // Validate address format
    if !address.starts_with('D') {
        return Ok("Invalid address: must start with 'D' (mainnet Divi address).".to_string());
    }

    // Validate via RPC
    match state.rpc.validate_address(address).await {
        Ok(validation) => {
            if !validation.isvalid {
                return Ok(format!(
                    "Invalid address: <code>{}</code> is not a valid Divi address.",
                    truncate_address(address)
                ));
            }
        }
        Err(e) => {
            warn!(address, error = %e, "Address validation RPC failed, proceeding anyway");
        }
    }

    // Check if already watching
    let existing = db::get_watches_for_user(&state.db, telegram_id)?;
    if existing.iter().any(|w| w.address == address) {
        return Ok(format!(
            "You are already watching <code>{}</code>.",
            truncate_address(address)
        ));
    }

    // Check per-user limit
    let max = state.config.general.max_watches_per_user as usize;
    if existing.len() >= max {
        return Ok(format!(
            "You have reached the maximum of {max} watched addresses. \
             Use /unwatch to remove one first."
        ));
    }

    // Add to DB
    db::add_watch(&state.db, telegram_id, address, label)?;
    info!(telegram_id, address, ?label, "Address watch added");

    // Spawn background backfill task
    let rpc = Arc::clone(&state.rpc);
    let db = Arc::clone(&state.db);
    let addr = address.to_string();
    tokio::spawn(async move {
        if let Err(e) = StakeAnalyzer::backfill_stakes(&rpc, &db, &addr).await {
            warn!(address = %addr, error = %e, "Backfill failed");
        }
    });

    let label_line = match label {
        Some(l) => format!("\nLabel: <b>{l}</b>"),
        None => String::new(),
    };

    Ok(format!(
        "Now watching <code>{address}</code>{label_line}\n\n\
         Scanning for recent staking history..."
    ))
}

// ---------------------------------------------------------------------------
// /unwatch <address>
// ---------------------------------------------------------------------------

async fn handle_unwatch(state: &BotState, telegram_id: i64, arg: &str) -> Result<String> {
    let address = arg.trim();
    if address.is_empty() {
        return Ok("Usage: /unwatch &lt;address&gt;".to_string());
    }

    let removed = db::remove_watch(&state.db, telegram_id, address)?;
    if removed {
        info!(telegram_id, address, "Address watch removed");
        Ok(format!(
            "Stopped watching <code>{}</code>.",
            truncate_address(address)
        ))
    } else {
        Ok(format!(
            "You are not watching <code>{}</code>.",
            truncate_address(address)
        ))
    }
}

// ---------------------------------------------------------------------------
// /list
// ---------------------------------------------------------------------------

async fn handle_list(state: &BotState, telegram_id: i64) -> Result<String> {
    let watches = db::get_watches_for_user(&state.db, telegram_id)?;

    if watches.is_empty() {
        return Ok(
            "You have no watched addresses.\n\nUse /watch &lt;address&gt; to start monitoring."
                .to_string(),
        );
    }

    let mut text = format!(
        "<b>Watched Addresses ({}/{})</b>\n\n",
        watches.len(),
        state.config.general.max_watches_per_user
    );

    for (i, w) in watches.iter().enumerate() {
        let label = match &w.label {
            Some(l) if !l.is_empty() => format!(" ({l})"),
            _ => String::new(),
        };

        let last_stake_info = match &w.last_stake_at {
            Some(ts) => format!("Last stake: {}", time_ago(ts)),
            None => "No stakes detected yet".to_string(),
        };

        text.push_str(&format!(
            "{}. <code>{}</code>{}\n   {}\n\n",
            i + 1,
            truncate_address(&w.address),
            label,
            last_stake_info,
        ));
    }

    Ok(text)
}

// ---------------------------------------------------------------------------
// /analyze [address]
// ---------------------------------------------------------------------------

async fn handle_analyze(state: &BotState, telegram_id: i64, arg: &str) -> Result<String> {
    let address = {
        let trimmed = arg.trim();
        if trimmed.is_empty() {
            // Auto-select if user has exactly one watch
            let watches = db::get_watches_for_user(&state.db, telegram_id)?;
            match watches.len() {
                0 => {
                    return Ok("You have no watched addresses. Use /watch first.".to_string());
                }
                1 => watches[0].address.clone(),
                _ => {
                    return Ok("You have multiple watched addresses. Please specify:\n\
                         /analyze &lt;address&gt;"
                        .to_string());
                }
            }
        } else {
            trimmed.to_string()
        }
    };

    // Fetch balance -- try regular address index first, fall back to vault scan
    let (balance, is_vault) = {
        let regular = match state.rpc.get_address_balance(&address).await {
            Ok(b) => b,
            Err(e) => {
                warn!(address = %address, error = %e, "Failed to fetch balance");
                return Ok(format!(
                    "Could not fetch balance for <code>{}</code>.\nError: {e}",
                    truncate_address(&address)
                ));
            }
        };

        if regular.balance > 0 {
            (regular, false)
        } else {
            // Address index returned 0 -- try vault balance (only_vaults=true)
            match state.rpc.get_vault_balance(&address).await {
                Ok(vault_bal) if vault_bal.balance > 0 => {
                    info!(address = %address, vault_balance = vault_bal.balance, "Using vault balance");
                    (vault_bal, true)
                }
                _ => (regular, false),
            }
        }
    };

    // Get recent stakes from DB
    let stakes = db::get_recent_stakes(&state.db, &address, 1000)?;
    let current_height = state.rpc.get_block_count().await.unwrap_or(0);

    // Use block height to determine when stakes happened (not detected_at
    // which is just the DB insertion time). Divi blocks are ~60 seconds apart.
    let blocks_24h = 24 * 60; // ~1,440 blocks
    let blocks_7d = 7 * 24 * 60; // ~10,080 blocks
    let blocks_30d = 30 * 24 * 60; // ~43,200 blocks

    let stakes_24h = stakes
        .iter()
        .filter(|s| current_height.saturating_sub(s.block_height) < blocks_24h)
        .count();

    let stakes_7d = stakes
        .iter()
        .filter(|s| current_height.saturating_sub(s.block_height) < blocks_7d)
        .count();

    let stakes_30d = stakes
        .iter()
        .filter(|s| current_height.saturating_sub(s.block_height) < blocks_30d)
        .count();

    let avg_amount = if stakes.is_empty() {
        0i64
    } else {
        let total: i64 = stakes.iter().map(|s| s.amount_satoshis).sum();
        total / stakes.len() as i64
    };

    // Compute expected interval
    let expected_secs = StakeAnalyzer::compute_expected_interval(
        balance.balance,
        state.config.general.network_staking_supply,
    );

    // Look up the watch to get last_stake_at and label
    let watch = db::get_watches_for_user(&state.db, telegram_id)?
        .into_iter()
        .find(|w| w.address == address);

    // Derive "last stake" from the most recent stake event's block height
    // rather than detected_at (which is insertion time, not block time).
    let last_stake_info = if let Some(latest) = stakes.first() {
        let blocks_ago = current_height.saturating_sub(latest.block_height);
        let secs_ago = blocks_ago * 60; // ~60s per block
        let ago_str = format_duration(secs_ago);
        Some((format!("{ago_str} ago"), secs_ago))
    } else {
        None
    };

    // Determine health status
    let health = match &last_stake_info {
        None => "No data",
        Some((_, elapsed)) => {
            if expected_secs.is_infinite() {
                "No data"
            } else if (*elapsed as f64) < expected_secs * 2.0 {
                "Healthy"
            } else {
                "Overdue"
            }
        }
    };

    let expected_str = if expected_secs.is_infinite() {
        "N/A (zero balance)".to_string()
    } else {
        format_duration(expected_secs as u64)
    };

    let last_stake_str = match &last_stake_info {
        Some((ago, _)) => ago.clone(),
        None => "Never".to_string(),
    };

    let label = watch
        .as_ref()
        .and_then(|w| w.label.as_ref())
        .filter(|l| !l.is_empty())
        .map(|l| format!(" ({l})"))
        .unwrap_or_default();

    let vault_indicator = if is_vault { " (vault)" } else { "" };

    // For vault addresses, show "Total rewards earned" from DB instead of
    // "Total received" which is inflated by recycled UTXO values.
    let received_line = if is_vault {
        let total_rewards = db::sum_stake_rewards(&state.db, &address).unwrap_or(0);
        format!(
            "<b>Total rewards earned:</b> {} DIVI",
            satoshi_to_divi(total_rewards)
        )
    } else {
        format!(
            "<b>Total received:</b> {} DIVI",
            satoshi_to_divi(balance.received)
        )
    };

    Ok(format!(
        "<b>Staking Analysis</b>\n\
         <code>{address}</code>{label}\n\n\
         <b>Balance:</b> {} DIVI{vault_indicator}\n\
         {received_line}\n\n\
         <b>Stakes (24h / 7d / 30d):</b> {stakes_24h} / {stakes_7d} / {stakes_30d}\n\
         <b>Avg stake amount:</b> {} DIVI\n\n\
         <b>Expected frequency:</b> {expected_str}\n\
         <b>Last stake:</b> {last_stake_str}\n\
         <b>Health:</b> {health}",
        satoshi_to_divi(balance.balance),
        satoshi_to_divi(avg_amount),
    ))
}

// ---------------------------------------------------------------------------
// /status
// ---------------------------------------------------------------------------

async fn handle_status(state: &BotState) -> Result<String> {
    let uptime_secs = state.start_time.elapsed().as_secs();
    let block_height = state.last_block_height.load(Ordering::Relaxed);
    let connected = state.monitor_connected.load(Ordering::Relaxed);

    let watch_count = db::count_watches(&state.db)?;
    let user_count = db::count_users(&state.db)?;

    let connection_status = if connected {
        "Connected"
    } else {
        "Disconnected"
    };

    Ok(format!(
        "<b>Stake Watch Status</b>\n\n\
         <b>Uptime:</b> {}\n\
         <b>Block height:</b> {}\n\
         <b>Watched addresses:</b> {}\n\
         <b>Users:</b> {}\n\
         <b>Backend:</b> {}\n\
         <b>Connection:</b> {}",
        format_duration(uptime_secs),
        if block_height == 0 {
            "unknown".to_string()
        } else {
            block_height.to_string()
        },
        watch_count,
        user_count,
        state.config.backend.backend_type,
        connection_status,
    ))
}

// ---------------------------------------------------------------------------
// /alerts
// ---------------------------------------------------------------------------

async fn handle_alerts(state: &BotState, telegram_id: i64) -> Result<String> {
    let subscriptions = db::get_subscriptions_for_user(&state.db, telegram_id)?;

    let mut text = String::from("<b>Alert Subscriptions</b>\n\n");

    if subscriptions.is_empty() {
        text.push_str("You have no active alert subscriptions.\n\n");
    } else {
        text.push_str("<b>Active:</b>\n");
        for sub in &subscriptions {
            text.push_str(&format!(
                "  - <b>{}</b> (threshold: {})\n",
                sub.alert_type, sub.threshold
            ));
        }
        text.push('\n');
    }

    text.push_str("<b>Available alert types:</b>\n");
    for &alert_type in crate::alert_analyzer::VALID_ALERT_TYPES {
        let default = crate::alert_analyzer::default_threshold_for(alert_type);
        text.push_str(&format!("  <b>{alert_type}</b> (default: {default})\n"));
    }
    text.push_str("\nUsage: /alert &lt;type&gt; [threshold]");

    Ok(text)
}

// ---------------------------------------------------------------------------
// /alert <type> [threshold]
// ---------------------------------------------------------------------------

async fn handle_alert(state: &BotState, telegram_id: i64, arg: &str) -> Result<String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Ok("Usage: /alert &lt;type&gt; [threshold]\n\n\
             Example: /alert large_tx 500000"
            .to_string());
    }

    let mut parts = arg.splitn(2, ' ');
    let alert_type = parts.next().unwrap().trim().to_lowercase();
    let threshold: f64 = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| crate::alert_analyzer::default_threshold_for(&alert_type));

    if !crate::alert_analyzer::VALID_ALERT_TYPES.contains(&alert_type.as_str()) {
        return Ok(format!(
            "Unknown alert type: <b>{alert_type}</b>\n\n\
             Valid types: {}",
            crate::alert_analyzer::VALID_ALERT_TYPES.join(", ")
        ));
    }

    db::add_alert_subscription(&state.db, telegram_id, &alert_type, threshold)?;
    info!(telegram_id, alert_type = %alert_type, threshold, "Alert subscription added");

    Ok(format!(
        "Subscribed to <b>{alert_type}</b> alerts (threshold: {threshold})."
    ))
}

// ---------------------------------------------------------------------------
// /unalert <type>
// ---------------------------------------------------------------------------

async fn handle_unalert(state: &BotState, telegram_id: i64, arg: &str) -> Result<String> {
    let alert_type = arg.trim().to_lowercase();
    if alert_type.is_empty() {
        return Ok("Usage: /unalert &lt;type&gt;".to_string());
    }

    let removed = db::remove_alert_subscription(&state.db, telegram_id, &alert_type)?;
    if removed {
        info!(telegram_id, alert_type = %alert_type, "Alert subscription removed");
        Ok(format!("Unsubscribed from <b>{alert_type}</b> alerts."))
    } else {
        Ok(format!(
            "You are not subscribed to <b>{alert_type}</b> alerts."
        ))
    }
}

// ---------------------------------------------------------------------------
// /forkwatch
// ---------------------------------------------------------------------------

async fn handle_fork_watch(state: &BotState, telegram_id: i64) -> Result<String> {
    db::add_fork_watcher(&state.db, telegram_id)?;
    info!(telegram_id, "Subscribed to fork alerts");
    Ok("You are now subscribed to fork detection alerts.\n\n\
         You will be notified if the bot detects a blockchain fork."
        .to_string())
}

// ---------------------------------------------------------------------------
// /forkunwatch
// ---------------------------------------------------------------------------

async fn handle_fork_unwatch(state: &BotState, telegram_id: i64) -> Result<String> {
    let removed = db::remove_fork_watcher(&state.db, telegram_id)?;
    if removed {
        info!(telegram_id, "Unsubscribed from fork alerts");
        Ok("Unsubscribed from fork detection alerts.".to_string())
    } else {
        Ok("You are not subscribed to fork detection alerts.".to_string())
    }
}

// ---------------------------------------------------------------------------
// /forkstatus
// ---------------------------------------------------------------------------

async fn handle_fork_status(state: &BotState) -> Result<String> {
    let config_endpoints = &state.config.fork_detection.endpoints;
    let db_endpoints = db::get_fork_endpoints(&state.db)?;

    if config_endpoints.is_empty() && db_endpoints.is_empty() {
        return Ok("<b>Fork Detection</b>\n\n\
             No fork monitoring endpoints configured.\n\
             Admins can add endpoints with /addfork."
            .to_string());
    }

    let mut text = String::from("<b>Fork Detection Status</b>\n\n");

    if !state.config.fork_detection.enabled {
        text.push_str("Status: <b>Disabled</b>\n\n");
    } else {
        text.push_str("Status: <b>Enabled</b>\n");
        text.push_str(&format!(
            "Check interval: {}s\n\n",
            state.config.fork_detection.check_interval_secs
        ));
    }

    text.push_str("<b>Endpoints:</b>\n");

    // Query each endpoint for current height
    for ep in config_endpoints {
        let height_info = match query_endpoint_height(&ep.rpc_url).await {
            Ok(h) => format!("height {h}"),
            Err(_) => "unreachable".to_string(),
        };
        text.push_str(&format!("  {} - {} (config)\n", ep.name, height_info));
    }

    for ep in &db_endpoints {
        let height_info = match query_endpoint_height(&ep.rpc_url).await {
            Ok(h) => format!("height {h}"),
            Err(_) => "unreachable".to_string(),
        };
        text.push_str(&format!("  {} - {} (user-added)\n", ep.name, height_info));
    }

    let watcher_count = db::count_fork_watchers(&state.db)?;
    text.push_str(&format!("\nSubscribed users: {watcher_count}"));

    Ok(text)
}

// ---------------------------------------------------------------------------
// /addfork <name> <rpc_url>  (admin only)
// ---------------------------------------------------------------------------

async fn handle_add_fork(state: &BotState, telegram_id: i64, arg: &str) -> Result<String> {
    if !state.is_admin(telegram_id) {
        return Ok("This command is restricted to bot administrators.".to_string());
    }

    let arg = arg.trim();
    let mut parts = arg.splitn(2, ' ');
    let name = match parts.next() {
        Some(n) if !n.is_empty() => n.trim(),
        _ => return Ok("Usage: /addfork &lt;name&gt; &lt;rpc_url&gt;".to_string()),
    };
    let rpc_url = match parts.next() {
        Some(u) if !u.is_empty() => u.trim(),
        _ => return Ok("Usage: /addfork &lt;name&gt; &lt;rpc_url&gt;".to_string()),
    };

    // Quick connectivity check
    match query_endpoint_height(rpc_url).await {
        Ok(height) => {
            db::add_fork_endpoint(&state.db, name, rpc_url, Some(telegram_id))?;
            info!(name, rpc_url, added_by = telegram_id, "Fork endpoint added");
            Ok(format!(
                "Fork endpoint <b>{name}</b> added.\n\
                 URL: <code>{rpc_url}</code>\n\
                 Current height: {height}"
            ))
        }
        Err(e) => Ok(format!(
            "Could not connect to endpoint: {e}\n\
             Please check the URL and try again."
        )),
    }
}

// ---------------------------------------------------------------------------
// /removefork <name>  (admin only)
// ---------------------------------------------------------------------------

async fn handle_remove_fork(state: &BotState, telegram_id: i64, arg: &str) -> Result<String> {
    if !state.is_admin(telegram_id) {
        return Ok("This command is restricted to bot administrators.".to_string());
    }

    let name = arg.trim();
    if name.is_empty() {
        return Ok("Usage: /removefork &lt;name&gt;".to_string());
    }

    let removed = db::remove_fork_endpoint(&state.db, name)?;
    if removed {
        info!(name, removed_by = telegram_id, "Fork endpoint removed");
        Ok(format!("Fork endpoint <b>{name}</b> removed."))
    } else {
        Ok(format!(
            "No fork endpoint named <b>{name}</b> found.\n\
             Note: Config-defined endpoints cannot be removed via command."
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Query a single RPC endpoint for its block count. Used by /forkstatus and
/// /addfork to verify connectivity.
async fn query_endpoint_height(rpc_url: &str) -> Result<u64> {
    use crate::rpc::JsonRpcClient;
    let client = JsonRpcClient::new(rpc_url.to_string(), None, None);
    client.get_block_count().await
}
