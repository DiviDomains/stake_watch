use axum::extract::{Path, Query, State};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use super::db::{self, VaultDb};
use crate::utils::satoshi_to_divi;

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub db: VaultDb,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route(
            "/api/vault/balance/{address}",
            axum::routing::get(get_balance),
        )
        .route(
            "/api/vault/stakes/{address}",
            axum::routing::get(get_stakes),
        )
        .route("/api/vault/utxos/{address}", axum::routing::get(get_utxos))
        .route("/api/vault/stats", axum::routing::get(get_stats))
        .route("/health", axum::routing::get(health))
        .layer(cors)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct BalanceResponse {
    address: String,
    balance: i64,
    balance_divi: String,
    utxo_count: u32,
}

#[derive(Debug, Deserialize)]
struct StakesQuery {
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct StakeEntry {
    txid: String,
    block_height: u64,
    value_divi: String,
    reward_divi: String,
}

#[derive(Debug, Serialize)]
struct StakesResponse {
    address: String,
    stakes: Vec<StakeEntry>,
}

#[derive(Debug, Serialize)]
struct UtxoEntry {
    txid: String,
    vout_n: u32,
    owner_address: String,
    manager_address: Option<String>,
    value_satoshis: i64,
    value_divi: String,
    block_height: u64,
    block_hash: Option<String>,
}

#[derive(Debug, Serialize)]
struct UtxosResponse {
    address: String,
    utxos: Vec<UtxoEntry>,
}

#[derive(Debug, Serialize)]
struct StatsResponse {
    total_utxos: u64,
    total_unspent: u64,
    total_addresses: u64,
    total_value_satoshis: i64,
    total_value_divi: String,
    last_scanned_height: u64,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn get_balance(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Json<BalanceResponse> {
    let balance = db::get_balance(&state.db, &address).unwrap_or(db::VaultBalance {
        balance_satoshis: 0,
        utxo_count: 0,
    });

    Json(BalanceResponse {
        address,
        balance: balance.balance_satoshis,
        balance_divi: satoshi_to_divi(balance.balance_satoshis),
        utxo_count: balance.utxo_count,
    })
}

async fn get_stakes(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Query(query): Query<StakesQuery>,
) -> Json<StakesResponse> {
    let limit = query.limit.unwrap_or(20);
    let utxos = db::get_stake_history(&state.db, &address, limit).unwrap_or_default();

    // Compute rewards as the difference between consecutive UTXO values.
    // Stakes are ordered by block_height DESC, so each stake's reward is
    // its value minus the next (older) stake's value.
    let mut stakes: Vec<StakeEntry> = Vec::with_capacity(utxos.len());
    for (i, utxo) in utxos.iter().enumerate() {
        let reward = if i + 1 < utxos.len() {
            utxo.value_satoshis - utxos[i + 1].value_satoshis
        } else {
            0
        };
        stakes.push(StakeEntry {
            txid: utxo.txid.clone(),
            block_height: utxo.block_height,
            value_divi: satoshi_to_divi(utxo.value_satoshis),
            reward_divi: satoshi_to_divi(reward),
        });
    }

    Json(StakesResponse { address, stakes })
}

async fn get_utxos(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Json<UtxosResponse> {
    let utxos = db::get_unspent_utxos(&state.db, &address).unwrap_or_default();

    let entries: Vec<UtxoEntry> = utxos
        .into_iter()
        .map(|u| UtxoEntry {
            txid: u.txid,
            vout_n: u.vout_n,
            owner_address: u.owner_address,
            manager_address: u.manager_address,
            value_satoshis: u.value_satoshis,
            value_divi: satoshi_to_divi(u.value_satoshis),
            block_height: u.block_height,
            block_hash: u.block_hash,
        })
        .collect();

    Json(UtxosResponse {
        address,
        utxos: entries,
    })
}

async fn get_stats(State(state): State<Arc<AppState>>) -> Json<StatsResponse> {
    let stats = db::get_stats(&state.db).unwrap_or(db::IndexerStats {
        total_utxos: 0,
        total_unspent: 0,
        total_addresses: 0,
        total_value_satoshis: 0,
        last_scanned_height: 0,
    });

    Json(StatsResponse {
        total_utxos: stats.total_utxos,
        total_unspent: stats.total_unspent,
        total_addresses: stats.total_addresses,
        total_value_satoshis: stats.total_value_satoshis,
        total_value_divi: satoshi_to_divi(stats.total_value_satoshis),
        last_scanned_height: stats.last_scanned_height,
    })
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}
