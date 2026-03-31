# Stake Watch - Claude Instructions

## Project Overview

Rust Telegram bot + web app for monitoring Divi blockchain staking rewards, lottery wins, fork detection, and blockchain anomalies. Public repo at `DiviDomains/stake_watch`.

## Architecture

- **Language**: Rust (2021 edition), vanilla HTML/CSS/JS for webapp
- **Async Runtime**: Tokio
- **Telegram Bot**: teloxide 0.13 (polling mode)
- **Web App**: axum HTTP server (port 18095) serving static files + REST API
- **Database**: SQLite via rusqlite (bundled), WAL mode
- **Config**: TOML files for settings, `.env` for secrets only
- **Multi-backend**: Socket.IO (services.divi.domains) or polling (chainz, custom node)
- **Telegram Mini App**: accessible via bot Menu button at https://stakewatch.divi.cx/

## Deployment

- **Server**: `ubuntu@dnsdivi` (same as `ubuntu@vps1.divi.domains`, IP: 15.204.117.157)
- **Bot binary**: `/opt/stake-watch/stake_watch`
- **Static files**: `/opt/stake-watch/static/`
- **Database**: `/opt/stake-watch/data/stake_watch.db` — **NEVER DELETE THIS**
- **Config**: `/opt/stake-watch/config/config.toml`
- **Secrets**: `/opt/stake-watch/.env`
- **Service**: `systemd stake-watch.service`
- **HTTPS**: nginx reverse proxy on `stakewatch.divi.cx` → localhost:18095
- **TLS cert**: Let's Encrypt at `/etc/letsencrypt/live/stakewatch.divi.cx/`
- **DNS**: BIND zone at `/etc/bind/zones/divi.cx.zone` (stakewatch A record)

### Deploy commands (DO NOT wipe the database)
```bash
rsync -az --exclude='target/' --exclude='.git/' --exclude='data/' --exclude='.env' --exclude='.omc/' . ubuntu@dnsdivi:/tmp/stake-watch-build/
ssh ubuntu@dnsdivi "source ~/.cargo/env && cd /tmp/stake-watch-build && cargo build --release --bin stake_watch"
ssh ubuntu@dnsdivi "sudo systemctl stop stake-watch && sudo cp /tmp/stake-watch-build/target/release/stake_watch /opt/stake-watch/stake_watch && sudo cp -r /tmp/stake-watch-build/static /opt/stake-watch/ && sudo chown -R www-data:www-data /opt/stake-watch && sudo systemctl start stake-watch && rm -rf /tmp/stake-watch-build"
```

For JS-only changes (no Rust rebuild needed):
```bash
scp static/js/file.js ubuntu@dnsdivi:/tmp/file.js && ssh ubuntu@dnsdivi "sudo cp /tmp/file.js /opt/stake-watch/static/js/file.js && sudo chown www-data:www-data /opt/stake-watch/static/js/file.js && rm /tmp/file.js"
```

## Key Source Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point, wires all subsystems (bot, monitor, processor, alerts, webapp) |
| `src/lib.rs` | Module re-exports for multi-binary support |
| `src/config.rs` | TOML + .env config loading, `AppConfig`, `Secrets` |
| `src/rpc.rs` | `RpcClient` trait with `JsonRpcClient` and `ChainzClient` implementations |
| `src/db.rs` | SQLite schema (8 tables), 30+ query functions, migrations |
| `src/monitor/` | `BlockMonitor` trait, `SocketIoMonitor`, `PollingMonitor` |
| `src/block_processor.rs` | Block analysis: stake/lottery/anomaly detection, notifications |
| `src/bot.rs` | Telegram command handlers (17 commands including admin) |
| `src/notifier.rs` | Notification formatting + delivery (HTML, rate-limited) |
| `src/stake_analyzer.rs` | Staking frequency estimation, missed stake alerts, backfill |
| `src/fork_detector.rs` | Multi-endpoint fork detection |
| `src/alert_analyzer.rs` | Blockchain anomaly detection (7 alert types) |
| `src/utils.rs` | Formatting helpers (satoshi_to_divi, time_ago, etc.) |
| `src/webapp/mod.rs` | axum router, static file serving, `WebAppState` |
| `src/webapp/api.rs` | REST API (16+ endpoints: watches, alerts, explorer, SSE, price) |
| `src/webapp/auth.rs` | Telegram initData HMAC-SHA256 validation |

### Frontend (static/)
| File | Purpose |
|------|---------|
| `static/index.html` | SPA shell with Telegram WebApp SDK |
| `static/css/style.css` | Telegram theme integration, mobile-first |
| `static/js/app.js` | SPA router, navigation, Telegram init |
| `static/js/api.js` | API client with initData auth |
| `static/js/dashboard.js` | Portfolio overview, address cards |
| `static/js/address.js` | Address detail, staking calculator |
| `static/js/explorer.js` | Block explorer (blocks, txs, addresses) |
| `static/js/watches.js` | Watch management (add/remove/reorder) |
| `static/js/alerts.js` | Alert subscription management |
| `static/js/users.js` | Admin users panel |
| `static/js/helpers.js` | Shared formatters, download helpers |
| `static/js/blockfeed.js` | SSE live block feed |

## Bot Commands

| Command | Access | Description |
|---------|--------|-------------|
| `/start` | All | Welcome + auto-register with defaults |
| `/help` | All | Command list (admin sees extra commands) |
| `/watch` | All | Watch an address |
| `/unwatch` | All | Stop watching |
| `/list` | All | List watched addresses |
| `/analyze` | All | Staking performance analysis |
| `/status` | All | Bot health & stats |
| `/alerts` | All | View alert subscriptions |
| `/alert` | All | Subscribe to blockchain alert |
| `/unalert` | All | Unsubscribe |
| `/forkwatch` | All | Subscribe to fork alerts |
| `/forkunwatch` | All | Unsubscribe fork alerts |
| `/forkstatus` | All | Fork monitoring status |
| `/addfork` | Admin | Add fork monitoring endpoint |
| `/removefork` | Admin | Remove fork endpoint |
| `/users` | Admin | List all registered users |
| `/broadcast` | Admin | Send message to all users |

## Critical Technical Details

### Vault Addresses
- Divi vault UTXOs are NOT visible in the regular address index
- Use `getaddressbalance` with `only_vaults=true` (second param) to get vault balances
- Use `getaddressdeltas` with `only_vaults=true` for vault transaction history
- Vault script type is `"vault"` with `addresses: [owner, manager]`
- Owner = addresses[0] (holds balance), Manager = addresses[1] (staking operator)
- The address index indexes vaults under the OWNER's pubkey hash (type=3)

### Socket.IO Block Hashes
- services.divi.domains zmq-viewer ALREADY reverses hashes to RPC format
- Do NOT reverse the hash — use `event.data` as-is
- Block processor has retry logic (block may not be indexed when ZMQ fires)

### RPC Parameter Formats
- Address methods: `[{"addresses": ["addr"]}, only_vaults_bool]`
- `getrawtransaction`: `[txid, 1]` for verbose/decoded output
- `vin.value` may be `None` — fetch previous tx if needed for reward calculation

### Reward Calculation
- Vault stake reward = total_outputs - total_inputs of the coinstake tx
- If vin.value is not populated, fetch the previous tx to get input value
- Sanity check: reward must be < 10,000 DIVI (flag if not)
- Treasury/Charity get "payment" events, regular addresses get "stake" events

### Default Watches
- Treasury: `DPhJsztbZafDc1YeyrRqSjmKjkmLJpQpUn`
- Charity: `DPujt2XAdHyRcZNB5ySZBBVKjzY2uXZGYq`
- Added on first interaction IF user has 0 watches (don't re-add if removed)
- Treasury/Charity have `include_in_portfolio = false`, `sort_order = 1000`

### Database Tables
users, watched_addresses (with sort_order, include_in_portfolio), stake_events (type: stake/lottery/payment), alert_log, alert_subscriptions, fork_watchers, fork_endpoints, fork_events, scan_state

### Staking Frequency Formula
```
probability_per_block = balance / network_supply (default 4B)
expected_seconds = (1 / probability) * 60
clamped to [1 hour, 1 year]
```

## Build & Test

```bash
cargo check                    # Type check
cargo test                     # Run tests (26 tests)
cargo build --release          # Release binary
cargo clippy -- -D warnings    # Lint (CI enforces this)
cargo fmt                      # Format (CI enforces this)
```

## CI/CD

- `ci.yml`: Runs on push to main — fmt, clippy -D warnings, test, build
- `release.yml`: Runs on `v*` tags — 5-platform binary matrix release
- **clippy must pass with -D warnings** or CI fails

## Git Identity
```
user.name = Divi Developer
user.email = divi@cri.xyz
```

## Admin
- Admin Telegram ID: 887521560 (set in ADMIN_TELEGRAM_IDS in .env)
- Bot token: in `.env` on server (TELEGRAM_BOT_TOKEN)

## Important Rules
1. **NEVER delete the database** when deploying updates
2. **ALWAYS test changes before deploying** — curl API endpoints, check JS with `node -c`
3. **Run `cargo fmt` and `cargo clippy -- -D warnings`** before pushing (CI enforces)
4. Treasury/Charity are NOT staking addresses — they receive block reward payments
5. Vault balances need `only_vaults=true` RPC parameter
6. Socket.IO hashes from services.divi.domains are already in correct format (don't reverse)
