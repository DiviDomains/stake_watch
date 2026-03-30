# Stake Watch - Claude Instructions

## Project Overview

Rust Telegram bot for monitoring Divi blockchain staking rewards, lottery wins, fork detection, and blockchain anomalies.

## Architecture

- **Language**: Rust (2021 edition)
- **Async Runtime**: Tokio
- **Telegram**: teloxide 0.13
- **Database**: SQLite via rusqlite (bundled)
- **Config**: TOML files for settings, `.env` for secrets only
- **Multi-backend**: Socket.IO (services.divi.domains) or polling (chainz, custom node)

## Key Source Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point, wires all subsystems |
| `src/config.rs` | TOML + .env config loading |
| `src/rpc.rs` | RPC client trait with JsonRpc and Chainz implementations |
| `src/db.rs` | SQLite schema + 25+ query functions |
| `src/monitor/` | BlockMonitor trait, Socket.IO and polling backends |
| `src/block_processor.rs` | Block analysis: stake/lottery/anomaly detection |
| `src/bot.rs` | Telegram command handlers (15 commands) |
| `src/notifier.rs` | Notification formatting + delivery |
| `src/stake_analyzer.rs` | Staking frequency estimation + missed stake alerts |
| `src/fork_detector.rs` | Multi-endpoint fork detection |
| `src/alert_analyzer.rs` | Blockchain anomaly detection (7 alert types) |
| `src/utils.rs` | Hex reversal, formatting helpers |

## Build & Test

```bash
cargo check          # Type check
cargo test           # Run tests (23 tests)
cargo build --release # Release binary
cargo clippy -- -D warnings # Lint
```

## Deployment

```bash
./deploy.sh                    # Build + deploy to VPS1
./deploy.sh /path/to/binary    # Deploy pre-built binary
```

Target: `ubuntu@vps1.divi.domains:/opt/stake-watch/`

## Configuration

- `config/config.toml` - Main config (backend, thresholds, limits)
- `config/chainz.toml` - chainz polling example
- `config/custom-node.toml` - Self-hosted node example
- `.env` - Secrets (TELEGRAM_BOT_TOKEN, ADMIN_TELEGRAM_IDS, RPC creds)

## RPC Notes

- Address methods require `[{"addresses": [addr]}]` parameter format
- `getrawtransaction` needs verbose=1 (`[txid, 1]`)
- Socket.IO block hashes need byte reversal (ZMQ little-endian)
- chainz uses REST API, not JSON-RPC; some features unavailable

## Git Identity

```
user.name = Divi Developer
user.email = divi@cri.xyz
```

## Repo

`DiviDomains/stake_watch` (public)
