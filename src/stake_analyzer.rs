use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, info, warn};

use crate::config::GeneralConfig;
use crate::db::{self, DbPool};
use crate::notifier::Notifier;
use crate::rpc::RpcClient;
use crate::utils::satoshi_to_divi;

// ---------------------------------------------------------------------------
// StakeAnalyzer
// ---------------------------------------------------------------------------

/// Estimates expected staking frequency based on balance and network supply,
/// runs periodic missed-stake alert checks, and backfills historical stake
/// data when a new address is watched.
pub struct StakeAnalyzer {
    rpc: Arc<dyn RpcClient>,
    db: DbPool,
    notifier: Arc<Notifier>,
    config: GeneralConfig,
}

impl StakeAnalyzer {
    pub fn new(
        rpc: Arc<dyn RpcClient>,
        db: DbPool,
        notifier: Arc<Notifier>,
        config: GeneralConfig,
    ) -> Self {
        Self {
            rpc,
            db,
            notifier,
            config,
        }
    }

    // -----------------------------------------------------------------------
    // Expected interval calculation
    // -----------------------------------------------------------------------

    /// Compute the expected time between staking rewards in seconds.
    ///
    /// The Divi staking model is essentially a Poisson process where the
    /// probability of winning a block is proportional to your share of the
    /// total staking supply. With ~60-second block times:
    ///
    ///   P(win per block) = balance / network_supply
    ///   expected_blocks  = 1 / P
    ///   expected_seconds = expected_blocks * 60
    ///
    /// The result is clamped to [1 hour, 365 days]. Returns `f64::INFINITY`
    /// if the balance is zero or negative.
    pub fn compute_expected_interval(balance_satoshis: i64, network_supply: u64) -> f64 {
        let balance_divi = balance_satoshis as f64 / 1e8;

        if balance_divi <= 0.0 {
            return f64::INFINITY;
        }

        if network_supply == 0 {
            return f64::INFINITY;
        }

        let probability = balance_divi / network_supply as f64;
        if probability <= 0.0 {
            return f64::INFINITY;
        }

        let expected_blocks = 1.0 / probability;
        let expected_secs = expected_blocks * 60.0;

        // Clamp to reasonable bounds
        const MIN_INTERVAL: f64 = 3600.0; // 1 hour
        const MAX_INTERVAL: f64 = 365.0 * 86400.0; // 1 year
        expected_secs.clamp(MIN_INTERVAL, MAX_INTERVAL)
    }

    // -----------------------------------------------------------------------
    // Missed-stake alert loop
    // -----------------------------------------------------------------------

    /// Run the missed-stake alert check loop. This runs indefinitely, sleeping
    /// for `alert_check_interval_secs` between iterations.
    pub async fn run_alert_loop(&self) {
        let interval = tokio::time::Duration::from_secs(self.config.alert_check_interval_secs);
        info!(
            interval_secs = self.config.alert_check_interval_secs,
            "Starting missed-stake alert loop"
        );

        loop {
            if let Err(e) = self.check_missed_stakes().await {
                warn!(error = %e, "Missed-stake check iteration failed");
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Single iteration: check every watched address for overdue stakes.
    async fn check_missed_stakes(&self) -> Result<()> {
        let watches = db::get_all_watches(&self.db)?;
        let now = chrono::Utc::now().naive_utc();

        // Deduplicate by address to avoid querying the same balance multiple
        // times when multiple users watch the same address.
        let mut seen_addresses = std::collections::HashSet::new();

        for watch in &watches {
            if !seen_addresses.insert(watch.address.clone()) {
                continue;
            }

            // Fetch balance; try regular first, fall back to vault scan
            let effective_balance = {
                let regular = match self.rpc.get_address_balance(&watch.address).await {
                    Ok(b) => b.balance,
                    Err(e) => {
                        debug!(address = %watch.address, error = %e, "Could not fetch balance");
                        continue;
                    }
                };

                if regular > 0 {
                    regular
                } else {
                    // Try vault balance (only_vaults=true)
                    match self.rpc.get_vault_balance(&watch.address).await {
                        Ok(vb) if vb.balance > 0 => vb.balance,
                        _ => {
                            // No regular or vault balance; skip
                            continue;
                        }
                    }
                }
            };

            // Compute expected interval
            let expected_secs = Self::compute_expected_interval(
                effective_balance,
                self.config.network_staking_supply,
            );
            if expected_secs.is_infinite() {
                continue;
            }

            // Determine how long since last stake
            let time_since_secs: u64 = match &watch.last_stake_at {
                None => {
                    // Grace period: if the address was added less than 24h ago,
                    // skip -- we don't have enough history to alert on.
                    let since_added = (now - watch.added_at).num_seconds().max(0) as u64;
                    if since_added < 86400 {
                        continue;
                    }
                    since_added
                }
                Some(last) => (now - *last).num_seconds().max(0) as u64,
            };

            // Check if overdue: time_since > expected * alert_multiplier
            let threshold_secs = expected_secs * self.config.alert_multiplier as f64;
            if (time_since_secs as f64) <= threshold_secs {
                continue;
            }

            // Cooldown: don't re-alert within 1x expected interval
            if let Some(ref last_alert) = watch.last_alert_at {
                let since_alert = (now - *last_alert).num_seconds().max(0) as u64;
                if (since_alert as f64) < expected_secs {
                    continue;
                }
            }

            // Send the missed-stake alert to all watchers of this address
            let label = watch.label.as_deref();
            let message = self.notifier.format_missed_stake_alert(
                &watch.address,
                label,
                expected_secs,
                time_since_secs,
                effective_balance,
            );

            info!(
                address = %watch.address,
                expected_secs,
                time_since_secs,
                "Sending missed-stake alert"
            );

            if let Err(e) = self
                .notifier
                .notify_users_for_address(&watch.address, &message)
                .await
            {
                warn!(
                    address = %watch.address,
                    error = %e,
                    "Failed to send missed-stake notification"
                );
            }

            // Update last_alert_at for all watchers of this address
            if let Err(e) = db::update_last_alert(&self.db, &watch.address) {
                warn!(address = %watch.address, error = %e, "Failed to update last_alert_at");
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Historical backfill
    // -----------------------------------------------------------------------

    /// Called when a user adds a new watch. Scans recent address deltas to
    /// discover and record historical stake events, and sets `last_stake_at`
    /// to the most recent one.
    ///
    /// This is best-effort: it will fail gracefully on backends that do not
    /// support `getaddressdeltas` (e.g., chainz).
    pub async fn backfill_stakes(
        rpc: &Arc<dyn RpcClient>,
        db: &DbPool,
        address: &str,
    ) -> Result<()> {
        info!(address, "Starting historical stake backfill");

        // Get current block height
        let current_height = rpc.get_block_count().await?;
        let start_height = current_height.saturating_sub(10_000);

        // Fetch address deltas for the last ~10,000 blocks.
        let deltas = match rpc
            .get_address_deltas(address, Some(start_height), Some(current_height))
            .await
        {
            Ok(d) => d,
            Err(e) => {
                info!(
                    address,
                    error = %e,
                    "get_address_deltas not available, skipping backfill"
                );
                return Ok(());
            }
        };

        if deltas.is_empty() {
            // No regular deltas -- try vault deltas (only_vaults=true)
            info!(address, "No regular deltas, trying vault deltas");
            match rpc
                .get_vault_deltas(address, Some(start_height), Some(current_height))
                .await
            {
                Ok(vault_deltas) if !vault_deltas.is_empty() => {
                    info!(address, count = vault_deltas.len(), "Found vault deltas");
                    // Use vault deltas instead — fall through to the same processing
                    // by reassigning deltas
                    let mut positive: Vec<_> = vault_deltas.into_iter().filter(|d| d.satoshis > 0).collect();
                    positive.sort_by(|a, b| b.height.cmp(&a.height));
                    positive.truncate(20);

                    let mut recorded = 0u32;
                    for delta in &positive {
                        if let Ok(tx) = rpc.get_raw_transaction(&delta.txid).await {
                            let is_stake = tx.vin.first().map_or(false, |v| v.coinbase.is_some());
                            if is_stake {
                                let _ = crate::db::record_stake_event(
                                    db,
                                    address,
                                    &delta.txid,
                                    delta.height,
                                    tx.blockhash.as_deref().unwrap_or(""),
                                    delta.satoshis,
                                    "stake",
                                );
                                recorded += 1;
                                if recorded >= 10 { break; }
                            }
                        }
                    }

                    // Set last_stake_at from most recent positive delta
                    if let Some(latest) = positive.first() {
                        crate::db::update_last_stake(db, address, latest.height)?;
                    }

                    info!(address, recorded, "Vault backfill complete");
                    return Ok(());
                }
                Ok(_) => {
                    info!(address, "No vault deltas found either, skipping backfill");
                    return Ok(());
                }
                Err(e) => {
                    info!(address, error = %e, "Vault deltas not available, skipping backfill");
                    return Ok(());
                }
            }
        }

        // Filter to positive deltas (credits) and take the most recent 20
        let mut positive_deltas: Vec<_> = deltas
            .into_iter()
            .filter(|d| d.satoshis > 0)
            .collect();

        // Sort by height descending so we process most recent first
        positive_deltas.sort_by(|a, b| b.height.cmp(&a.height));
        positive_deltas.truncate(20);

        let mut latest_height: Option<u64> = None;
        let mut recorded = 0u32;

        for delta in &positive_deltas {
            // Fetch the full transaction to check if it's a coinbase (staking)
            let tx = match rpc.get_raw_transaction(&delta.txid).await {
                Ok(t) => t,
                Err(e) => {
                    debug!(
                        txid = %delta.txid,
                        error = %e,
                        "Could not fetch transaction for backfill"
                    );
                    continue;
                }
            };

            // In Divi, staking rewards come from coinbase/coinstake transactions.
            // Check if the first vin has a coinbase field or no txid (coinstake).
            let is_coinbase_or_coinstake = tx
                .vin
                .first()
                .map_or(false, |v| v.coinbase.is_some() || v.txid.is_none());

            if !is_coinbase_or_coinstake {
                continue;
            }

            // Determine the block hash from the transaction
            let block_hash = match &tx.blockhash {
                Some(h) => h.clone(),
                None => match rpc.get_block_hash(delta.height).await {
                    Ok(h) => h,
                    Err(_) => continue,
                },
            };

            // Record up to 10 stake events
            if recorded < 10 {
                match db::record_stake_event(
                    db,
                    address,
                    &delta.txid,
                    delta.height,
                    &block_hash,
                    delta.satoshis,
                    "stake",
                ) {
                    Ok(true) => {
                        recorded += 1;
                        debug!(
                            address,
                            txid = %delta.txid,
                            height = delta.height,
                            amount = %satoshi_to_divi(delta.satoshis),
                            "Backfilled stake event"
                        );
                    }
                    Ok(false) => {
                        // Duplicate -- already recorded
                    }
                    Err(e) => {
                        warn!(
                            txid = %delta.txid,
                            error = %e,
                            "Failed to record backfilled stake"
                        );
                    }
                }
            }

            // Track the highest (most recent) height for last_stake_at
            if latest_height.is_none() || Some(delta.height) > latest_height {
                latest_height = Some(delta.height);
            }
        }

        // Update last_stake_at to the most recent stake found
        if let Some(height) = latest_height {
            if let Err(e) = db::update_last_stake(db, address, height) {
                warn!(address, error = %e, "Failed to update last_stake after backfill");
            }
        }

        info!(
            address,
            recorded,
            latest_height = ?latest_height,
            "Backfill complete"
        );

        Ok(())
    }

    /// Backfill stake history for a vault-locked address by scanning recent
    /// coinstake transactions for vault outputs containing the address.
    ///
    /// This is used when `get_address_deltas` returns empty (vault addresses
    /// are invisible to the address index).
    async fn backfill_vault_stakes(
        rpc: &Arc<dyn RpcClient>,
        db: &DbPool,
        address: &str,
        current_height: u64,
    ) -> Result<()> {
        // Scan more blocks for backfill than for balance (need historical data)
        const VAULT_BACKFILL_DEPTH: u64 = 2000;
        let start_height = current_height.saturating_sub(VAULT_BACKFILL_DEPTH);

        let mut latest_height: Option<u64> = None;
        let mut recorded = 0u32;

        for height in (start_height..=current_height).rev() {
            let hash = match rpc.get_block_hash(height).await {
                Ok(h) => h,
                Err(_) => continue,
            };

            let block = match rpc.get_block(&hash).await {
                Ok(b) => b,
                Err(_) => continue,
            };

            // Coinstake is tx[1]
            let coinstake_txid = match block.tx.get(1) {
                Some(txid) => txid,
                None => continue,
            };

            let tx = match rpc.get_raw_transaction(coinstake_txid).await {
                Ok(t) => t,
                Err(_) => continue,
            };

            // Check each vout for a vault output containing our address
            for vout in &tx.vout {
                let is_vault = vout
                    .script_pub_key
                    .script_type
                    .as_deref()
                    == Some("vault");

                if !is_vault {
                    continue;
                }

                let contains_address = vout
                    .script_pub_key
                    .addresses
                    .as_ref()
                    .map_or(false, |addrs| addrs.iter().any(|a| a == address));

                if !contains_address {
                    continue;
                }

                let amount_satoshis = (vout.value * 100_000_000.0).round() as i64;

                if recorded < 10 {
                    match db::record_stake_event(
                        db,
                        address,
                        &tx.txid,
                        height,
                        &hash,
                        amount_satoshis,
                        "stake",
                    ) {
                        Ok(true) => {
                            recorded += 1;
                            debug!(
                                address,
                                txid = %tx.txid,
                                height,
                                amount = %satoshi_to_divi(amount_satoshis),
                                "Backfilled vault stake event"
                            );
                        }
                        Ok(false) => {}
                        Err(e) => {
                            warn!(
                                txid = %tx.txid,
                                error = %e,
                                "Failed to record backfilled vault stake"
                            );
                        }
                    }
                }

                if latest_height.is_none() || Some(height) > latest_height {
                    latest_height = Some(height);
                }

                // Only count the first matching vout per tx
                break;
            }
        }

        // Update last_stake_at to the most recent vault stake found
        if let Some(height) = latest_height {
            if let Err(e) = db::update_last_stake(db, address, height) {
                warn!(address, error = %e, "Failed to update last_stake after vault backfill");
            }
        }

        info!(
            address,
            recorded,
            latest_height = ?latest_height,
            "Vault backfill complete"
        );

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_balance_returns_infinity() {
        assert!(StakeAnalyzer::compute_expected_interval(0, 3_000_000).is_infinite());
    }

    #[test]
    fn test_negative_balance_returns_infinity() {
        assert!(
            StakeAnalyzer::compute_expected_interval(-100_000_000, 3_000_000).is_infinite()
        );
    }

    #[test]
    fn test_zero_supply_returns_infinity() {
        assert!(StakeAnalyzer::compute_expected_interval(100_000_000, 0).is_infinite());
    }

    #[test]
    fn test_expected_interval_basic() {
        // 10,000 DIVI in a 3,000,000 supply network
        // P = 10,000 / 3,000,000 = 1/300
        // expected_blocks = 300
        // expected_secs = 300 * 60 = 18,000 (5 hours)
        let balance = 10_000 * 100_000_000; // 10,000 DIVI in satoshis
        let supply = 3_000_000;
        let result = StakeAnalyzer::compute_expected_interval(balance, supply);
        assert!((result - 18_000.0).abs() < 1.0);
    }

    #[test]
    fn test_expected_interval_clamped_low() {
        // Very large balance relative to supply -> clamped to 1 hour minimum
        let balance = 100_000_000 * 100_000_000_i64; // 100M DIVI
        let supply = 3_000_000;
        let result = StakeAnalyzer::compute_expected_interval(balance, supply);
        assert_eq!(result, 3600.0);
    }

    #[test]
    fn test_expected_interval_clamped_high() {
        // Tiny balance -> clamped to 365 days maximum
        let balance = 1; // 0.00000001 DIVI
        let supply = 3_000_000_000;
        let result = StakeAnalyzer::compute_expected_interval(balance, supply);
        assert_eq!(result, 365.0 * 86400.0);
    }

    #[test]
    fn test_expected_interval_proportional() {
        // Doubling balance should halve the interval (use values that
        // don't hit the 1-hour floor clamp).
        let supply = 3_000_000;
        let base = 1_000 * 100_000_000_i64; // 1,000 DIVI -> ~180,000s
        let interval1 = StakeAnalyzer::compute_expected_interval(base, supply);
        let interval2 = StakeAnalyzer::compute_expected_interval(base * 2, supply);
        assert!((interval1 / interval2 - 2.0).abs() < 0.01);
    }
}
