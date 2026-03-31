use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        Json,
    },
    routing::{delete, get},
    Router,
};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::{info, warn};

use super::{auth, WebAppState};
use crate::{db, stake_analyzer::StakeAnalyzer, utils};

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(state: Arc<WebAppState>) -> Router {
    Router::new()
        // Authenticated user endpoints
        .route("/me", get(get_me))
        .route("/watches", get(get_watches).post(add_watch))
        .route("/watches/{address}", delete(remove_watch))
        .route("/watches/{address}/analysis", get(get_analysis))
        .route("/watches/{address}/stakes", get(get_stakes))
        .route("/alerts", get(get_alerts).post(add_alert))
        .route("/alerts/{alert_type}", delete(remove_alert))
        // Public explorer endpoints
        .route("/blocks", get(get_blocks))
        .route("/blocks/{hash}", get(get_block))
        .route("/tx/{txid}", get(get_tx))
        .route("/address/{address}", get(get_address))
        .route("/address/{address}/vault", get(get_vault_balance))
        .route("/search", get(search))
        .route("/network", get(get_network))
        .route("/feed", get(sse_feed))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_telegram_user(
    headers: &HeaderMap,
    secrets: &crate::config::Secrets,
) -> Option<auth::TelegramUser> {
    let init_data = headers.get("X-Telegram-Init-Data")?.to_str().ok()?;
    auth::validate_init_data(init_data, &secrets.telegram_bot_token)
}

fn unauthorized() -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiError {
            error: "Missing or invalid Telegram initData".to_string(),
        }),
    )
}

fn internal_error(msg: impl std::fmt::Display) -> (StatusCode, Json<ApiError>) {
    warn!("Internal error: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            error: format!("{msg}"),
        }),
    )
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError { error: msg.into() }),
    )
}

// ---------------------------------------------------------------------------
// Shared response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ApiError {
    error: String,
}

#[derive(Serialize)]
struct MeResponse {
    id: i64,
    first_name: String,
    username: Option<String>,
    watch_count: u32,
    max_watches: u32,
}

#[derive(Serialize)]
struct WatchResponse {
    address: String,
    label: Option<String>,
    added_at: String,
    last_stake_at: Option<String>,
    last_stake_ago: Option<String>,
    balance_divi: Option<String>,
    vault_balance_divi: Option<String>,
}

#[derive(Deserialize)]
struct AddWatchRequest {
    address: String,
    label: Option<String>,
}

#[derive(Serialize)]
struct AnalysisResponse {
    address: String,
    label: Option<String>,
    balance_divi: String,
    is_vault: bool,
    total_received_divi: String,
    stakes_24h: usize,
    stakes_7d: usize,
    stakes_30d: usize,
    avg_stake_divi: String,
    expected_frequency: String,
    expected_frequency_secs: Option<f64>,
    last_stake: String,
    health: String,
    explorer_url: String,
}

#[derive(Serialize)]
struct StakeResponse {
    txid: String,
    block_height: u64,
    block_hash: String,
    amount_divi: String,
    event_type: String,
    detected_at: String,
    explorer_url: String,
}

#[derive(Serialize)]
struct AlertResponse {
    alert_type: String,
    threshold: f64,
    created_at: String,
}

#[derive(Deserialize)]
struct AddAlertRequest {
    alert_type: String,
    threshold: Option<f64>,
}

#[derive(Serialize)]
struct BlockSummary {
    hash: String,
    height: u64,
    time: u64,
    tx_count: usize,
    size: u64,
}

#[derive(Serialize)]
struct BlockDetail {
    hash: String,
    height: u64,
    time: u64,
    size: u64,
    tx_count: usize,
    transactions: Vec<TxSummary>,
}

#[derive(Serialize)]
struct TxSummary {
    txid: String,
    vin_count: usize,
    vout_count: usize,
    total_output_divi: String,
}

#[derive(Serialize)]
struct TxDetail {
    txid: String,
    blockhash: Option<String>,
    vin: Vec<VinDetail>,
    vout: Vec<VoutDetail>,
}

#[derive(Serialize)]
struct VinDetail {
    coinbase: Option<String>,
    txid: Option<String>,
    vout: Option<u32>,
    value: Option<f64>,
}

#[derive(Serialize)]
struct VoutDetail {
    value_divi: String,
    n: u32,
    script_type: Option<String>,
    addresses: Option<Vec<String>>,
}

#[derive(Serialize)]
struct AddressInfo {
    address: String,
    balance_divi: String,
    received_divi: String,
    vault_balance_divi: Option<String>,
    recent_deltas: Vec<DeltaInfo>,
}

#[derive(Serialize)]
struct DeltaInfo {
    txid: String,
    height: u64,
    amount_divi: String,
}

#[derive(Serialize)]
struct VaultInfo {
    address: String,
    vault_balance_divi: String,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Serialize)]
struct SearchResult {
    #[serde(rename = "type")]
    result_type: String,
    value: String,
}

#[derive(Serialize)]
struct NetworkInfo {
    block_count: u64,
    explorer_url: String,
}

#[derive(Deserialize)]
struct BlocksQuery {
    limit: Option<u32>,
}

#[derive(Serialize)]
struct SuccessResponse {
    ok: bool,
    message: String,
}

// ---------------------------------------------------------------------------
// GET /api/me
// ---------------------------------------------------------------------------

async fn get_me(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    // Ensure user exists in DB
    let _ = db::add_user(&state.db, user.id, user.username.as_deref());

    let watch_count = db::get_watch_count_for_user(&state.db, user.id).map_err(internal_error)?;

    Ok(Json(MeResponse {
        id: user.id,
        first_name: user.first_name,
        username: user.username,
        watch_count,
        max_watches: state.config.general.max_watches_per_user,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/watches
// ---------------------------------------------------------------------------

async fn get_watches(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<WatchResponse>>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    let watches = db::get_watches_for_user(&state.db, user.id).map_err(internal_error)?;

    let mut result = Vec::with_capacity(watches.len());
    for w in &watches {
        // Try to fetch balances (best-effort)
        let balance_divi = match state.rpc.get_address_balance(&w.address).await {
            Ok(b) => Some(utils::satoshi_to_divi(b.balance)),
            Err(_) => None,
        };

        let vault_balance_divi = match state.rpc.get_vault_balance(&w.address).await {
            Ok(b) if b.balance > 0 => Some(utils::satoshi_to_divi(b.balance)),
            _ => None,
        };

        let last_stake_ago = w.last_stake_at.as_ref().map(|ts| utils::time_ago(ts));

        result.push(WatchResponse {
            address: w.address.clone(),
            label: w.label.clone(),
            added_at: w.added_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            last_stake_at: w
                .last_stake_at
                .map(|ts| ts.format("%Y-%m-%d %H:%M:%S").to_string()),
            last_stake_ago,
            balance_divi,
            vault_balance_divi,
        });
    }

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// POST /api/watches
// ---------------------------------------------------------------------------

async fn add_watch(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
    Json(body): Json<AddWatchRequest>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    let address = body.address.trim();
    if address.is_empty() {
        return Err(bad_request("address is required"));
    }

    // Validate address format
    if !address.starts_with('D') && !address.starts_with('y') {
        return Err(bad_request(
            "Invalid address: must start with 'D' (mainnet) or 'y' (testnet)",
        ));
    }

    // RPC validation
    match state.rpc.validate_address(address).await {
        Ok(v) if !v.isvalid => {
            return Err(bad_request("Invalid blockchain address"));
        }
        Err(e) => {
            warn!(address, error = %e, "Address validation RPC failed, proceeding anyway");
        }
        _ => {}
    }

    // Check limit
    let count = db::get_watch_count_for_user(&state.db, user.id).map_err(internal_error)?;
    if count >= state.config.general.max_watches_per_user {
        return Err(bad_request(format!(
            "Maximum of {} watches reached",
            state.config.general.max_watches_per_user
        )));
    }

    let label = body.label.as_deref().filter(|s| !s.is_empty());
    let added = db::add_watch(&state.db, user.id, address, label).map_err(internal_error)?;

    if !added {
        return Err(bad_request("You are already watching this address"));
    }

    info!(
        telegram_id = user.id,
        address,
        ?label,
        "Web: address watch added"
    );

    // Spawn background backfill
    let rpc = Arc::clone(&state.rpc);
    let db = state.db.clone();
    let addr = address.to_string();
    tokio::spawn(async move {
        if let Err(e) = StakeAnalyzer::backfill_stakes(&rpc, &db, &addr).await {
            warn!(address = %addr, error = %e, "Backfill failed");
        }
    });

    Ok(Json(SuccessResponse {
        ok: true,
        message: format!("Now watching {address}"),
    }))
}

// ---------------------------------------------------------------------------
// DELETE /api/watches/:address
// ---------------------------------------------------------------------------

async fn remove_watch(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
    Path(address): Path<String>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    let removed = db::remove_watch(&state.db, user.id, &address).map_err(internal_error)?;

    if !removed {
        return Err(bad_request("You are not watching this address"));
    }

    info!(telegram_id = user.id, address = %address, "Web: address watch removed");

    Ok(Json(SuccessResponse {
        ok: true,
        message: format!("Stopped watching {address}"),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/watches/:address/analysis
// ---------------------------------------------------------------------------

async fn get_analysis(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
    Path(address): Path<String>,
) -> Result<Json<AnalysisResponse>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    // Fetch balance -- try regular first, fall back to vault
    let (balance, is_vault) = {
        let regular = state
            .rpc
            .get_address_balance(&address)
            .await
            .map_err(internal_error)?;

        if regular.balance > 0 {
            (regular, false)
        } else {
            match state.rpc.get_vault_balance(&address).await {
                Ok(vb) if vb.balance > 0 => (vb, true),
                _ => (regular, false),
            }
        }
    };

    let stakes = db::get_recent_stakes(&state.db, &address, 1000).map_err(internal_error)?;
    let current_height = state.rpc.get_block_count().await.unwrap_or(0);

    let blocks_24h = 24 * 60u64;
    let blocks_7d = 7 * 24 * 60u64;
    let blocks_30d = 30 * 24 * 60u64;

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

    let expected_secs = StakeAnalyzer::compute_expected_interval(
        balance.balance,
        state.config.general.network_staking_supply,
    );

    let last_stake_info = if let Some(latest) = stakes.first() {
        let blocks_ago = current_height.saturating_sub(latest.block_height);
        let secs_ago = blocks_ago * 60;
        Some((utils::format_duration(secs_ago) + " ago", secs_ago))
    } else {
        None
    };

    let health = match &last_stake_info {
        None => "No data".to_string(),
        Some((_, elapsed)) => {
            if expected_secs.is_infinite() {
                "No data".to_string()
            } else if (*elapsed as f64) < expected_secs * 2.0 {
                "Healthy".to_string()
            } else {
                "Overdue".to_string()
            }
        }
    };

    let expected_str = if expected_secs.is_infinite() {
        "N/A (zero balance)".to_string()
    } else {
        utils::format_duration(expected_secs as u64)
    };

    let last_stake_str = match &last_stake_info {
        Some((ago, _)) => ago.clone(),
        None => "Never".to_string(),
    };

    let label = db::get_watch_label(&state.db, user.id, &address).unwrap_or(None);

    let total_received = if is_vault {
        let total_rewards = db::sum_stake_rewards(&state.db, &address).unwrap_or(0);
        utils::satoshi_to_divi(total_rewards)
    } else {
        utils::satoshi_to_divi(balance.received)
    };

    Ok(Json(AnalysisResponse {
        address: address.clone(),
        label,
        balance_divi: utils::satoshi_to_divi(balance.balance),
        is_vault,
        total_received_divi: total_received,
        stakes_24h,
        stakes_7d,
        stakes_30d,
        avg_stake_divi: utils::satoshi_to_divi(avg_amount),
        expected_frequency: expected_str,
        expected_frequency_secs: if expected_secs.is_infinite() {
            None
        } else {
            Some(expected_secs)
        },
        last_stake: last_stake_str,
        health,
        explorer_url: state.explorer_url.clone(),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/watches/:address/stakes
// ---------------------------------------------------------------------------

async fn get_stakes(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
    Path(address): Path<String>,
) -> Result<Json<Vec<StakeResponse>>, (StatusCode, Json<ApiError>)> {
    let _user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    let stakes = db::get_recent_stakes(&state.db, &address, 100).map_err(internal_error)?;

    let result: Vec<StakeResponse> = stakes
        .into_iter()
        .map(|s| StakeResponse {
            txid: s.txid.clone(),
            block_height: s.block_height,
            block_hash: s.block_hash,
            amount_divi: utils::satoshi_to_divi(s.amount_satoshis),
            event_type: s.event_type,
            detected_at: s.detected_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            explorer_url: format!("{}/tx/{}", state.explorer_url, s.txid),
        })
        .collect();

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// GET /api/alerts
// ---------------------------------------------------------------------------

async fn get_alerts(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AlertResponse>>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    let subs = db::get_subscriptions_for_user(&state.db, user.id).map_err(internal_error)?;

    let result: Vec<AlertResponse> = subs
        .into_iter()
        .map(|s| AlertResponse {
            alert_type: s.alert_type,
            threshold: s.threshold,
            created_at: s.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// POST /api/alerts
// ---------------------------------------------------------------------------

async fn add_alert(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
    Json(body): Json<AddAlertRequest>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    let alert_type = body.alert_type.trim().to_lowercase();
    if !crate::alert_analyzer::VALID_ALERT_TYPES.contains(&alert_type.as_str()) {
        return Err(bad_request(format!(
            "Unknown alert type: {alert_type}. Valid: {}",
            crate::alert_analyzer::VALID_ALERT_TYPES.join(", ")
        )));
    }

    let threshold = body
        .threshold
        .unwrap_or_else(|| crate::alert_analyzer::default_threshold_for(&alert_type));

    db::add_alert_subscription(&state.db, user.id, &alert_type, threshold)
        .map_err(internal_error)?;

    info!(telegram_id = user.id, alert_type = %alert_type, threshold, "Web: alert subscription added");

    Ok(Json(SuccessResponse {
        ok: true,
        message: format!("Subscribed to {alert_type} (threshold: {threshold})"),
    }))
}

// ---------------------------------------------------------------------------
// DELETE /api/alerts/:alert_type
// ---------------------------------------------------------------------------

async fn remove_alert(
    State(state): State<Arc<WebAppState>>,
    headers: HeaderMap,
    Path(alert_type): Path<String>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ApiError>)> {
    let user = get_telegram_user(&headers, &state.secrets).ok_or_else(unauthorized)?;

    let removed =
        db::remove_alert_subscription(&state.db, user.id, &alert_type).map_err(internal_error)?;

    if !removed {
        return Err(bad_request(format!(
            "You are not subscribed to {alert_type}"
        )));
    }

    Ok(Json(SuccessResponse {
        ok: true,
        message: format!("Unsubscribed from {alert_type}"),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/blocks?limit=20
// ---------------------------------------------------------------------------

async fn get_blocks(
    State(state): State<Arc<WebAppState>>,
    Query(params): Query<BlocksQuery>,
) -> Result<Json<Vec<BlockSummary>>, (StatusCode, Json<ApiError>)> {
    let limit = params.limit.unwrap_or(20).min(50);

    let height = state.rpc.get_block_count().await.map_err(internal_error)?;

    let mut blocks = Vec::with_capacity(limit as usize);
    for h in (height.saturating_sub(limit as u64 - 1)..=height).rev() {
        let hash = match state.rpc.get_block_hash(h).await {
            Ok(hash) => hash,
            Err(_) => continue,
        };
        let block = match state.rpc.get_block(&hash).await {
            Ok(b) => b,
            Err(_) => continue,
        };
        blocks.push(BlockSummary {
            hash: block.hash,
            height: block.height,
            time: block.time,
            tx_count: block.tx.len(),
            size: block.size,
        });
    }

    Ok(Json(blocks))
}

// ---------------------------------------------------------------------------
// GET /api/blocks/:hash
// ---------------------------------------------------------------------------

async fn get_block(
    State(state): State<Arc<WebAppState>>,
    Path(hash): Path<String>,
) -> Result<Json<BlockDetail>, (StatusCode, Json<ApiError>)> {
    let block = state.rpc.get_block(&hash).await.map_err(internal_error)?;

    let mut transactions = Vec::with_capacity(block.tx.len());
    for txid in &block.tx {
        match state.rpc.get_raw_transaction(txid).await {
            Ok(tx) => {
                let total_output: f64 = tx.vout.iter().map(|v| v.value).sum();
                transactions.push(TxSummary {
                    txid: tx.txid,
                    vin_count: tx.vin.len(),
                    vout_count: tx.vout.len(),
                    total_output_divi: format!("{total_output:.8}"),
                });
            }
            Err(_) => {
                transactions.push(TxSummary {
                    txid: txid.clone(),
                    vin_count: 0,
                    vout_count: 0,
                    total_output_divi: "0.00000000".to_string(),
                });
            }
        }
    }

    Ok(Json(BlockDetail {
        hash: block.hash,
        height: block.height,
        time: block.time,
        size: block.size,
        tx_count: block.tx.len(),
        transactions,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/tx/:txid
// ---------------------------------------------------------------------------

async fn get_tx(
    State(state): State<Arc<WebAppState>>,
    Path(txid): Path<String>,
) -> Result<Json<TxDetail>, (StatusCode, Json<ApiError>)> {
    let tx = state
        .rpc
        .get_raw_transaction(&txid)
        .await
        .map_err(internal_error)?;

    let vin: Vec<VinDetail> = tx
        .vin
        .iter()
        .map(|v| VinDetail {
            coinbase: v.coinbase.clone(),
            txid: v.txid.clone(),
            vout: v.vout,
            value: v.value,
        })
        .collect();

    let vout: Vec<VoutDetail> = tx
        .vout
        .iter()
        .map(|v| VoutDetail {
            value_divi: format!("{:.8}", v.value),
            n: v.n,
            script_type: v.script_pub_key.script_type.clone(),
            addresses: v.script_pub_key.addresses.clone(),
        })
        .collect();

    Ok(Json(TxDetail {
        txid: tx.txid,
        blockhash: tx.blockhash,
        vin,
        vout,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/address/:address
// ---------------------------------------------------------------------------

async fn get_address(
    State(state): State<Arc<WebAppState>>,
    Path(address): Path<String>,
) -> Result<Json<AddressInfo>, (StatusCode, Json<ApiError>)> {
    let balance = state
        .rpc
        .get_address_balance(&address)
        .await
        .map_err(internal_error)?;

    let vault_balance_divi = match state.rpc.get_vault_balance(&address).await {
        Ok(vb) if vb.balance > 0 => Some(utils::satoshi_to_divi(vb.balance)),
        _ => None,
    };

    // Fetch recent deltas (best-effort)
    let recent_deltas = match state.rpc.get_address_deltas(&address, None, None).await {
        Ok(deltas) => deltas
            .into_iter()
            .rev()
            .take(20)
            .map(|d| DeltaInfo {
                txid: d.txid,
                height: d.height,
                amount_divi: utils::satoshi_to_divi(d.satoshis),
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    Ok(Json(AddressInfo {
        address,
        balance_divi: utils::satoshi_to_divi(balance.balance),
        received_divi: utils::satoshi_to_divi(balance.received),
        vault_balance_divi,
        recent_deltas,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/address/:address/vault
// ---------------------------------------------------------------------------

async fn get_vault_balance(
    State(state): State<Arc<WebAppState>>,
    Path(address): Path<String>,
) -> Result<Json<VaultInfo>, (StatusCode, Json<ApiError>)> {
    let vb = state
        .rpc
        .get_vault_balance(&address)
        .await
        .map_err(internal_error)?;

    Ok(Json(VaultInfo {
        address,
        vault_balance_divi: utils::satoshi_to_divi(vb.balance),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/search?q=...
// ---------------------------------------------------------------------------

async fn search(
    State(state): State<Arc<WebAppState>>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchResult>, (StatusCode, Json<ApiError>)> {
    let q = params.q.trim();
    if q.is_empty() {
        return Err(bad_request("Query parameter 'q' is required"));
    }

    // Auto-detect type:
    // - pure digits => block height
    // - 64 hex chars => block hash or txid
    // - starts with D or y => address

    if q.chars().all(|c| c.is_ascii_digit()) {
        // Block height
        let height: u64 = q.parse().map_err(|_| bad_request("Invalid block height"))?;
        let hash = state
            .rpc
            .get_block_hash(height)
            .await
            .map_err(internal_error)?;
        return Ok(Json(SearchResult {
            result_type: "block".to_string(),
            value: hash,
        }));
    }

    if q.len() == 64 && q.chars().all(|c| c.is_ascii_hexdigit()) {
        // Could be block hash or txid -- try block first
        match state.rpc.get_block(q).await {
            Ok(_) => {
                return Ok(Json(SearchResult {
                    result_type: "block".to_string(),
                    value: q.to_string(),
                }));
            }
            Err(_) => {
                // Try as txid
                match state.rpc.get_raw_transaction(q).await {
                    Ok(_) => {
                        return Ok(Json(SearchResult {
                            result_type: "tx".to_string(),
                            value: q.to_string(),
                        }));
                    }
                    Err(_) => {
                        return Err(bad_request("Hash not found as block or transaction"));
                    }
                }
            }
        }
    }

    if q.starts_with('D') || q.starts_with('y') {
        // Address
        match state.rpc.validate_address(q).await {
            Ok(v) if v.isvalid => {
                return Ok(Json(SearchResult {
                    result_type: "address".to_string(),
                    value: q.to_string(),
                }));
            }
            _ => {
                return Err(bad_request("Invalid address"));
            }
        }
    }

    Err(bad_request(
        "Unrecognized query format. Use a block height, hash, txid, or address.",
    ))
}

// ---------------------------------------------------------------------------
// GET /api/network
// ---------------------------------------------------------------------------

async fn get_network(
    State(state): State<Arc<WebAppState>>,
) -> Result<Json<NetworkInfo>, (StatusCode, Json<ApiError>)> {
    let block_count = state.rpc.get_block_count().await.map_err(internal_error)?;

    Ok(Json(NetworkInfo {
        block_count,
        explorer_url: state.explorer_url.clone(),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/feed  (SSE)
// ---------------------------------------------------------------------------

async fn sse_feed(
    State(state): State<Arc<WebAppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<ApiError>)> {
    let tx = state
        .block_tx
        .as_ref()
        .ok_or_else(|| bad_request("Block feed not available"))?;

    let rx = tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(hash) => Some(Ok(Event::default().event("block").data(hash))),
        Err(_) => None, // lagged — skip
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
