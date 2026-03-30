use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tracing::{debug, error, info, warn};

use crate::rpc::RpcClient;
use super::db::{self, VaultDb};

/// Chain scanner that processes blocks for vault UTXOs.
pub struct Scanner {
    rpc: Arc<dyn RpcClient>,
    db: VaultDb,
}

impl Scanner {
    /// Create a new scanner with the given RPC client and vault database.
    pub fn new(rpc: Arc<dyn RpcClient>, db: VaultDb) -> Self {
        Self { rpc, db }
    }

    /// Return a reference to the vault database.
    pub fn db(&self) -> &VaultDb {
        &self.db
    }

    /// Process a single block at the given height for vault UTXOs.
    ///
    /// This inspects the coinstake transaction (block.tx[1]):
    /// - Marks spent vault UTXOs from the transaction inputs
    /// - Adds new vault UTXOs from outputs with script type "vault"
    pub async fn process_block(&self, height: u64) -> Result<()> {
        // 1. Get block hash
        let hash = self.rpc.get_block_hash(height).await?;

        // 2. Get block
        let block = self.rpc.get_block(&hash).await?;

        // 3. Skip blocks with fewer than 2 transactions (no coinstake)
        if block.tx.len() < 2 {
            db::set_last_scanned_height(&self.db, height)?;
            return Ok(());
        }

        // 4. Get coinstake transaction (tx[1])
        let coinstake_txid = &block.tx[1];
        let tx = self.rpc.get_raw_transaction(coinstake_txid).await?;

        // 5. Process inputs: mark spent vault UTXOs
        for vin in &tx.vin {
            if let (Some(ref prev_txid), Some(prev_vout)) = (&vin.txid, vin.vout) {
                if let Err(e) =
                    db::mark_spent(&self.db, prev_txid, prev_vout, &tx.txid, height)
                {
                    warn!(
                        prev_txid = %prev_txid,
                        prev_vout = prev_vout,
                        error = %e,
                        "Failed to mark vault UTXO as spent"
                    );
                }
            }
        }

        // 6. Process outputs: add new vault UTXOs
        for vout in &tx.vout {
            let is_vault = vout
                .script_pub_key
                .script_type
                .as_deref()
                == Some("vault");

            if !is_vault {
                continue;
            }

            let addresses = match &vout.script_pub_key.addresses {
                Some(addrs) if !addrs.is_empty() => addrs,
                _ => continue,
            };

            let owner = &addresses[0];
            let manager = addresses.get(1).map(|s| s.as_str());
            let value_satoshis = (vout.value * 100_000_000.0).round() as i64;

            if let Err(e) = db::add_vault_utxo(
                &self.db,
                &tx.txid,
                vout.n,
                owner,
                manager,
                value_satoshis,
                height,
                Some(&hash),
            ) {
                warn!(
                    txid = %tx.txid,
                    vout_n = vout.n,
                    error = %e,
                    "Failed to add vault UTXO"
                );
            }
        }

        // 7. Update scan state
        db::set_last_scanned_height(&self.db, height)?;

        debug!(height, hash = %hash, "Processed block for vault UTXOs");
        Ok(())
    }

    /// Scan a range of blocks (inclusive) for vault UTXOs.
    ///
    /// Blocks are processed sequentially. Progress is logged every 1000 blocks.
    pub async fn scan_range(&self, start: u64, end: u64) -> Result<()> {
        if start > end {
            return Ok(());
        }

        let total = end - start + 1;
        let scan_start = Instant::now();
        let mut processed: u64 = 0;

        info!(
            start,
            end,
            total,
            "Starting vault UTXO scan"
        );

        for height in start..=end {
            if let Err(e) = self.process_block(height).await {
                error!(height, error = %e, "Error processing block, continuing");
            }

            processed += 1;

            if processed % 1000 == 0 {
                let elapsed = scan_start.elapsed().as_secs_f64();
                let rate = processed as f64 / elapsed;
                let remaining = (total - processed) as f64 / rate;
                info!(
                    height,
                    processed,
                    total,
                    rate_blocks_per_sec = format!("{:.1}", rate),
                    eta_seconds = format!("{:.0}", remaining),
                    "Scan progress"
                );
            }
        }

        let elapsed = scan_start.elapsed();
        let rate = if elapsed.as_secs_f64() > 0.0 {
            processed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        info!(
            processed,
            elapsed_secs = format!("{:.1}", elapsed.as_secs_f64()),
            rate_blocks_per_sec = format!("{:.1}", rate),
            "Vault UTXO scan complete"
        );

        Ok(())
    }
}
