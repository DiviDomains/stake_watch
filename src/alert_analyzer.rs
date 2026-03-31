use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::db::{self, DbPool};
use crate::notifier::Notifier;
use crate::rpc::{Block, Transaction};

// ---------------------------------------------------------------------------
// Alert type constants
// ---------------------------------------------------------------------------

pub const ALERT_LARGE_TX: &str = "large_tx";
pub const ALERT_LARGE_BLOCK: &str = "large_block";
pub const ALERT_MANY_INPUTS: &str = "many_inputs";
pub const ALERT_MANY_OUTPUTS: &str = "many_outputs";
pub const ALERT_OP_RETURN: &str = "op_return";
pub const ALERT_UNUSUAL_SCRIPT: &str = "unusual_script";
pub const ALERT_ANYTHING_UNUSUAL: &str = "anything_unusual";

/// All valid alert type strings that users can subscribe to.
pub const VALID_ALERT_TYPES: &[&str] = &[
    ALERT_LARGE_TX,
    ALERT_LARGE_BLOCK,
    ALERT_MANY_INPUTS,
    ALERT_MANY_OUTPUTS,
    ALERT_OP_RETURN,
    ALERT_UNUSUAL_SCRIPT,
    ALERT_ANYTHING_UNUSUAL,
];

// ---------------------------------------------------------------------------
// Default thresholds
// ---------------------------------------------------------------------------

pub const DEFAULT_LARGE_TX: f64 = 1_000_000.0; // 1M DIVI
pub const DEFAULT_LARGE_BLOCK: f64 = 10_000_000.0; // 10M DIVI
pub const DEFAULT_MANY_INPUTS: f64 = 10.0;
pub const DEFAULT_MANY_OUTPUTS: f64 = 10.0;

/// Convenience aliases used by the bot's /alerts display.
pub const DEFAULT_LARGE_STAKE_THRESHOLD: f64 = DEFAULT_LARGE_TX;
pub const DEFAULT_WHALE_MOVE_THRESHOLD: f64 = DEFAULT_LARGE_TX;
pub const DEFAULT_LOTTERY_WIN_THRESHOLD: f64 = 0.0;

/// Return the default threshold for a given alert type string.
pub fn default_threshold_for(alert_type: &str) -> f64 {
    match alert_type {
        ALERT_LARGE_TX => DEFAULT_LARGE_TX,
        ALERT_LARGE_BLOCK => DEFAULT_LARGE_BLOCK,
        ALERT_MANY_INPUTS => DEFAULT_MANY_INPUTS,
        ALERT_MANY_OUTPUTS => DEFAULT_MANY_OUTPUTS,
        ALERT_OP_RETURN => 0.0,
        ALERT_UNUSUAL_SCRIPT => 0.0,
        ALERT_ANYTHING_UNUSUAL => 0.0,
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Known "normal" script types that do NOT trigger unusual_script alerts
// ---------------------------------------------------------------------------

const NORMAL_SCRIPT_TYPES: &[&str] = &[
    "pubkeyhash",
    "scripthash",
    "multisig",
    "nulldata",
    "pubkey",
    "witness_v0_keyhash",
    "witness_v0_scripthash",
    "nonstandard",
];

// ---------------------------------------------------------------------------
// AlertAnalyzer
// ---------------------------------------------------------------------------

/// Detected anomaly with enough context to notify subscribers.
#[allow(dead_code)]
struct Anomaly {
    alert_type: &'static str,
    description: String,
    value: f64,
    default_threshold: f64,
}

/// Analyzes each block's transactions for anomalous patterns and notifies
/// subscribed users when thresholds are exceeded.
pub struct AlertAnalyzer {
    db: DbPool,
    notifier: Arc<Notifier>,
}

impl AlertAnalyzer {
    pub fn new(db: DbPool, notifier: Arc<Notifier>) -> Self {
        Self { db, notifier }
    }

    /// Analyze a block and its transactions for anomalies. For each detected
    /// anomaly, look up subscribers and send notifications.
    pub async fn analyze_block(
        &self,
        block: &Block,
        transactions: &[Transaction],
    ) -> anyhow::Result<()> {
        let mut anomalies: Vec<Anomaly> = Vec::new();

        // ---- Per-transaction analysis ----
        let mut block_total_value: f64 = 0.0;

        for tx in transactions {
            let tx_total: f64 = tx.vout.iter().map(|v| v.value).sum();
            block_total_value += tx_total;

            let input_count = tx.vin.len();
            let output_count = tx.vout.len();

            // Large transaction
            if tx_total >= DEFAULT_LARGE_TX {
                anomalies.push(Anomaly {
                    alert_type: ALERT_LARGE_TX,
                    description: format!(
                        "Large transaction detected: {:.2} DIVI in tx {}",
                        tx_total, tx.txid
                    ),
                    value: tx_total,
                    default_threshold: DEFAULT_LARGE_TX,
                });
            }

            // Many inputs
            if input_count as f64 >= DEFAULT_MANY_INPUTS {
                anomalies.push(Anomaly {
                    alert_type: ALERT_MANY_INPUTS,
                    description: format!("Transaction with {} inputs: tx {}", input_count, tx.txid),
                    value: input_count as f64,
                    default_threshold: DEFAULT_MANY_INPUTS,
                });
            }

            // Many outputs
            if output_count as f64 >= DEFAULT_MANY_OUTPUTS {
                anomalies.push(Anomaly {
                    alert_type: ALERT_MANY_OUTPUTS,
                    description: format!(
                        "Transaction with {} outputs: tx {}",
                        output_count, tx.txid
                    ),
                    value: output_count as f64,
                    default_threshold: DEFAULT_MANY_OUTPUTS,
                });
            }

            // OP_RETURN detection
            for vout in &tx.vout {
                if let Some(ref asm) = vout.script_pub_key.asm {
                    if asm.starts_with("OP_RETURN") {
                        anomalies.push(Anomaly {
                            alert_type: ALERT_OP_RETURN,
                            description: format!(
                                "OP_RETURN output in tx {} (output #{})",
                                tx.txid, vout.n
                            ),
                            value: 1.0,
                            default_threshold: 0.0,
                        });
                    }
                }

                // Unusual script type detection
                if let Some(ref script_type) = vout.script_pub_key.script_type {
                    let is_normal = NORMAL_SCRIPT_TYPES.contains(&script_type.as_str());
                    if !is_normal {
                        anomalies.push(Anomaly {
                            alert_type: ALERT_UNUSUAL_SCRIPT,
                            description: format!(
                                "Unusual script type '{}' in tx {} (output #{})",
                                script_type, tx.txid, vout.n
                            ),
                            value: 1.0,
                            default_threshold: 0.0,
                        });
                    }
                }
            }
        }

        // ---- Block-level analysis ----
        if block_total_value >= DEFAULT_LARGE_BLOCK {
            anomalies.push(Anomaly {
                alert_type: ALERT_LARGE_BLOCK,
                description: format!(
                    "Large block detected: {:.2} DIVI total in block {} (height {})",
                    block_total_value, block.hash, block.height
                ),
                value: block_total_value,
                default_threshold: DEFAULT_LARGE_BLOCK,
            });
        }

        if anomalies.is_empty() {
            debug!(
                block_height = block.height,
                "No anomalies detected in block"
            );
            return Ok(());
        }

        info!(
            block_height = block.height,
            anomaly_count = anomalies.len(),
            "Anomalies detected in block"
        );

        // ---- Notify subscribers ----
        self.notify_subscribers(block, &anomalies).await
    }

    /// For each anomaly, find subscribers for the specific alert type AND
    /// subscribers for "anything_unusual", then send notifications.
    async fn notify_subscribers(&self, block: &Block, anomalies: &[Anomaly]) -> anyhow::Result<()> {
        // Collect all "anything_unusual" subscribers once
        let anything_subs = db::get_subscribers_for_alert_type(&self.db, ALERT_ANYTHING_UNUSUAL)
            .unwrap_or_default();
        let anything_subscribers: Vec<i64> = anything_subs.iter().map(|s| s.telegram_id).collect();

        for anomaly in anomalies {
            // Get direct subscribers for this specific alert type
            let type_subs = db::get_subscribers_for_alert_type(&self.db, anomaly.alert_type)
                .unwrap_or_default();
            let type_subscribers: Vec<i64> = type_subs.iter().map(|s| s.telegram_id).collect();

            // Merge subscriber lists (deduplicate by chat_id)
            let mut all_chat_ids: Vec<i64> = type_subscribers;
            for &chat_id in &anything_subscribers {
                if !all_chat_ids.contains(&chat_id) {
                    all_chat_ids.push(chat_id);
                }
            }

            if all_chat_ids.is_empty() {
                debug!(
                    alert_type = anomaly.alert_type,
                    "No subscribers for alert type, skipping"
                );
                continue;
            }

            // Format the notification message
            let message = format!(
                "\u{26a0}\u{fe0f} *Block Alert* (height {})\n\n\
                 Type: `{}`\n\
                 {}\n\n\
                 Block: `{}`",
                block.height, anomaly.alert_type, anomaly.description, block.hash,
            );

            for chat_id in &all_chat_ids {
                if let Err(e) = self.notifier.send_message(*chat_id, &message).await {
                    warn!(
                        chat_id = chat_id,
                        alert_type = anomaly.alert_type,
                        error = %e,
                        "Failed to send alert notification"
                    );
                }
            }

            info!(
                alert_type = anomaly.alert_type,
                subscriber_count = all_chat_ids.len(),
                "Alert notifications sent"
            );
        }

        Ok(())
    }
}
