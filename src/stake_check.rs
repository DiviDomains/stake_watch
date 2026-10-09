//! `/check`: a plain-language staking check for one address.
//!
//! Answers the questions a staker actually asks: is anything wrong, when should
//! my next stake come, and what will it earn. The math lives here as pure
//! functions so it can be tested without a node or Telegram.

/// Divi targets one block a minute.
pub const SECS_PER_BLOCK: f64 = 60.0;
pub const BLOCKS_PER_DAY: f64 = 1440.0;
pub const BLOCKS_PER_YEAR: f64 = 525_600.0;

/// How far back the stake backfill looks (`StakeAnalyzer::backfill_stakes`).
pub const BACKFILL_BLOCKS: u64 = 50_000;

/// Coins must be this many blocks old (1 hour) before they can stake.
pub const MIN_STAKE_AGE_BLOCKS: u64 = 60;

/// Staker reward used when no recent stake has been recorded to measure it from.
pub const FALLBACK_STAKE_REWARD_SATOSHIS: i64 = 415 * 100_000_000;

/// A gap with less than this chance of happening by luck means something is wrong.
const OVERDUE_CHANCE: f64 = 0.01;
/// A gap with less than this chance is unusual but still most likely luck.
const SLOW_CHANCE: f64 = 0.10;

/// What a wallet of a given size should expect from staking.
#[derive(Debug, Clone, PartialEq)]
pub struct Expectation {
    /// Average number of blocks between stakes.
    pub blocks_per_stake: f64,
    /// Chance of at least one stake in the next 30 days.
    pub chance_30d: f64,
    /// Chance of at least one stake in the next year.
    pub chance_1y: f64,
    /// Average DIVI earned per year.
    pub yearly_divi: f64,
    /// `yearly_divi` as a share of the balance.
    pub yearly_rate: f64,
}

/// Each block goes to one staker, picked in proportion to the coins they stake,
/// so a wallet's stakes arrive as a Poisson process with rate balance / network.
pub fn expectation(
    balance_satoshis: i64,
    network_staking_supply: u64,
    reward_satoshis: i64,
) -> Option<Expectation> {
    if balance_satoshis <= 0 || network_staking_supply == 0 {
        return None;
    }
    let balance = balance_satoshis as f64 / 1e8;
    let share = balance / network_staking_supply as f64;
    let blocks_per_stake = 1.0 / share;
    let reward = reward_satoshis as f64 / 1e8;
    let yearly_divi = BLOCKS_PER_YEAR * share * reward;
    Some(Expectation {
        blocks_per_stake,
        chance_30d: chance_within(30.0 * BLOCKS_PER_DAY, blocks_per_stake),
        chance_1y: chance_within(BLOCKS_PER_YEAR, blocks_per_stake),
        yearly_divi,
        yearly_rate: yearly_divi / balance,
    })
}

/// Chance of at least one stake within `blocks`.
pub fn chance_within(blocks: f64, blocks_per_stake: f64) -> f64 {
    1.0 - (-blocks / blocks_per_stake).exp()
}

/// Chance that a healthy wallet goes `blocks` without a stake.
pub fn chance_of_gap(blocks: f64, blocks_per_stake: f64) -> f64 {
    (-blocks / blocks_per_stake).exp()
}

/// Chance that a healthy wallet expecting `mean` stakes gets `k` or fewer.
pub fn poisson_at_most(k: u64, mean: f64) -> f64 {
    let mut term = (-mean).exp();
    let mut sum = term;
    for i in 1..=k {
        term *= mean / i as f64;
        sum += term;
    }
    sum.min(1.0)
}

/// Everything `/check` knows about one address.
#[derive(Debug, Clone)]
pub struct CheckInput {
    pub address: String,
    pub label: Option<String>,
    pub balance_satoshis: i64,
    pub is_vault: bool,
    /// Heights of the address's unspent coins, when the backend can list them.
    pub coin_heights: Option<Vec<u64>>,
    pub current_height: u64,
    /// Heights of recorded stakes, newest first.
    pub stake_heights: Vec<u64>,
    /// How many blocks back the stake history is complete.
    pub history_blocks: u64,
    pub network_staking_supply: u64,
    pub reward_satoshis: i64,
}

/// Render the check as Telegram HTML.
pub fn render(c: &CheckInput) -> String {
    let mut out = String::from("<b>Stake check</b>");
    if let Some(label) = c.label.as_deref().filter(|l| !l.is_empty()) {
        out.push_str(&format!(" — {}", escape(label)));
    }
    out.push_str(&format!("\n<code>{}</code>\n\n", c.address));

    if c.balance_satoshis <= 0 {
        out.push_str(
            "This address holds no DIVI, so it can't stake.\n\
             Send coins to it and keep the wallet open and unlocked for staking.",
        );
        return out;
    }

    // Balance. The wallet holds it as separate coins (UTXOs): every deposit, stake reward
    // and bit of change is one. Users read "coins" as transactions, so only the young ones
    // are mentioned, as payments that can't stake yet.
    let vault = if c.is_vault { " (vault)" } else { "" };
    out.push_str(&format!(
        "<b>Balance:</b> {} DIVI{vault}\n",
        fmt_divi(c.balance_satoshis as f64 / 1e8, 2)
    ));
    let coins = c.coin_heights.as_deref().unwrap_or(&[]);
    let young = coins
        .iter()
        .filter(|h| c.current_height.saturating_sub(**h) < MIN_STAKE_AGE_BLOCKS)
        .count();
    if young > 0 {
        let (n, it) = if young == 1 {
            ("1 payment".to_string(), "It")
        } else {
            (format!("{young} payments"), "They")
        };
        out.push_str(&format!(
            "{n} into this wallet (a deposit, stake reward or change) arrived less than an hour ago. \
             {it} can stake once an hour old.\n"
        ));
    } else if !coins.is_empty() {
        out.push_str("Everything in it is old enough to stake.\n");
    }
    out.push('\n');

    let Some(exp) = expectation(
        c.balance_satoshis,
        c.network_staking_supply,
        c.reward_satoshis,
    ) else {
        return out;
    };

    // Verdict
    let oldest_coin_age = coins
        .iter()
        .map(|h| c.current_height.saturating_sub(*h))
        .max();
    match c.stake_heights.first() {
        None => {
            // No stake on record. Count the dry spell from when the coins
            // arrived, if that is more recent than the start of the history.
            let (waited, since) = match oldest_coin_age {
                Some(age) if age < c.history_blocks => (age, "your coins arrived"),
                _ => (c.history_blocks, ""),
            };
            let chance = chance_of_gap(waited as f64, exp.blocks_per_stake);
            if chance < OVERDUE_CHANCE {
                out.push_str(&overdue(waited, chance));
            } else {
                out.push_str("⏳ <b>Waiting for its first stake.</b> Nothing is wrong.\n");
                if since.is_empty() {
                    out.push_str(&format!(
                        "No stakes in the last {}.\n",
                        human_span(waited as f64 * SECS_PER_BLOCK)
                    ));
                } else {
                    out.push_str(&format!(
                        "No stakes yet since {since} {} ago.\n",
                        human_span(waited as f64 * SECS_PER_BLOCK)
                    ));
                }
            }
        }
        Some(&last) => {
            let gap = c.current_height.saturating_sub(last);
            let chance = chance_of_gap(gap as f64, exp.blocks_per_stake);
            let last_str = human_span(gap as f64 * SECS_PER_BLOCK);
            let span = (c.history_blocks as f64).min(30.0 * BLOCKS_PER_DAY);
            let recent = c
                .stake_heights
                .iter()
                .filter(|h| (c.current_height.saturating_sub(**h) as f64) < span)
                .count();
            let expected = span / exp.blocks_per_stake;
            if chance < OVERDUE_CHANCE {
                out.push_str(&overdue(gap, chance));
            } else if poisson_at_most(recent as u64, expected) < OVERDUE_CHANCE {
                out.push_str(&format!(
                    "🟡 <b>Staking only part of the time.</b> Last stake {last_str} ago, \
                     but far fewer stakes than a wallet this size gets.\n\
                     The wallet may be closed, out of sync or locked some of the time.\n"
                ));
            } else if chance < SLOW_CHANCE {
                out.push_str(&format!(
                    "🟡 <b>A slow patch.</b> Last stake {last_str} ago. \
                     Only {} of wallets this size go that long, but it's most likely luck.\n",
                    fmt_pct(chance)
                ));
            } else {
                out.push_str(&format!("✅ <b>On track.</b> Last stake {last_str} ago.\n"));
            }
            if expected >= 1.0 {
                out.push_str(&format!(
                    "{recent} stake{} in the last {} (about {} expected).\n",
                    if recent == 1 { "" } else { "s" },
                    human_span(span * SECS_PER_BLOCK),
                    expected.round() as u64
                ));
            }
        }
    }

    // What to expect
    out.push_str("\n<b>What to expect</b>\n");
    out.push_str(&format!(
        "Each stake pays about {} DIVI.\n",
        fmt_divi(c.reward_satoshis as f64 / 1e8, 0)
    ));
    out.push_str(&format!(
        "A wallet this size stakes about {}.\n",
        human_every(exp.blocks_per_stake * SECS_PER_BLOCK)
    ));
    if exp.chance_30d < 0.99 {
        out.push_str(&format!(
            "Chance of a stake in the next 30 days: {}. In the next year: {}.\n",
            fmt_pct(exp.chance_30d),
            fmt_pct(exp.chance_1y)
        ));
    }
    out.push_str(&format!(
        "Over time that averages about {} DIVI a year ({}).\n",
        fmt_divi(exp.yearly_divi, 0),
        fmt_pct(exp.yearly_rate)
    ));
    if exp.blocks_per_stake > 30.0 * BLOCKS_PER_DAY {
        out.push_str(
            "Rewards come in rare, large payments rather than a steady trickle; \
             a bigger balance stakes more often.\n",
        );
    }

    out.push('\n');
    if c.is_vault {
        out.push_str("A vault stakes on its staking node, which must stay online.");
    } else {
        out.push_str("Your wallet stakes only while it is open, synced and unlocked for staking.");
    }
    out
}

fn overdue(blocks: u64, chance: f64) -> String {
    format!(
        "⚠️ <b>Overdue.</b> No stake in {}. Fewer than {} of wallets this size go that long.\n\
         Check that the wallet is open, synced and unlocked for staking.\n",
        human_span(blocks as f64 * SECS_PER_BLOCK),
        fmt_pct(chance.max(0.001))
    )
}

/// "once every 3.4 years", "about 5 times a day".
pub fn human_every(secs: f64) -> String {
    let day = 86_400.0;
    if secs < day {
        let per_day = day / secs;
        if per_day >= 1.5 {
            return format!("{} times a day", per_day.round() as u64);
        }
        return "once a day".to_string();
    }
    format!("once every {}", human_span(secs))
}

/// "15 hours", "9 days", "3.4 years".
pub fn human_span(secs: f64) -> String {
    let minutes = secs / 60.0;
    let hours = minutes / 60.0;
    let days = hours / 24.0;
    let (n, unit) = if minutes < 90.0 {
        (minutes.round(), "minute")
    } else if hours < 48.0 {
        (hours.round(), "hour")
    } else if days < 60.0 {
        (days.round(), "day")
    } else if days < 730.0 {
        (((days / 30.44) * 10.0).round() / 10.0, "month")
    } else {
        (((days / 365.25) * 10.0).round() / 10.0, "year")
    };
    let n_str = if n.fract() == 0.0 {
        format!("{}", n as u64)
    } else {
        format!("{n:.1}")
    };
    format!("{n_str} {unit}{}", if n == 1.0 { "" } else { "s" })
}

/// "33%", "2.1%", "0.4%".
pub fn fmt_pct(x: f64) -> String {
    let p = x * 100.0;
    if p >= 10.0 {
        format!("{}%", p.round() as u64)
    } else {
        format!("{p:.1}%")
    }
}

/// Thousands separators and fixed decimals: 2258.8346 -> "2,258.83".
pub fn fmt_divi(x: f64, decimals: usize) -> String {
    let s = format!("{x:.decimals$}");
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s.clone(), None),
    };
    let (sign, digits) = match int.strip_prefix('-') {
        Some(d) => ("-", d.to_string()),
        None => ("", int),
    };
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUPPLY: u64 = 4_000_000_000;
    const REWARD: i64 = 415 * 100_000_000;

    fn small_new_wallet() -> CheckInput {
        // The wallet that prompted this command: ~2,259 DIVI in 3 coins that
        // arrived ~900 blocks ago, never staked.
        CheckInput {
            address: "DExampleSmallNewWalletAddress00000".into(),
            label: Some("Desktop".into()),
            balance_satoshis: 225_883_465_313,
            is_vault: false,
            coin_heights: Some(vec![4_250_567, 4_250_583, 4_250_591]),
            current_height: 4_251_473,
            stake_heights: vec![],
            history_blocks: BACKFILL_BLOCKS,
            network_staking_supply: SUPPLY,
            reward_satoshis: REWARD,
        }
    }

    #[test]
    fn expectation_small_wallet() {
        let e = expectation(225_883_465_313, SUPPLY, REWARD).unwrap();
        let years = e.blocks_per_stake / BLOCKS_PER_YEAR;
        assert!((years - 3.37).abs() < 0.02, "{years}");
        assert!((e.chance_1y - 0.257).abs() < 0.005, "{}", e.chance_1y);
        assert!((e.yearly_rate - 0.0545).abs() < 0.001, "{}", e.yearly_rate);
    }

    #[test]
    fn expectation_rejects_empty() {
        assert!(expectation(0, SUPPLY, REWARD).is_none());
        assert!(expectation(1, 0, REWARD).is_none());
    }

    #[test]
    fn new_small_wallet_is_waiting_not_broken() {
        let text = render(&small_new_wallet());
        assert!(text.contains("Waiting for its first stake"), "{text}");
        assert!(
            text.contains("since your coins arrived 15 hours ago"),
            "{text}"
        );
        assert!(
            text.contains("Everything in it is old enough to stake"),
            "{text}"
        );
        assert!(text.contains("2,258.83 DIVI\n"), "{text}");
        assert!(text.contains("once every 3.4 years"), "{text}");
        assert!(text.contains("about 415 DIVI"), "{text}");
        assert!(!text.contains("Overdue"), "{text}");
    }

    #[test]
    fn young_coin_is_reported() {
        let mut c = small_new_wallet();
        c.coin_heights = Some(vec![4_250_567, 4_251_450]);
        let text = render(&c);
        assert!(
            text.contains("1 payment into this wallet (a deposit, stake reward or change) arrived less than an hour ago. It can stake"),
            "{text}"
        );
    }

    #[test]
    fn big_wallet_on_track_and_overdue() {
        // 4.29M DIVI stakes about every 0.65 days.
        let mut c = small_new_wallet();
        c.balance_satoshis = 4_291_993 * 100_000_000;
        c.coin_heights = None;
        c.stake_heights = vec![4_251_000, 4_250_000];
        let text = render(&c);
        assert!(text.contains("Staking only part of the time"), "{text}");
        assert!(text.contains("2 stakes in the last 30 days"), "{text}");

        // About 46 expected in 30 days; 40 recent ones is normal luck.
        c.stake_heights = (0..40).map(|i| 4_251_000 - i * 1000).collect();
        let text = render(&c);
        assert!(text.contains("On track"), "{text}");
        assert!(
            text.contains("40 stakes in the last 30 days (about 46 expected)"),
            "{text}"
        );

        c.stake_heights = vec![4_251_473 - 10 * 1440];
        let text = render(&c);
        assert!(text.contains("Overdue"), "{text}");
        assert!(text.contains("No stake in 10 days"), "{text}");
    }

    #[test]
    fn big_wallet_with_no_stakes_is_overdue() {
        let mut c = small_new_wallet();
        c.balance_satoshis = 4_291_993 * 100_000_000;
        c.coin_heights = Some(vec![4_200_000]);
        let text = render(&c);
        assert!(text.contains("Overdue"), "{text}");
    }

    #[test]
    fn empty_address() {
        let mut c = small_new_wallet();
        c.balance_satoshis = 0;
        assert!(render(&c).contains("holds no DIVI"));
    }

    #[test]
    fn poisson() {
        assert!((poisson_at_most(0, 2.0) - (-2.0f64).exp()).abs() < 1e-12);
        assert!((poisson_at_most(1, 1.0) - 2.0 * (-1.0f64).exp()).abs() < 1e-12);
        assert!(poisson_at_most(2, 46.0) < 1e-15);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_divi(2_258.834_653_13, 2), "2,258.83");
        assert_eq!(fmt_divi(415.0, 0), "415");
        assert_eq!(fmt_divi(1_234_567.0, 0), "1,234,567");
        assert_eq!(fmt_pct(0.257), "26%");
        assert_eq!(fmt_pct(0.021), "2.1%");
        assert_eq!(human_span(906.0 * 60.0), "15 hours");
        assert_eq!(human_span(86_400.0), "24 hours");
        assert_eq!(human_span(10.0 * 86_400.0), "10 days");
        assert_eq!(human_every(0.65 * 86_400.0), "2 times a day");
        assert_eq!(human_every(3600.0), "24 times a day");
    }
}
