use std::time::Duration;

use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::{error, info, warn};

use crate::db::{self, DbPool};
use crate::utils::{format_duration, satoshi_to_divi, truncate_address};

// ---------------------------------------------------------------------------
// Notifier
// ---------------------------------------------------------------------------

/// Formats and delivers Telegram notifications for stake events, lottery wins,
/// missed-stake alerts, fork alerts, and generic blockchain alerts.
pub struct Notifier {
    bot: Bot,
    db: DbPool,
    pub explorer_url: String,
}

impl Notifier {
    pub fn new(bot: Bot, db: DbPool, explorer_url: String) -> Self {
        Self {
            bot,
            db,
            explorer_url,
        }
    }

    // -----------------------------------------------------------------------
    // Message formatting
    // -----------------------------------------------------------------------

    /// Format a staking reward notification (HTML).
    pub fn format_stake_notification(
        &self,
        address: &str,
        label: Option<&str>,
        amount_satoshis: i64,
        block_height: u64,
        txid: &str,
    ) -> String {
        let label_line = match label {
            Some(l) if !l.is_empty() => format!(" ({l})"),
            _ => String::new(),
        };
        format!(
            "<b>Staking Reward Received</b>\n\n\
             Address: <a href=\"{base}/explorer/address/{address}\">{short_addr}</a>{label_line}\n\
             Amount: <b>{amount} DIVI</b>\n\
             Block: <a href=\"{base}/explorer/tx/{txid}\">{block_height}</a>\n\n\
             <a href=\"{base}/explorer/tx/{txid}\">View transaction</a>",
            base = self.explorer_url,
            short_addr = truncate_address(address),
            amount = satoshi_to_divi(amount_satoshis),
        )
    }

    /// Format a lottery / superblock win notification (HTML).
    pub fn format_lottery_notification(
        &self,
        address: &str,
        label: Option<&str>,
        amount_satoshis: i64,
        block_height: u64,
        txid: &str,
    ) -> String {
        let label_line = match label {
            Some(l) if !l.is_empty() => format!(" ({l})"),
            _ => String::new(),
        };
        format!(
            "<b>Lottery Win!</b>\n\n\
             Congratulations! Your address won the lottery!\n\n\
             Address: <a href=\"{base}/explorer/address/{address}\">{short_addr}</a>{label_line}\n\
             Amount: <b>{amount} DIVI</b>\n\
             Block: <a href=\"{base}/explorer/tx/{txid}\">{block_height}</a>\n\n\
             <a href=\"{base}/explorer/tx/{txid}\">View transaction</a>",
            base = self.explorer_url,
            short_addr = truncate_address(address),
            amount = satoshi_to_divi(amount_satoshis),
        )
    }

    /// Format a missed-stake warning notification (HTML).
    pub fn format_missed_stake_alert(
        &self,
        address: &str,
        label: Option<&str>,
        expected_interval_secs: f64,
        time_since_last_secs: u64,
        balance_satoshis: i64,
    ) -> String {
        let label_line = match label {
            Some(l) if !l.is_empty() => format!(" ({l})"),
            _ => String::new(),
        };

        let expected_str = format_duration(expected_interval_secs as u64);
        let elapsed_str = format_duration(time_since_last_secs);
        let overdue_factor = if expected_interval_secs > 0.0 {
            time_since_last_secs as f64 / expected_interval_secs
        } else {
            0.0
        };

        format!(
            "<b>Missed Stake Warning</b>\n\n\
             Address: <a href=\"{base}/explorer/address/{address}\">{short_addr}</a>{label_line}\n\
             Balance: {balance} DIVI\n\n\
             Expected stake every: <b>{expected_str}</b>\n\
             Time since last stake: <b>{elapsed_str}</b> ({overdue_factor:.1}x expected)\n\n\
             Your address may have stopped staking. Please check:\n\
             - Is your wallet running and unlocked for staking?\n\
             - Is your node fully synced?\n\
             - Is your balance still available (not locked)?",
            base = self.explorer_url,
            short_addr = truncate_address(address),
            balance = satoshi_to_divi(balance_satoshis),
        )
    }

    /// Format a fork detection alert (HTML).
    ///
    /// `mismatches` contains tuples of (endpoint_a_name, hash_a, endpoint_b_name, hash_b).
    pub fn format_fork_alert(
        &self,
        height: u64,
        mismatches: &[(String, String, String, String)],
    ) -> String {
        let mut text = format!(
            "<b>FORK DETECTED</b>\n\n\
             Block height: <b>{height}</b>\n\n\
             Endpoints disagree on block hash:\n"
        );

        let explorer = &self.explorer_url;
        for (ep_a, hash_a, ep_b, hash_b) in mismatches {
            text.push_str(&format!(
                "\n  <b>{ep_a}</b>: <a href=\"{explorer}/explorer/block/{hash_a}\">{short_a}</a>\n\
                   <b>{ep_b}</b>: <a href=\"{explorer}/explorer/block/{hash_b}\">{short_b}</a>\n",
                short_a = truncate_address(hash_a),
                short_b = truncate_address(hash_b),
            ));
        }

        text.push_str("\nThis indicates a chain split. Investigate immediately.");

        text
    }

    /// Format a generic blockchain alert (HTML).
    pub fn format_blockchain_alert(
        &self,
        alert_type: &str,
        details: &str,
        txid: Option<&str>,
    ) -> String {
        let tx_link = match txid {
            Some(id) => format!(
                "\n\n<a href=\"{}/tx/{id}\">View transaction</a>",
                self.explorer_url
            ),
            None => String::new(),
        };

        format!(
            "<b>Blockchain Alert</b>\n\n\
             Type: <b>{alert_type}</b>\n\
             {details}{tx_link}"
        )
    }

    // -----------------------------------------------------------------------
    // Delivery
    // -----------------------------------------------------------------------

    /// Send an HTML message to a single Telegram chat. Returns `Ok(())` on
    /// success or if the user has blocked the bot (logged and skipped).
    pub async fn send_message(&self, telegram_id: i64, message: &str) -> Result<()> {
        let chat_id = ChatId(telegram_id);
        match self
            .bot
            .send_message(chat_id, message)
            .parse_mode(ParseMode::Html)
            .link_preview_options(teloxide::types::LinkPreviewOptions {
                is_disabled: true,
                url: None,
                prefer_small_media: false,
                prefer_large_media: false,
                show_above_text: false,
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                let err_str = e.to_string();
                // Handle "Forbidden: bot was blocked by the user" gracefully.
                // Also handle deactivated accounts and chat-not-found errors.
                if err_str.contains("Forbidden")
                    || err_str.contains("blocked")
                    || err_str.contains("deactivated")
                    || err_str.contains("not found")
                {
                    warn!(
                        telegram_id,
                        error = %e,
                        "User blocked/deactivated, skipping notification"
                    );
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Notify all users who are watching a given address. Looks up watchers
    /// in the database and sends the message to each.
    pub async fn notify_users_for_address(&self, address: &str, message: &str) -> Result<()> {
        let user_ids = db::get_users_for_address(&self.db, address)?;

        if user_ids.is_empty() {
            return Ok(());
        }

        info!(
            address,
            user_count = user_ids.len(),
            "Sending notifications for address"
        );

        self.notify_users(&user_ids, message).await
    }

    /// Send a message to a specific list of Telegram user IDs. Respects
    /// Telegram's rate limit (~30 messages/second) by inserting a small
    /// delay between sends when the batch is large.
    pub async fn notify_users(&self, telegram_ids: &[i64], message: &str) -> Result<()> {
        let needs_throttle = telegram_ids.len() > 25;

        for (i, &tid) in telegram_ids.iter().enumerate() {
            if let Err(e) = self.send_message(tid, message).await {
                error!(
                    telegram_id = tid,
                    error = %e,
                    "Failed to deliver notification"
                );
            }

            // Throttle to stay under Telegram's 30 msg/sec limit
            if needs_throttle && (i + 1) % 25 == 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        Ok(())
    }

    /// Send a message to all configured admin users.
    pub async fn notify_admins(&self, message: &str, admin_ids: &[i64]) -> Result<()> {
        if admin_ids.is_empty() {
            return Ok(());
        }

        info!(admin_count = admin_ids.len(), "Sending admin notification");

        self.notify_users(admin_ids, message).await
    }
}
