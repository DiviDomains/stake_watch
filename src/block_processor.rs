use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::alert_analyzer::AlertAnalyzer;
use crate::db::{self, DbPool};
use crate::notifier::Notifier;
use crate::rpc::{RpcClient, Transaction, Vout};
use crate::utils::satoshi_to_divi;

/// Processes block hashes received from a monitor, inspecting each block's
/// transactions for staking rewards to watched addresses and running
/// anomaly detection.
pub struct BlockProcessor {
    rpc: Arc<dyn RpcClient>,
    db: DbPool,
    notifier: Arc<Notifier>,
    alert_analyzer: AlertAnalyzer,
}

impl BlockProcessor {
    pub fn new(rpc: Arc<dyn RpcClient>, db: DbPool, notifier: Arc<Notifier>) -> Self {
        let alert_analyzer = AlertAnalyzer::new(db.clone(), Arc::clone(&notifier));
        Self {
            rpc,
            db,
            notifier,
            alert_analyzer,
        }
    }

    /// Run the block processing loop, consuming block hashes from the channel
    /// until the sender is dropped or an unrecoverable error occurs.
    pub async fn run(&self, mut rx: mpsc::Receiver<String>) {
        info!("Block processor started, waiting for block hashes");

        while let Some(block_hash) = rx.recv().await {
            if let Err(e) = self.process_block(&block_hash).await {
                error!(
                    block_hash = %block_hash,
                    error = %e,
                    "Error processing block"
                );
            }
        }

        info!("Block processor channel closed, shutting down");
    }

    /// Process a single block: fetch data, check for stake rewards to watched
    /// addresses, detect lottery winners, and run anomaly analysis.
    async fn process_block(&self, hash: &str) -> anyhow::Result<()> {
        // 1. Fetch the block (with retry — ZMQ event may arrive before the
        //    block is fully indexed by the RPC node)
        let mut block = None;
        for attempt in 0..3 {
            match self.rpc.get_block(hash).await {
                Ok(b) => {
                    block = Some(b);
                    break;
                }
                Err(e) if attempt < 2 => {
                    tracing::debug!(hash, attempt, error = %e, "Block not ready, retrying...");
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        500 * (attempt + 1) as u64,
                    ))
                    .await;
                }
                Err(e) => return Err(e),
            }
        }
        let block = block.unwrap();

        info!(
            height = block.height,
            hash = %block.hash,
            tx_count = block.tx.len(),
            "Processing block"
        );

        // 2. Get watched addresses from DB (O(1) lookup via HashSet)
        let watched_addresses = db::get_all_watched_addresses(&self.db)?;

        // 3. Check if we have any subscribers for alerts
        let has_alert_subscribers = {
            let subs = db::get_subscribers_for_alert_type(
                &self.db,
                crate::alert_analyzer::ALERT_ANYTHING_UNUSUAL,
            )
            .unwrap_or_default();
            !subs.is_empty()
        };

        // 4. If no watched addresses and no alert subscribers, skip processing
        if watched_addresses.is_empty() && !has_alert_subscribers {
            debug!(
                height = block.height,
                "No watched addresses and no alert subscribers, skipping block"
            );
            return Ok(());
        }

        // 5. Fetch and analyze transactions
        let mut fetched_transactions: Vec<Transaction> = Vec::new();

        // Process coinbase transaction (tx[0]) -- block reward (treasury/charity payments)
        if let Some(coinbase_txid) = block.tx.first() {
            match self.rpc.get_raw_transaction(coinbase_txid).await {
                Ok(coinbase_tx) => {
                    if !watched_addresses.is_empty() {
                        self.check_stake_outputs(
                            &block,
                            &coinbase_tx,
                            &watched_addresses,
                            "payment",
                        )
                        .await;
                    }
                    fetched_transactions.push(coinbase_tx);
                }
                Err(e) => {
                    warn!(
                        txid = %coinbase_txid,
                        error = %e,
                        "Failed to fetch coinbase transaction"
                    );
                }
            }
        }

        // Process coinstake transaction (tx[1]) -- staking reward
        if let Some(coinstake_txid) = block.tx.get(1) {
            match self.rpc.get_raw_transaction(coinstake_txid).await {
                Ok(coinstake_tx) => {
                    if !watched_addresses.is_empty() {
                        self.check_stake_outputs(
                            &block,
                            &coinstake_tx,
                            &watched_addresses,
                            "stake",
                        )
                        .await;
                    }
                    fetched_transactions.push(coinstake_tx);
                }
                Err(e) => {
                    warn!(
                        txid = %coinstake_txid,
                        error = %e,
                        "Failed to fetch coinstake transaction"
                    );
                }
            }
        }

        // Fetch remaining transactions for alert analysis if we have subscribers
        if has_alert_subscribers && block.tx.len() > 2 {
            for txid in &block.tx[2..] {
                match self.rpc.get_raw_transaction(txid).await {
                    Ok(tx) => {
                        fetched_transactions.push(tx);
                    }
                    Err(e) => {
                        warn!(
                            txid = %txid,
                            error = %e,
                            "Failed to fetch transaction for alert analysis"
                        );
                    }
                }
            }
        }

        // 6. Check lottery winners
        if !watched_addresses.is_empty() {
            self.check_lottery_winners(&block, &watched_addresses).await;
        }

        // 7. Run anomaly detection on the block
        if has_alert_subscribers || !watched_addresses.is_empty() {
            if let Err(e) = self
                .alert_analyzer
                .analyze_block(&block, &fetched_transactions)
                .await
            {
                warn!(
                    height = block.height,
                    error = %e,
                    "Alert analysis failed"
                );
            }
        }

        debug!(height = block.height, "Block processing complete");

        Ok(())
    }

    /// Check transaction outputs against watched addresses and record/notify
    /// any matching stake events. Deduplicates per address per transaction.
    async fn check_stake_outputs(
        &self,
        block: &crate::rpc::Block,
        tx: &Transaction,
        watched_addresses: &HashSet<String>,
        event_type: &str,
    ) {
        // Collect unique watched addresses that appear in any output
        let mut seen: HashSet<String> = HashSet::new();
        for vout in &tx.vout {
            for addr in self.find_matching_addresses(vout, watched_addresses) {
                seen.insert(addr);
            }
        }

        for address in &seen {
            info!(
                address = %address,
                event_type = event_type,
                txid = %tx.txid,
                height = block.height,
                "Stake reward detected for watched address"
            );

            if let Err(e) = self.record_and_notify(address, block, tx, event_type).await {
                error!(
                    address = %address,
                    error = %e,
                    "Failed to record/notify stake event"
                );
            }
        }
    }

    /// Find addresses in a vout that are in the watched set.
    fn find_matching_addresses(
        &self,
        vout: &Vout,
        watched_addresses: &HashSet<String>,
    ) -> Vec<String> {
        match &vout.script_pub_key.addresses {
            Some(addrs) => addrs
                .iter()
                .filter(|a| watched_addresses.contains(a.as_str()))
                .cloned()
                .collect(),
            None => Vec::new(),
        }
    }

    /// Record a stake event in the database and send a notification to all
    /// users watching the address.
    ///
    /// The amount is the address's own: a block's coinstake also carries the
    /// treasury, charity, masternode and lottery payments, so the whole
    /// transaction's outputs minus inputs is not one address's reward. An
    /// address that spent an input staked: its reward is what came back to the
    /// staking addresses minus what they put in (vault stakes included). An
    /// address that only received an output was paid (treasury and charity in
    /// a treasury block), and gets "Block Payment Received!" for that output.
    async fn record_and_notify(
        &self,
        address: &str,
        block: &crate::rpc::Block,
        tx: &Transaction,
        _event_type: &str,
    ) -> anyhow::Result<()> {
        // What each input spent, and from which address
        let mut inputs: Vec<(Vec<String>, i64)> = Vec::new();
        for vin in &tx.vin {
            if let Some(prev_txid) = &vin.txid {
                let prev_tx = self.rpc.get_raw_transaction(prev_txid).await?;
                let prev_vout = prev_tx
                    .vout
                    .get(vin.vout.unwrap_or(0) as usize)
                    .ok_or_else(|| anyhow::anyhow!("input {prev_txid} has no output {:?}", vin.vout))?;
                inputs.push((
                    prev_vout.script_pub_key.addresses.clone().unwrap_or_default(),
                    to_sats(prev_vout.value),
                ));
            }
        }

        let (event_type, amount_satoshis) = classify(address, tx, &inputs);
        if event_type == "payment" {
            // A lottery win is paid here too: check_lottery_winners reports it as one
            if let Ok(Some(lottery)) = self.rpc.get_lottery_block_winners(&block.hash).await {
                if lottery.winners.iter().any(|w| w.address == address) {
                    return Ok(());
                }
            }
        }
        if amount_satoshis <= 0 {
            warn!(txid = %tx.txid, address, event_type, amount_satoshis, "No positive amount for watched address, not reported");
            return Ok(());
        }

        // Record the stake event in the database.
        db::record_stake_event(
            &self.db,
            address,
            &tx.txid,
            block.height,
            &block.hash,
            amount_satoshis,
            event_type,
        )?;

        // Update last_stake_at and last_stake_height for all watchers of this address.
        db::update_last_stake(&self.db, address, block.height)?;

        // Get users watching this address
        let users = db::get_users_for_address(&self.db, address)?;

        let explorer_url = &self.notifier.explorer_url;

        // Send notifications — include label per user
        for chat_id in &users {
            let label = db::get_watch_label(&self.db, *chat_id, address).unwrap_or(None);
            let label_str = label
                .map(|l| format!(" ({})", teloxide::utils::html::escape(&l)))
                .unwrap_or_default();

            let (title, amount_label) = match event_type {
                "payment" => ("Block Payment Received!", "Amount"),
                "lottery" => ("Lottery Win!", "Prize"),
                _ => ("Stake Reward Detected!", "Reward"),
            };

            let message = format!(
                "<b>{title}</b>\n\n\
                 Address: {}{label_str}\n\
                 {amount_label}: <b>{} DIVI</b>\n\
                 Block: {}\n\
                 Transaction: {}",
                crate::utils::address_link(explorer_url, address),
                satoshi_to_divi(amount_satoshis),
                crate::utils::block_link(explorer_url, block.height),
                crate::utils::tx_link(explorer_url, &tx.txid),
            );

            if let Err(e) = self.notifier.send_message(*chat_id, &message).await {
                warn!(
                    chat_id = chat_id,
                    address = address,
                    error = %e,
                    "Failed to send stake notification"
                );
            }
        }

        info!(
            address = address,
            users_notified = users.len(),
            "Stake event recorded and notifications sent"
        );

        Ok(())
    }

    /// Check lottery block winners against watched addresses.
    async fn check_lottery_winners(
        &self,
        block: &crate::rpc::Block,
        watched_addresses: &HashSet<String>,
    ) {
        match self.rpc.get_lottery_block_winners(&block.hash).await {
            Ok(Some(lottery)) if !lottery.winners.is_empty() => {
                info!(
                    height = block.height,
                    winner_count = lottery.winners.len(),
                    "Lottery block detected"
                );

                // --- Per-address notifications for watched addresses ---
                for winner in &lottery.winners {
                    if watched_addresses.contains(&winner.address) {
                        info!(
                            address = %winner.address,
                            amount = winner.amount,
                            height = block.height,
                            "Lottery winner detected for watched address"
                        );

                        // Record lottery event
                        let amount_satoshis = (winner.amount * 100_000_000.0).round() as i64;
                        if let Err(e) = db::record_stake_event(
                            &self.db,
                            &winner.address,
                            &format!("lottery-{}-{}", block.hash, winner.address),
                            block.height,
                            &block.hash,
                            amount_satoshis,
                            "lottery",
                        ) {
                            warn!(
                                error = %e,
                                "Failed to record lottery event"
                            );
                        }

                        // Update last stake tracking
                        if let Err(e) =
                            db::update_last_stake(&self.db, &winner.address, block.height)
                        {
                            warn!(
                                error = %e,
                                "Failed to update last stake for lottery winner"
                            );
                        }

                        // Notify users watching this address
                        let users = match db::get_users_for_address(&self.db, &winner.address) {
                            Ok(users) => users,
                            Err(e) => {
                                error!(
                                    error = %e,
                                    "Failed to get users for lottery winner"
                                );
                                continue;
                            }
                        };

                        let explorer_url = &self.notifier.explorer_url;
                        let short_addr = crate::utils::truncate_address(&winner.address);
                        let message = format!(
                            "\u{1f3c6} <b>Lottery Winner!</b>\n\n\
                             Address: <a href=\"{explorer_url}/address/{addr}\">{short_addr}</a>\n\
                             Prize: <b>{amount} DIVI</b>\n\
                             Block: <a href=\"{explorer_url}/block/{hash}\">{height}</a>",
                            addr = winner.address,
                            amount =
                                satoshi_to_divi((winner.amount * 100_000_000.0).round() as i64),
                            hash = block.hash,
                            height = block.height,
                        );

                        for chat_id in &users {
                            if let Err(e) = self.notifier.send_message(*chat_id, &message).await {
                                warn!(
                                    chat_id = chat_id,
                                    address = %winner.address,
                                    error = %e,
                                    "Failed to send lottery notification"
                                );
                            }
                        }
                    }
                }

                // --- Lottery block summary for alert subscribers ---
                let lottery_subs = db::get_subscribers_for_alert_type(
                    &self.db,
                    crate::alert_analyzer::ALERT_LOTTERY_BLOCK,
                )
                .unwrap_or_default();
                let anything_subs = db::get_subscribers_for_alert_type(
                    &self.db,
                    crate::alert_analyzer::ALERT_ANYTHING_UNUSUAL,
                )
                .unwrap_or_default();

                let mut subscriber_ids: Vec<i64> =
                    lottery_subs.iter().map(|s| s.telegram_id).collect();
                for sub in &anything_subs {
                    if !subscriber_ids.contains(&sub.telegram_id) {
                        subscriber_ids.push(sub.telegram_id);
                    }
                }

                if !subscriber_ids.is_empty() {
                    let winners_data: Vec<(String, f64)> = lottery
                        .winners
                        .iter()
                        .map(|w| (w.address.clone(), w.amount))
                        .collect();

                    let summary = self.notifier.format_lottery_block_summary(
                        block.height,
                        &block.hash,
                        &winners_data,
                    );

                    for chat_id in &subscriber_ids {
                        if let Err(e) = self.notifier.send_message(*chat_id, &summary).await {
                            warn!(
                                chat_id = chat_id,
                                error = %e,
                                "Failed to send lottery block summary"
                            );
                        }
                    }

                    info!(
                        subscriber_count = subscriber_ids.len(),
                        "Lottery block summary sent"
                    );
                }
            }
            Ok(Some(_)) | Ok(None) => {
                debug!(height = block.height, "No lottery winners for this block");
            }
            Err(e) => {
                // Not all blocks have lottery data; this is expected to fail
                // on non-lottery blocks, so log at debug level.
                debug!(
                    height = block.height,
                    error = %e,
                    "Could not fetch lottery winners"
                );
            }
        }
    }
}

fn to_sats(v: f64) -> i64 {
    (v * 100_000_000.0).round() as i64
}

/// What a coinstake or coinbase transaction gave `address`, given each input's
/// address and value. An address that spent an input staked: its reward is what
/// came back to the staking addresses minus what they put in. Any other address
/// was paid: the treasury and charity in a treasury block, a masternode, a
/// lottery winner.
fn classify(address: &str, tx: &Transaction, inputs: &[(Vec<String>, i64)]) -> (&'static str, i64) {
    let stakers: HashSet<&str> = inputs.iter().flat_map(|(a, _)| a.iter().map(String::as_str)).collect();
    let paid_to = |to: &dyn Fn(&str) -> bool| -> i64 {
        tx.vout
            .iter()
            .filter(|v| v.script_pub_key.addresses.as_ref().is_some_and(|a| a.iter().any(|x| to(x))))
            .map(|v| to_sats(v.value))
            .sum()
    };
    if stakers.contains(address) {
        let total_in: i64 = inputs.iter().map(|(_, v)| v).sum();
        ("stake", paid_to(&|x| stakers.contains(x)) - total_in)
    } else {
        ("payment", paid_to(&|x| x == address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The coinstake of treasury block 4254182: the staker's output, then the
    // treasury and charity payments in the same transaction.
    const TREASURY: &str = "DPhJsztbZafDc1YeyrRqSjmKjkmLJpQpUn";
    const CHARITY: &str = "DPujt2XAdHyRcZNB5ySZBBVKjzY2uXZGYq";
    const STAKER: &str = "DEYpM4d6staker";

    fn coinstake() -> Transaction {
        serde_json::from_value(serde_json::json!({
            "txid": "81b18bc0efe09ef92dc5c18c2f55d1927129dea726e8f53876ee129392fb8f56",
            "vin": [{"txid": "prev", "vout": 1}],
            "vout": [
                {"value": 0.0, "n": 0, "scriptPubKey": {}},
                {"value": 12588.68457031, "n": 1, "scriptPubKey": {"addresses": [STAKER]}},
                {"value": 806480.0, "n": 2, "scriptPubKey": {"addresses": [TREASURY]}},
                {"value": 50405.0, "n": 3, "scriptPubKey": {"addresses": [CHARITY]}},
            ],
        }))
        .unwrap()
    }

    fn inputs() -> Vec<(Vec<String>, i64)> {
        vec![(vec![STAKER.to_string()], to_sats(12173.68457031))]
    }

    #[test]
    fn treasury_and_charity_in_a_coinstake_are_payments() {
        assert_eq!(classify(TREASURY, &coinstake(), &inputs()), ("payment", 80_648_000_000_000));
        assert_eq!(classify(CHARITY, &coinstake(), &inputs()), ("payment", 5_040_500_000_000));
    }

    #[test]
    fn the_staker_gets_its_own_reward() {
        assert_eq!(classify(STAKER, &coinstake(), &inputs()), ("stake", to_sats(415.0)));
    }
}
