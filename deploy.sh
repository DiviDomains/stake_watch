#!/usr/bin/env bash
# deploy.sh - Deploy stake_watch to the VPS
#
# Usage:
#   ./deploy.sh                         # Build locally, then deploy
#   ./deploy.sh path/to/stake_watch     # Deploy a pre-built binary
#
# After writing, make this executable: chmod +x deploy.sh

set -euo pipefail

VPS_HOST="${DEPLOY_HOST:-ubuntu@dnsdivi}"
VPS_DIR="/opt/stake-watch"
BINARY_NAME="stake_watch"
SERVICE_NAME="stake-watch"

# ---------------------------------------------------------------------------
# Colors
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[INFO]${NC}  $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# ---------------------------------------------------------------------------
# Resolve binary
# ---------------------------------------------------------------------------
BINARY_PATH="${1:-}"

if [ -z "$BINARY_PATH" ]; then
    info "No binary path provided. Building locally..."
    cargo build --release || error "Build failed"
    BINARY_PATH="target/release/${BINARY_NAME}"
fi

[ -f "$BINARY_PATH" ] || error "Binary not found at: $BINARY_PATH"

info "Deploying stake_watch to ${VPS_HOST}:${VPS_DIR}..."

# ---------------------------------------------------------------------------
# Directory structure
# ---------------------------------------------------------------------------
info "Creating remote directory structure..."
ssh "$VPS_HOST" "
    sudo mkdir -p ${VPS_DIR}/{config,data}
    sudo chown -R www-data:www-data ${VPS_DIR}
"

# ---------------------------------------------------------------------------
# Upload binary
# ---------------------------------------------------------------------------
info "Uploading binary..."
scp "$BINARY_PATH" "${VPS_HOST}:/tmp/${BINARY_NAME}"
ssh "$VPS_HOST" "
    sudo mv /tmp/${BINARY_NAME} ${VPS_DIR}/${BINARY_NAME}
    sudo chmod +x ${VPS_DIR}/${BINARY_NAME}
"

# ---------------------------------------------------------------------------
# Upload config files
# ---------------------------------------------------------------------------
if ls config/*.toml &>/dev/null; then
    info "Uploading config files..."
    ssh "$VPS_HOST" "mkdir -p /tmp/stake-watch-config"
    scp config/*.toml "${VPS_HOST}:/tmp/stake-watch-config/"
    ssh "$VPS_HOST" "
        sudo cp /tmp/stake-watch-config/*.toml ${VPS_DIR}/config/
        rm -rf /tmp/stake-watch-config
    "
else
    warn "No .toml config files found in ./config/ — skipping config upload."
fi

# ---------------------------------------------------------------------------
# Bootstrap .env if missing
# ---------------------------------------------------------------------------
ENV_EXISTS=$(ssh "$VPS_HOST" "[ -f ${VPS_DIR}/.env ] && echo yes || echo no")
if [ "$ENV_EXISTS" = "no" ]; then
    if [ -f ".env.example" ]; then
        warn ".env not found on server. Uploading .env.example as .env..."
        scp .env.example "${VPS_HOST}:/tmp/.env.example"
        ssh "$VPS_HOST" "sudo mv /tmp/.env.example ${VPS_DIR}/.env"
        warn "ACTION REQUIRED: Edit ${VPS_DIR}/.env on the server and set TELEGRAM_BOT_TOKEN"
    else
        warn ".env not found on server and no .env.example locally. Create ${VPS_DIR}/.env manually."
    fi
else
    info ".env already exists on server — skipping."
fi

# ---------------------------------------------------------------------------
# Install systemd service
# ---------------------------------------------------------------------------
[ -f "stake-watch.service" ] || error "stake-watch.service not found in current directory."

info "Installing systemd service..."
scp stake-watch.service "${VPS_HOST}:/tmp/stake-watch.service"
ssh "$VPS_HOST" "
    sudo mv /tmp/stake-watch.service /etc/systemd/system/${SERVICE_NAME}.service
    sudo systemctl daemon-reload
    sudo systemctl enable ${SERVICE_NAME}
"

# ---------------------------------------------------------------------------
# Fix ownership and restart
# ---------------------------------------------------------------------------
info "Setting ownership..."
ssh "$VPS_HOST" "sudo chown -R www-data:www-data ${VPS_DIR}"

info "Restarting service..."
ssh "$VPS_HOST" "sudo systemctl restart ${SERVICE_NAME}"

sleep 2

if ssh "$VPS_HOST" "sudo systemctl is-active --quiet ${SERVICE_NAME}"; then
    ssh "$VPS_HOST" "sudo systemctl status ${SERVICE_NAME} --no-pager"
    info "Deployment successful!"
else
    ssh "$VPS_HOST" "sudo systemctl status ${SERVICE_NAME} --no-pager" || true
    warn "Service did not start cleanly. Check logs:"
    warn "  ssh ${VPS_HOST} 'sudo journalctl -u ${SERVICE_NAME} -n 50'"
    exit 1
fi

info "Tail logs with: ssh ${VPS_HOST} 'sudo journalctl -u ${SERVICE_NAME} -f'"
