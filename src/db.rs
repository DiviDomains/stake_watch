use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tracing::info;

/// Thread-safe database connection handle.
pub type DbPool = Arc<Mutex<Connection>>;

// ---------------------------------------------------------------------------
// Result structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WatchedAddress {
    pub id: i64,
    pub telegram_id: i64,
    pub address: String,
    pub label: Option<String>,
    pub added_at: NaiveDateTime,
    pub last_stake_at: Option<NaiveDateTime>,
    pub last_stake_height: Option<u64>,
    pub last_alert_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct StakeEvent {
    pub id: i64,
    pub address: String,
    pub txid: String,
    pub block_height: u64,
    pub block_hash: String,
    pub amount_satoshis: i64,
    pub event_type: String,
    pub detected_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct AlertSubscription {
    pub id: i64,
    pub telegram_id: i64,
    pub alert_type: String,
    pub threshold: f64,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct ForkEndpoint {
    pub id: i64,
    pub name: String,
    pub rpc_url: String,
    pub added_by: Option<i64>,
    pub added_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct ForkEvent {
    pub id: i64,
    pub height: u64,
    pub endpoint_a: String,
    pub hash_a: String,
    pub endpoint_b: String,
    pub hash_b: String,
    pub detected_at: NaiveDateTime,
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Open (or create) the SQLite database and run migrations.
pub fn init_db(path: &str) -> Result<DbPool> {
    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating db directory: {}", parent.display()))?;
    }

    let conn = Connection::open(path).with_context(|| format!("opening database at {path}"))?;

    // Enable WAL mode for better concurrent read performance
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA busy_timeout=5000;")?;

    create_tables(&conn)?;
    create_indexes(&conn)?;

    info!(path, "Database initialized");
    Ok(Arc::new(Mutex::new(conn)))
}

fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS users (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            telegram_id INTEGER NOT NULL UNIQUE,
            telegram_username TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            settings_json TEXT
        );

        CREATE TABLE IF NOT EXISTS watched_addresses (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            telegram_id      INTEGER NOT NULL,
            address          TEXT NOT NULL,
            label            TEXT,
            added_at         TEXT NOT NULL DEFAULT (datetime('now')),
            last_stake_at    TEXT,
            last_stake_height INTEGER,
            last_alert_at    TEXT,
            UNIQUE(telegram_id, address)
        );

        CREATE TABLE IF NOT EXISTS stake_events (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            address         TEXT NOT NULL,
            txid            TEXT NOT NULL,
            block_height    INTEGER NOT NULL,
            block_hash      TEXT NOT NULL,
            amount_satoshis INTEGER NOT NULL,
            event_type      TEXT NOT NULL CHECK(event_type IN ('stake', 'lottery', 'payment')),
            detected_at     TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(txid, address)
        );

        CREATE TABLE IF NOT EXISTS alert_log (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            telegram_id INTEGER NOT NULL,
            address     TEXT NOT NULL,
            alert_type  TEXT NOT NULL,
            message     TEXT NOT NULL,
            sent_at     TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS alert_subscriptions (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            telegram_id INTEGER NOT NULL,
            alert_type  TEXT NOT NULL CHECK(alert_type IN (
                'large_tx', 'large_block', 'many_inputs', 'many_outputs',
                'op_return', 'unusual_script', 'anything_unusual'
            )),
            threshold   REAL NOT NULL DEFAULT 0.0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(telegram_id, alert_type)
        );

        CREATE TABLE IF NOT EXISTS fork_watchers (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            telegram_id INTEGER NOT NULL UNIQUE,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS fork_endpoints (
            id       INTEGER PRIMARY KEY AUTOINCREMENT,
            name     TEXT NOT NULL UNIQUE,
            rpc_url  TEXT NOT NULL,
            added_by INTEGER,
            added_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS fork_events (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            height     INTEGER NOT NULL,
            endpoint_a TEXT NOT NULL,
            hash_a     TEXT NOT NULL,
            endpoint_b TEXT NOT NULL,
            hash_b     TEXT NOT NULL,
            detected_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    Ok(())
}

fn create_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_address ON watched_addresses(address);
        CREATE INDEX IF NOT EXISTS idx_stake_address ON stake_events(address);
        CREATE INDEX IF NOT EXISTS idx_stake_height ON stake_events(block_height);
        ",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: parse NaiveDateTime from SQLite text
// ---------------------------------------------------------------------------

fn parse_dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_else(|_| chrono::Utc::now().naive_utc())
}

fn parse_dt_opt(s: Option<String>) -> Option<NaiveDateTime> {
    s.map(|v| parse_dt(&v))
}

fn now_str() -> String {
    chrono::Utc::now()
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

// ---------------------------------------------------------------------------
// User operations
// ---------------------------------------------------------------------------

pub fn add_user(db: &DbPool, telegram_id: i64, username: Option<&str>) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    conn.execute(
        "INSERT OR IGNORE INTO users (telegram_id, telegram_username) VALUES (?1, ?2)",
        params![telegram_id, username],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Watch operations
// ---------------------------------------------------------------------------

pub fn add_watch(
    db: &DbPool,
    telegram_id: i64,
    address: &str,
    label: Option<&str>,
) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO watched_addresses (telegram_id, address, label) VALUES (?1, ?2, ?3)",
        params![telegram_id, address, label],
    )?;
    Ok(inserted > 0)
}

pub fn remove_watch(db: &DbPool, telegram_id: i64, address: &str) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let deleted = conn.execute(
        "DELETE FROM watched_addresses WHERE telegram_id = ?1 AND address = ?2",
        params![telegram_id, address],
    )?;
    Ok(deleted > 0)
}

pub fn get_watches_for_user(db: &DbPool, telegram_id: i64) -> Result<Vec<WatchedAddress>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT id, telegram_id, address, label, added_at, last_stake_at, last_stake_height, last_alert_at
         FROM watched_addresses WHERE telegram_id = ?1 ORDER BY added_at",
    )?;
    let rows = stmt
        .query_map(params![telegram_id], |row| {
            Ok(WatchedAddress {
                id: row.get(0)?,
                telegram_id: row.get(1)?,
                address: row.get(2)?,
                label: row.get(3)?,
                added_at: parse_dt(&row.get::<_, String>(4)?),
                last_stake_at: parse_dt_opt(row.get(5)?),
                last_stake_height: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                last_alert_at: parse_dt_opt(row.get(7)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_all_watched_addresses(db: &DbPool) -> Result<HashSet<String>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare("SELECT DISTINCT address FROM watched_addresses")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(rows)
}

pub fn get_watch_label(db: &DbPool, telegram_id: i64, address: &str) -> Result<Option<String>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let label = conn
        .query_row(
            "SELECT label FROM watched_addresses WHERE telegram_id = ?1 AND address = ?2",
            params![telegram_id, address],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    Ok(label)
}

pub fn get_users_for_address(db: &DbPool, address: &str) -> Result<Vec<i64>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare("SELECT telegram_id FROM watched_addresses WHERE address = ?1")?;
    let rows = stmt
        .query_map(params![address], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_watch_count_for_user(db: &DbPool, telegram_id: i64) -> Result<u32> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM watched_addresses WHERE telegram_id = ?1",
        params![telegram_id],
        |row| row.get(0),
    )?;
    Ok(count as u32)
}

/// Get watches that haven't received an alert in `stale_secs` seconds.
pub fn get_stale_watches(db: &DbPool, stale_secs: u64) -> Result<Vec<WatchedAddress>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::seconds(stale_secs as i64);
    let cutoff_str = cutoff.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut stmt = conn.prepare(
        "SELECT id, telegram_id, address, label, added_at, last_stake_at, last_stake_height, last_alert_at
         FROM watched_addresses
         WHERE last_alert_at IS NULL OR last_alert_at < ?1",
    )?;
    let rows = stmt
        .query_map(params![cutoff_str], |row| {
            Ok(WatchedAddress {
                id: row.get(0)?,
                telegram_id: row.get(1)?,
                address: row.get(2)?,
                label: row.get(3)?,
                added_at: parse_dt(&row.get::<_, String>(4)?),
                last_stake_at: parse_dt_opt(row.get(5)?),
                last_stake_height: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                last_alert_at: parse_dt_opt(row.get(7)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Stake event operations
// ---------------------------------------------------------------------------

pub fn record_stake_event(
    db: &DbPool,
    address: &str,
    txid: &str,
    block_height: u64,
    block_hash: &str,
    amount_satoshis: i64,
    event_type: &str,
) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO stake_events (address, txid, block_height, block_hash, amount_satoshis, event_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![address, txid, block_height as i64, block_hash, amount_satoshis, event_type],
    )?;
    Ok(inserted > 0)
}

pub fn get_recent_stakes(db: &DbPool, address: &str, limit: u32) -> Result<Vec<StakeEvent>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT id, address, txid, block_height, block_hash, amount_satoshis, event_type, detected_at
         FROM stake_events WHERE address = ?1
         ORDER BY block_height DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![address, limit], |row| {
            Ok(StakeEvent {
                id: row.get(0)?,
                address: row.get(1)?,
                txid: row.get(2)?,
                block_height: row.get::<_, i64>(3)? as u64,
                block_hash: row.get(4)?,
                amount_satoshis: row.get(5)?,
                event_type: row.get(6)?,
                detected_at: parse_dt(&row.get::<_, String>(7)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn update_last_stake(db: &DbPool, address: &str, height: u64) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let now = now_str();
    conn.execute(
        "UPDATE watched_addresses SET last_stake_at = ?1, last_stake_height = ?2 WHERE address = ?3",
        params![now, height as i64, address],
    )?;
    Ok(())
}

/// Sum all recorded stake reward amounts for an address.
pub fn sum_stake_rewards(db: &DbPool, address: &str) -> Result<i64> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(amount_satoshis), 0) FROM stake_events WHERE address = ?1",
        params![address],
        |row| row.get(0),
    )?;
    Ok(total)
}

/// Update last_alert_at for ALL watchers of the given address.
pub fn update_last_alert(db: &DbPool, address: &str) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let now = now_str();
    conn.execute(
        "UPDATE watched_addresses SET last_alert_at = ?1 WHERE address = ?2",
        params![now, address],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Alert subscription operations
// ---------------------------------------------------------------------------

pub fn add_alert_subscription(
    db: &DbPool,
    telegram_id: i64,
    alert_type: &str,
    threshold: f64,
) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let inserted = conn.execute(
        "INSERT OR REPLACE INTO alert_subscriptions (telegram_id, alert_type, threshold) VALUES (?1, ?2, ?3)",
        params![telegram_id, alert_type, threshold],
    )?;
    Ok(inserted > 0)
}

pub fn remove_alert_subscription(db: &DbPool, telegram_id: i64, alert_type: &str) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let deleted = conn.execute(
        "DELETE FROM alert_subscriptions WHERE telegram_id = ?1 AND alert_type = ?2",
        params![telegram_id, alert_type],
    )?;
    Ok(deleted > 0)
}

pub fn get_subscriptions_for_user(db: &DbPool, telegram_id: i64) -> Result<Vec<AlertSubscription>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT id, telegram_id, alert_type, threshold, created_at
         FROM alert_subscriptions WHERE telegram_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![telegram_id], |row| {
            Ok(AlertSubscription {
                id: row.get(0)?,
                telegram_id: row.get(1)?,
                alert_type: row.get(2)?,
                threshold: row.get(3)?,
                created_at: parse_dt(&row.get::<_, String>(4)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_subscribers_for_alert_type(
    db: &DbPool,
    alert_type: &str,
) -> Result<Vec<AlertSubscription>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT id, telegram_id, alert_type, threshold, created_at
         FROM alert_subscriptions WHERE alert_type = ?1",
    )?;
    let rows = stmt
        .query_map(params![alert_type], |row| {
            Ok(AlertSubscription {
                id: row.get(0)?,
                telegram_id: row.get(1)?,
                alert_type: row.get(2)?,
                threshold: row.get(3)?,
                created_at: parse_dt(&row.get::<_, String>(4)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Fork watcher operations
// ---------------------------------------------------------------------------

pub fn add_fork_watcher(db: &DbPool, telegram_id: i64) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO fork_watchers (telegram_id) VALUES (?1)",
        params![telegram_id],
    )?;
    Ok(inserted > 0)
}

pub fn remove_fork_watcher(db: &DbPool, telegram_id: i64) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let deleted = conn.execute(
        "DELETE FROM fork_watchers WHERE telegram_id = ?1",
        params![telegram_id],
    )?;
    Ok(deleted > 0)
}

pub fn get_fork_watchers(db: &DbPool) -> Result<Vec<i64>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare("SELECT telegram_id FROM fork_watchers")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Fork endpoint operations
// ---------------------------------------------------------------------------

pub fn add_fork_endpoint(
    db: &DbPool,
    name: &str,
    rpc_url: &str,
    added_by: Option<i64>,
) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO fork_endpoints (name, rpc_url, added_by) VALUES (?1, ?2, ?3)",
        params![name, rpc_url, added_by],
    )?;
    Ok(inserted > 0)
}

pub fn remove_fork_endpoint(db: &DbPool, name: &str) -> Result<bool> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let deleted = conn.execute("DELETE FROM fork_endpoints WHERE name = ?1", params![name])?;
    Ok(deleted > 0)
}

pub fn get_fork_endpoints(db: &DbPool) -> Result<Vec<ForkEndpoint>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt =
        conn.prepare("SELECT id, name, rpc_url, added_by, added_at FROM fork_endpoints")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ForkEndpoint {
                id: row.get(0)?,
                name: row.get(1)?,
                rpc_url: row.get(2)?,
                added_by: row.get(3)?,
                added_at: parse_dt(&row.get::<_, String>(4)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn record_fork_event(
    db: &DbPool,
    height: u64,
    endpoint_a: &str,
    hash_a: &str,
    endpoint_b: &str,
    hash_b: &str,
) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    conn.execute(
        "INSERT INTO fork_events (height, endpoint_a, hash_a, endpoint_b, hash_b) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![height as i64, endpoint_a, hash_a, endpoint_b, hash_b],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Aggregate counts (used by /status and /forkstatus commands)
// ---------------------------------------------------------------------------

pub fn count_watches(db: &DbPool) -> Result<u64> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM watched_addresses", [], |row| {
        row.get(0)
    })?;
    Ok(count as u64)
}

pub fn count_users(db: &DbPool) -> Result<u64> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
    Ok(count as u64)
}

pub fn count_fork_watchers(db: &DbPool) -> Result<u64> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM fork_watchers", [], |row| row.get(0))?;
    Ok(count as u64)
}

/// Get all unique watched addresses along with their watchers, suitable for
/// the missed-stake alert loop.
pub fn get_all_watches(db: &DbPool) -> Result<Vec<WatchedAddress>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT id, telegram_id, address, label, added_at, last_stake_at, last_stake_height, last_alert_at
         FROM watched_addresses ORDER BY address",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(WatchedAddress {
                id: row.get(0)?,
                telegram_id: row.get(1)?,
                address: row.get(2)?,
                label: row.get(3)?,
                added_at: parse_dt(&row.get::<_, String>(4)?),
                last_stake_at: parse_dt_opt(row.get(5)?),
                last_stake_height: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                last_alert_at: parse_dt_opt(row.get(7)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
