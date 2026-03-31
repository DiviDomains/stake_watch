use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use tracing::info;

/// Thread-safe database connection handle for the vault indexer.
pub type VaultDb = Arc<Mutex<Connection>>;

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VaultUtxo {
    pub txid: String,
    pub vout_n: u32,
    pub owner_address: String,
    pub manager_address: Option<String>,
    pub value_satoshis: i64,
    pub block_height: u64,
    pub block_hash: Option<String>,
    pub spent_txid: Option<String>,
    pub spent_height: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct VaultBalance {
    pub balance_satoshis: i64,
    pub utxo_count: u32,
}

#[derive(Debug, Clone)]
pub struct IndexerStats {
    pub total_utxos: u64,
    pub total_unspent: u64,
    pub total_addresses: u64,
    pub total_value_satoshis: i64,
    pub last_scanned_height: u64,
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Open (or create) the vault indexer SQLite database and run migrations.
pub fn init_db(path: &str) -> Result<VaultDb> {
    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating vault db directory: {}", parent.display()))?;
    }

    let conn =
        Connection::open(path).with_context(|| format!("opening vault database at {path}"))?;

    // Enable WAL mode for better concurrent read performance
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA busy_timeout=5000;")?;

    create_tables(&conn)?;

    info!(path, "Vault indexer database initialized");
    Ok(Arc::new(Mutex::new(conn)))
}

fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS vault_utxos (
            txid             TEXT NOT NULL,
            vout_n           INTEGER NOT NULL,
            owner_address    TEXT NOT NULL,
            manager_address  TEXT,
            value_satoshis   INTEGER NOT NULL,
            block_height     INTEGER NOT NULL,
            block_hash       TEXT,
            created_at       TEXT DEFAULT (datetime('now')),
            spent_txid       TEXT,
            spent_height     INTEGER,
            PRIMARY KEY (txid, vout_n)
        );

        CREATE INDEX IF NOT EXISTS idx_vault_owner ON vault_utxos(owner_address);
        CREATE INDEX IF NOT EXISTS idx_vault_height ON vault_utxos(block_height);

        CREATE TABLE IF NOT EXISTS scan_state (
            key   TEXT PRIMARY KEY,
            value TEXT
        );
        ",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scan state
// ---------------------------------------------------------------------------

/// Get the last scanned block height from the database. Returns 0 if not set.
pub fn get_last_scanned_height(db: &VaultDb) -> u64 {
    let conn = match db.lock() {
        Ok(c) => c,
        Err(_) => return 0,
    };
    conn.query_row(
        "SELECT value FROM scan_state WHERE key = 'last_scanned_height'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<u64>().ok())
    .unwrap_or(0)
}

/// Set the last scanned block height in the database.
pub fn set_last_scanned_height(db: &VaultDb, height: u64) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    conn.execute(
        "INSERT OR REPLACE INTO scan_state (key, value) VALUES ('last_scanned_height', ?1)",
        params![height.to_string()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// UTXO operations
// ---------------------------------------------------------------------------

/// Insert a new vault UTXO into the database.
pub fn add_vault_utxo(
    db: &VaultDb,
    txid: &str,
    vout_n: u32,
    owner: &str,
    manager: Option<&str>,
    value_satoshis: i64,
    height: u64,
    hash: Option<&str>,
) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    conn.execute(
        "INSERT OR REPLACE INTO vault_utxos
         (txid, vout_n, owner_address, manager_address, value_satoshis, block_height, block_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            txid,
            vout_n,
            owner,
            manager,
            value_satoshis,
            height as i64,
            hash
        ],
    )?;
    Ok(())
}

/// Mark a vault UTXO as spent by recording the spending transaction and height.
pub fn mark_spent(
    db: &VaultDb,
    txid: &str,
    vout_n: u32,
    spent_txid: &str,
    spent_height: u64,
) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    conn.execute(
        "UPDATE vault_utxos SET spent_txid = ?1, spent_height = ?2 WHERE txid = ?3 AND vout_n = ?4",
        params![spent_txid, spent_height as i64, txid, vout_n],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Query operations
// ---------------------------------------------------------------------------

/// Get the total vault balance and UTXO count for an address (unspent only).
pub fn get_balance(db: &VaultDb, owner_address: &str) -> Result<VaultBalance> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let (balance, count): (i64, u32) = conn.query_row(
        "SELECT COALESCE(SUM(value_satoshis), 0), COUNT(*)
         FROM vault_utxos
         WHERE owner_address = ?1 AND spent_txid IS NULL",
        params![owner_address],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(VaultBalance {
        balance_satoshis: balance,
        utxo_count: count,
    })
}

/// Get all unspent vault UTXOs for an address.
pub fn get_unspent_utxos(db: &VaultDb, owner_address: &str) -> Result<Vec<VaultUtxo>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT txid, vout_n, owner_address, manager_address, value_satoshis,
                block_height, block_hash, spent_txid, spent_height
         FROM vault_utxos
         WHERE owner_address = ?1 AND spent_txid IS NULL
         ORDER BY block_height DESC",
    )?;
    let rows = stmt
        .query_map(params![owner_address], row_to_vault_utxo)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get vault UTXO history for an address (both spent and unspent), ordered by
/// block_height descending, with an optional limit.
pub fn get_stake_history(db: &VaultDb, owner_address: &str, limit: u32) -> Result<Vec<VaultUtxo>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT txid, vout_n, owner_address, manager_address, value_satoshis,
                block_height, block_hash, spent_txid, spent_height
         FROM vault_utxos
         WHERE owner_address = ?1
         ORDER BY block_height DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![owner_address, limit], row_to_vault_utxo)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get aggregate statistics about the indexed vault UTXOs.
pub fn get_stats(db: &VaultDb) -> Result<IndexerStats> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;

    let total_utxos: u64 = conn.query_row("SELECT COUNT(*) FROM vault_utxos", [], |row| {
        row.get::<_, i64>(0)
    })? as u64;

    let total_unspent: u64 = conn.query_row(
        "SELECT COUNT(*) FROM vault_utxos WHERE spent_txid IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )? as u64;

    let total_addresses: u64 = conn.query_row(
        "SELECT COUNT(DISTINCT owner_address) FROM vault_utxos WHERE spent_txid IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )? as u64;

    let total_value_satoshis: i64 = conn.query_row(
        "SELECT COALESCE(SUM(value_satoshis), 0) FROM vault_utxos WHERE spent_txid IS NULL",
        [],
        |row| row.get(0),
    )?;

    // Read last_scanned_height without going through the VaultDb wrapper
    // (we already hold the lock)
    let last_scanned_height: u64 = conn
        .query_row(
            "SELECT value FROM scan_state WHERE key = 'last_scanned_height'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    Ok(IndexerStats {
        total_utxos,
        total_unspent,
        total_addresses,
        total_value_satoshis,
        last_scanned_height,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn row_to_vault_utxo(row: &rusqlite::Row) -> rusqlite::Result<VaultUtxo> {
    Ok(VaultUtxo {
        txid: row.get(0)?,
        vout_n: row.get::<_, u32>(1)?,
        owner_address: row.get(2)?,
        manager_address: row.get(3)?,
        value_satoshis: row.get(4)?,
        block_height: row.get::<_, i64>(5)? as u64,
        block_hash: row.get(6)?,
        spent_txid: row.get(7)?,
        spent_height: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
    })
}
