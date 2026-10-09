//! How much DIVI is staking across the whole network, measured from the chain.
//!
//! The node doesn't report it, and a fixed figure goes stale (the config said
//! 4 billion when about 2.2 billion was staking, which made every expectation
//! too low). A wallet that stakes all the time wins blocks in proportion to its
//! share, so `network = balance × blocks / stakes`. Summed over the busiest
//! watched wallets this gives the network size; a wallet that is sometimes
//! offline only makes the estimate a little high, never low.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn};

use crate::db::DbPool;
use crate::rpc::{AddressDelta, RpcClient};
use crate::stake_check::BLOCKS_PER_DAY;

/// Current estimate in whole DIVI. 0 until `init` runs.
static NETWORK_STAKING_SUPPLY: AtomicU64 = AtomicU64::new(0);

/// How far back the measurement looks.
const WINDOW_BLOCKS: u64 = 30 * BLOCKS_PER_DAY as u64;
/// Wallets measured, busiest first.
const SAMPLE_WALLETS: usize = 5;
/// A wallet needs this many stakes in the window to say anything useful.
const MIN_STAKES: usize = 20;
/// Estimates outside this range are rejected as a bad measurement.
const SANE_RANGE: std::ops::RangeInclusive<u64> = 500_000_000..=20_000_000_000;
const REFRESH: Duration = Duration::from_secs(6 * 3600);

/// The network staking supply in whole DIVI.
pub fn get() -> u64 {
    NETWORK_STAKING_SUPPLY.load(Ordering::Relaxed)
}

/// Start from the configured figure, used until the first measurement.
pub fn init(configured: u64) {
    NETWORK_STAKING_SUPPLY.store(configured, Ordering::Relaxed);
}

/// Stakes in a list of address deltas, as `(height, reward_satoshis)`.
///
/// A stake spends the address's coin and pays it back with the reward in one
/// transaction, so it is a transaction with a negative delta and a positive
/// net. Deposits and lottery wins are only positive; sends net negative.
pub fn stakes_from_deltas(deltas: &[AddressDelta]) -> Vec<(u64, i64)> {
    let mut by_tx: HashMap<&str, (u64, i64, bool)> = HashMap::new();
    for d in deltas {
        let e = by_tx.entry(d.txid.as_str()).or_insert((d.height, 0, false));
        e.0 = e.0.max(d.height);
        e.1 += d.satoshis;
        e.2 |= d.satoshis < 0;
    }
    let mut stakes: Vec<(u64, i64)> = by_tx
        .into_values()
        .filter(|&(_, net, spent)| spent && net > 0)
        .map(|(h, net, _)| (h, net))
        .collect();
    stakes.sort_unstable_by_key(|s| std::cmp::Reverse(s.0));
    stakes
}

/// Measure the network staking supply in whole DIVI.
pub async fn measure(rpc: &Arc<dyn RpcClient>, db: &DbPool) -> Result<Option<u64>> {
    let height = rpc.get_block_count().await?;
    let start = height.saturating_sub(WINDOW_BLOCKS);
    let candidates = crate::db::busiest_stakers(db, start, SAMPLE_WALLETS)?;

    let (mut balance_sum, mut stake_sum) = (0f64, 0usize);
    for address in candidates {
        let deltas = rpc
            .get_address_deltas(&address, Some(start), Some(height))
            .await?;
        let stakes = stakes_from_deltas(&deltas).len();
        if stakes < MIN_STAKES {
            continue;
        }
        let balance = rpc.get_address_balance(&address).await?.balance;
        if balance <= 0 {
            continue; // vault: its balance isn't on the address index
        }
        balance_sum += balance as f64 / 1e8;
        stake_sum += stakes;
    }
    if stake_sum == 0 {
        return Ok(None);
    }
    Ok(Some(
        (balance_sum * WINDOW_BLOCKS as f64 / stake_sum as f64) as u64,
    ))
}

/// Re-measure every few hours for as long as the process runs.
pub async fn run_refresh_loop(rpc: Arc<dyn RpcClient>, db: DbPool) {
    loop {
        match measure(&rpc, &db).await {
            Ok(Some(n)) if SANE_RANGE.contains(&n) => {
                info!(divi = n, "Network staking supply measured");
                NETWORK_STAKING_SUPPLY.store(n, Ordering::Relaxed);
            }
            Ok(Some(n)) => warn!(
                divi = n,
                kept = get(),
                "Network staking supply out of range; kept"
            ),
            Ok(None) => warn!(
                kept = get(),
                "Network staking supply: no wallet to measure; kept"
            ),
            Err(e) => {
                warn!(error = %e, kept = get(), "Network staking supply measurement failed; kept")
            }
        }
        tokio::time::sleep(REFRESH).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(txid: &str, satoshis: i64, height: u64) -> AddressDelta {
        AddressDelta {
            txid: txid.into(),
            index: 0,
            satoshis,
            height,
        }
    }

    #[test]
    fn finds_stakes_only() {
        let deltas = vec![
            d("stake", -1_000_000_000_000, 10), // coin spent…
            d("stake", 1_041_500_000_000, 10),  // …paid back with 415 DIVI
            d("deposit", 5_000_000_000, 11),
            d("send", -2_000_000_000, 12),
            d("send", 1_500_000_000, 12), // change
            d("lottery", 10_000_000_000, 13),
        ];
        let s = stakes_from_deltas(&deltas);
        assert_eq!(s, vec![(10, 41_500_000_000)]);
    }
}
