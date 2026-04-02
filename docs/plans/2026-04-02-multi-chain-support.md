# Multi-Chain Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make stake_watch chain-agnostic so a single codebase supports both Divi and PIVX (and future PoS forks) via configuration.

**Architecture:** Add a `[chain]` config section that parameterizes all chain-specific values (name, ticker, address prefixes, reward amounts, special addresses, feature flags). Replace every hardcoded Divi reference with config-driven lookups. Feature-gate lottery and vault support behind config booleans.

**Tech Stack:** Rust, TOML config, existing teloxide/reqwest/rusqlite stack. No new dependencies.

---

## Summary of Divi-Specific Hardcodes to Replace

| File | Hardcode | Replace With |
|------|----------|-------------|
| `config.rs` | `default_staking_supply() -> 3_000_000` | `chain.network_staking_supply` |
| `stake_analyzer.rs` | `TREASURY_ADDRESS`, `CHARITY_ADDRESS` constants | `chain.excluded_addresses[]` |
| `stake_analyzer.rs` | Comment "The Divi staking model" | Generic comment |
| `notifier.rs` | `"DIVI"` in all 6+ notification messages | `chain.ticker` |
| `notifier.rs` | `"Divi Lottery Block Winners"` | Conditional on `chain.has_lottery` |
| `alert_analyzer.rs` | `"1M DIVI"`, `"10M DIVI"` in comments | `chain.ticker` |
| `rpc.rs` | `ChainzClient::validate_address` checks `D`/`y` prefix | `chain.address_prefixes[]` |
| `rpc.rs` | `get_lottery_block_winners` always called | Conditional on `chain.has_lottery` |
| `rpc.rs` | `get_vault_balance/deltas` always in trait | Conditional on `chain.has_vaults` |
| `block_processor.rs` | Lottery winner processing | Conditional on `chain.has_lottery` |
| `block_processor.rs` | Vault stake detection | Conditional on `chain.has_vaults` |
| `bot.rs` | `"DIVI"` in user-facing messages | `chain.ticker` |
| `utils.rs` | `satoshi_to_divi` function name | Keep name (internal), use `chain.ticker` for display |

---

### Task 1: Add `ChainConfig` to config.rs

**Files:**
- Modify: `src/config.rs`

**Step 1: Add the ChainConfig struct and wire it into AppConfig**

```rust
// Add to config.rs after the existing structs

#[derive(Debug, Clone, Deserialize)]
pub struct ChainConfig {
    #[serde(default = "default_chain_name")]
    pub name: String,
    #[serde(default = "default_chain_ticker")]
    pub ticker: String,
    /// Valid address prefixes for offline validation (e.g., ["D"] for mainnet)
    #[serde(default = "default_address_prefixes")]
    pub address_prefixes: Vec<String>,
    /// Addresses that receive block rewards but are NOT staking (treasury, charity, etc.)
    /// These are excluded from missed-stake alerts.
    #[serde(default)]
    pub excluded_addresses: Vec<String>,
    #[serde(default = "default_true")]
    pub has_lottery: bool,
    #[serde(default = "default_true")]
    pub has_vaults: bool,
    #[serde(default)]
    pub has_masternodes: bool,
    /// Block time in seconds (used for staking frequency calculation)
    #[serde(default = "default_block_time")]
    pub block_time_secs: u64,
}

fn default_chain_name() -> String { "Divi".to_string() }
fn default_chain_ticker() -> String { "DIVI".to_string() }
fn default_address_prefixes() -> Vec<String> { vec!["D".to_string(), "y".to_string()] }
fn default_true() -> bool { true }
fn default_block_time() -> u64 { 60 }
```

And add to `AppConfig`:
```rust
pub struct AppConfig {
    pub general: GeneralConfig,
    pub backend: BackendConfig,
    #[serde(default)]
    pub chain: ChainConfig,
    #[serde(default)]
    pub fork_detection: ForkDetectionConfig,
}
```

Move `network_staking_supply` out of `GeneralConfig` and into `ChainConfig` (keep a fallback in GeneralConfig for backwards compat).

**Step 2: Run tests to verify config parsing doesn't break**

Run: `cargo test -- --test-threads=1`
Expected: All existing tests pass (ChainConfig has defaults matching current Divi behavior)

**Step 3: Commit**

```bash
git add src/config.rs
git commit -m "feat: add ChainConfig struct for multi-chain support"
```

---

### Task 2: Replace hardcoded addresses in stake_analyzer.rs

**Files:**
- Modify: `src/stake_analyzer.rs`

**Step 1: Replace TREASURY_ADDRESS/CHARITY_ADDRESS with config-driven lookup**

Remove the hardcoded constants:
```rust
// REMOVE these:
pub const TREASURY_ADDRESS: &str = "DPhJsztbZafDc1YeyrRqSjmKjkmLJpQpUn";
pub const CHARITY_ADDRESS: &str = "DPujt2XAdHyRcZNB5ySZBBVKjzY2uXZGYq";
```

Replace `event_type_for_address` to take excluded addresses from config:
```rust
/// Return the appropriate event type for a coinbase output to the given address.
/// Excluded addresses (treasury, charity, etc.) receive "payment" type; all others receive "stake".
pub fn event_type_for_address(address: &str, excluded_addresses: &[String]) -> &'static str {
    if excluded_addresses.iter().any(|a| a == address) {
        "payment"
    } else {
        "stake"
    }
}
```

**Step 2: Update StakeAnalyzer to accept ChainConfig**

Change the `StakeAnalyzer` struct to store `ChainConfig` (or at least the excluded_addresses and network_staking_supply). Update `check_missed_stakes` to use `self.config.chain.excluded_addresses` instead of the hardcoded constants.

Update `compute_expected_interval` to accept `block_time_secs` parameter:
```rust
pub fn compute_expected_interval(balance_satoshis: i64, network_supply: u64, block_time_secs: u64) -> f64 {
    // ... existing logic but use block_time_secs instead of hardcoded 60
    let expected_secs = expected_blocks * block_time_secs as f64;
    // ...
}
```

**Step 3: Update all callers of event_type_for_address and compute_expected_interval**

Search for all call sites and pass the config values through.

**Step 4: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: Existing tests pass (update test cases to pass `60` for block_time_secs)

**Step 5: Commit**

```bash
git add src/stake_analyzer.rs src/block_processor.rs src/bot.rs
git commit -m "refactor: replace hardcoded Divi addresses with config-driven excluded_addresses"
```

---

### Task 3: Replace hardcoded "DIVI" ticker in notifier.rs and bot.rs

**Files:**
- Modify: `src/notifier.rs`
- Modify: `src/bot.rs`

**Step 1: Add ticker field to Notifier**

```rust
pub struct Notifier {
    bot: Bot,
    db: DbPool,
    pub explorer_url: String,
    pub ticker: String,  // NEW
}
```

**Step 2: Replace all `"DIVI"` string literals in notification formats**

In `notifier.rs`, replace every occurrence of `DIVI` in format strings with `{ticker}`:
- `format_stake_notification`: `"{amount} DIVI"` -> `"{amount} {ticker}"`
- `format_lottery_notification`: same
- `format_missed_stake_alert`: `"DIVI"` -> `"{ticker}"`
- `format_lottery_block_summary`: `"Divi Lottery Block Winners"` -> `"{name} Lottery Block Winners"` (or skip entirely if `!has_lottery`)
- `format_blockchain_alert`: similar

**Step 3: Replace all `"DIVI"` in bot.rs user-facing messages**

Search for `DIVI` in bot.rs and replace with `self.ticker` or equivalent passed from config.

**Step 4: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 5: Commit**

```bash
git add src/notifier.rs src/bot.rs
git commit -m "refactor: replace hardcoded DIVI ticker with config-driven chain.ticker"
```

---

### Task 4: Feature-gate lottery and vault support

**Files:**
- Modify: `src/block_processor.rs`
- Modify: `src/rpc.rs`
- Modify: `src/stake_analyzer.rs`
- Modify: `src/bot.rs`

**Step 1: Pass chain config flags to BlockProcessor**

Add `has_lottery: bool` and `has_vaults: bool` fields to `BlockProcessor`.

**Step 2: Gate lottery processing in block_processor.rs**

Wrap the lottery winner check in `process_block`:
```rust
// Only check lottery winners if the chain supports it
if self.has_lottery && !watched_addresses.is_empty() {
    self.check_lottery_winners(&block, &watched_addresses).await;
}
```

**Step 3: Gate vault balance lookups in stake_analyzer.rs**

In `check_missed_stakes`, wrap the vault balance fallback:
```rust
if regular > 0 {
    regular
} else if self.has_vaults {
    match self.rpc.get_vault_balance(&watch.address).await {
        Ok(vb) if vb.balance > 0 => vb.balance,
        _ => continue,
    }
} else {
    continue;
}
```

**Step 4: Gate vault backfill in stake_analyzer.rs**

In `backfill_stakes`, skip vault delta attempts when `has_vaults` is false.

**Step 5: Make RPC trait methods return graceful errors for unsupported operations**

The trait methods `get_lottery_block_winners`, `get_vault_balance`, `get_vault_deltas` already return `Result`/`Option` and the callers handle errors gracefully. No trait changes needed — the gating in steps 2-4 prevents the calls entirely.

**Step 6: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 7: Commit**

```bash
git add src/block_processor.rs src/stake_analyzer.rs src/rpc.rs src/bot.rs
git commit -m "feat: feature-gate lottery and vault support behind chain config flags"
```

---

### Task 5: Update ChainzClient address validation

**Files:**
- Modify: `src/rpc.rs`

**Step 1: Make address validation use configured prefixes**

The `ChainzClient::validate_address` currently hardcodes `D`/`y`. Since the ChainzClient doesn't have access to config, the simplest approach is to make the `RpcClient` trait's `validate_address` take optional prefixes, or better: since PIVX will use `JsonRpcClient` (direct RPC), the ChainzClient validation is only for Divi anyway.

Best approach: add `address_prefixes` to `ChainzClient::new()`:
```rust
pub struct ChainzClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
    address_prefixes: Vec<String>,
}
```

Update `validate_address`:
```rust
let is_valid = self.address_prefixes.iter().any(|p| address.starts_with(p.as_str()))
    && (25..=34).contains(&address.len())
    && address.chars().all(|c| base58_chars.contains(c));
```

Update `create_rpc_client` to pass prefixes from config.

**Step 2: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 3: Commit**

```bash
git add src/rpc.rs
git commit -m "refactor: make ChainzClient address validation use configured prefixes"
```

---

### Task 6: Add PIVX config file and update alert_analyzer.rs

**Files:**
- Create: `config/pivx-node.toml`
- Modify: `src/alert_analyzer.rs`

**Step 1: Create PIVX config file**

```toml
[chain]
name = "PIVX"
ticker = "PIV"
address_prefixes = ["D", "S"]
excluded_addresses = []
has_lottery = false
has_vaults = false
has_masternodes = true
block_time_secs = 60

[general]
db_path = "./data/stake_watch_pivx.db"
network_staking_supply = 50_000_000
alert_multiplier = 3
alert_check_interval_secs = 300
max_watches_per_user = 20

[backend]
type = "polling"
rpc_url = "http://127.0.0.1:51473"
explorer_url = "https://pivx-explorer.com"

[backend.polling]
interval_secs = 60

[backend.rpc_auth]
enabled = true

[fork_detection]
enabled = false
```

**Step 2: Replace "DIVI" in alert_analyzer.rs comments and defaults**

The `DEFAULT_LARGE_TX` comment says "1M DIVI" — make comments generic or reference the ticker. The actual threshold values are numeric and chain-agnostic, so only comments need updating.

**Step 3: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 4: Commit**

```bash
git add config/pivx-node.toml src/alert_analyzer.rs
git commit -m "feat: add PIVX config file and generalize alert_analyzer comments"
```

---

### Task 7: Enable addressindex on PIVX node and test

**Files:**
- Modify (remote): `~/.pivx/pivx.conf` on dnsdivi

**Step 1: Add addressindex to PIVX node config**

SSH to dnsdivi and add to `~/.pivx/pivx.conf`:
```
addressindex=1
txindex=1
```

**Step 2: Restart PIVX node to rebuild indexes**

```bash
sudo systemctl stop pivxd
# Note: enabling addressindex/txindex on a synced chain requires reindex
sudo systemctl start pivxd
```

If the node is still syncing, just add the flags and it will build indexes during sync. If already synced, will need `-reindex` flag once.

**Step 3: Verify addressindex works after sync**

```bash
# After sync completes, test:
pivx-cli getaddressbalance '{"addresses":["DSomeKnownAddress"]}'
```

**Step 4: Document the required PIVX node flags**

Add a comment to `config/pivx-node.toml`:
```toml
# Requires PIVX node with: addressindex=1, txindex=1
# Add to ~/.pivx/pivx.conf and restart
```

**Step 5: Commit**

```bash
git add config/pivx-node.toml
git commit -m "docs: document required PIVX node flags for addressindex"
```

---

### Task 8: Update web app for multi-chain support

**Files:**
- Modify: `src/webapp/` (API responses should include chain info)

**Step 1: Add chain info to webapp API**

Add an endpoint or include chain metadata (name, ticker, explorer_url) in API responses so the frontend can display the correct ticker. This is likely a small change to pass `ChainConfig` into the webapp module.

**Step 2: Update any hardcoded "DIVI" in static HTML/JS**

Check `static/` directory for hardcoded Divi references and make them dynamic via the API.

**Step 3: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 4: Commit**

```bash
git add src/webapp/ static/
git commit -m "feat: pass chain config to web app for dynamic ticker display"
```

---

### Task 9: Update README and config examples

**Files:**
- Modify: `README.md`
- Modify: `config/services-divi-domains.toml` (add `[chain]` section with Divi defaults)
- Modify: `config/chainz.toml` (add `[chain]` section)
- Modify: `config/custom-node.toml` (add `[chain]` section)

**Step 1: Add `[chain]` section to existing Divi config files**

Add to each existing config:
```toml
[chain]
name = "Divi"
ticker = "DIVI"
address_prefixes = ["D", "y"]
excluded_addresses = ["DPhJsztbZafDc1YeyrRqSjmKjkmLJpQpUn", "DPujt2XAdHyRcZNB5ySZBBVKjzY2uXZGYq"]
has_lottery = true
has_vaults = true
has_masternodes = false
block_time_secs = 60
```

**Step 2: Update README**

Add a "Multi-Chain Support" section explaining that stake_watch supports multiple PoS chains via the `[chain]` config section, with Divi as the default and PIVX as the first additional chain.

**Step 3: Commit**

```bash
git add README.md config/
git commit -m "docs: update README and config files for multi-chain support"
```

---

### Task 10: End-to-end test with PIVX node on dnsdivi

**Step 1: Build and deploy to dnsdivi**

```bash
# Build for linux
cargo build --release --target x86_64-unknown-linux-gnu
# Or use cross-compilation / build on the server

# Copy binary and config
scp target/release/stake_watch ubuntu@dnsdivi:/opt/stake-watch/
scp config/pivx-node.toml ubuntu@dnsdivi:/opt/stake-watch/config/
```

**Step 2: Configure .env on server**

```bash
ssh ubuntu@dnsdivi
cat > /opt/stake-watch/.env << EOF
TELEGRAM_BOT_TOKEN=<create a new bot via @BotFather>
RPC_USERNAME=pivxrpc
RPC_PASSWORD=<from ~/.pivx/pivx.conf>
EOF
```

**Step 3: Start and verify**

```bash
sudo systemctl start stake-watch
sudo journalctl -u stake-watch -f
# Verify: connects to PIVX RPC, processes blocks, no "DIVI" in logs
```

**Step 4: Test Telegram commands**

- `/start` - should show PIVX branding
- `/status` - should show PIVX chain info
- `/watch <pivx-address>` - should accept PIVX addresses
- `/analyze <address>` - should show PIV balances

---

## Backwards Compatibility

All `ChainConfig` fields have defaults matching current Divi behavior. Existing Divi config files without a `[chain]` section will continue to work identically. No migration needed.

## Future Work (Not in Scope)

- Masternode monitoring commands (`/masternode`, `/mnstatus`) — PIVX-specific value-add
- SHIELD transaction alerts — PIVX shielded pool monitoring
- Governance proposal alerts — PIVX budget system notifications
- Auto-detection of chain from RPC `getblockchaininfo` response
