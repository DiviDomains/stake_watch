// ============================================================
// Stake Watch -- Block Explorer Views
// ============================================================
// Search bar, recent blocks, block detail, transaction detail,
// and address page with balance and transaction history.
// ============================================================

import { api } from './api.js';
import { startBlockFeed } from './blockfeed.js';
import {
    formatDivi,
    formatDiviFloat,
    formatDiviShort,
    formatTimestamp,
    timeAgo,
    escapeHtml,
    addressLink,
    txLink,
    blockLink,
    downloadJson,
    downloadCsv,
} from './helpers.js';

// Expose download helpers globally so inline onclick handlers can call them
window.downloadJson = downloadJson;
window.downloadCsv = downloadCsv;

// Track active SSE connection so we can close it on navigation
let activeBlockFeed = null;

// ----- Explorer Main View -----

export async function renderExplorer(container) {
    // Close any active block feed
    if (activeBlockFeed) {
        activeBlockFeed.close();
        activeBlockFeed = null;
    }

    let html = `<div class="view-enter">`;

    // Search bar
    html += `
        <div class="search-bar">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="11" cy="11" r="8"/>
                <line x1="21" y1="21" x2="16.65" y2="16.65"/>
            </svg>
            <input type="text"
                   id="explorer-search"
                   placeholder="Block height, hash, txid, or address..."
                   autocomplete="off"
                   autocorrect="off"
                   autocapitalize="off"
                   spellcheck="false"
                   enterkeyhint="search" />
        </div>`;

    // Live feed indicator + blocks container
    html += `
        <div class="flex-between mb-md">
            <div class="section-title" style="margin: 0;">Recent Blocks</div>
            <div class="text-xs text-hint"><span class="live-dot"></span>Live</div>
        </div>
        <div id="block-list">
            <div class="loading">Loading blocks...</div>
        </div>
    </div>`;

    container.innerHTML = html;

    // Bind search handler
    const searchInput = document.getElementById('explorer-search');
    searchInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
            handleSearch(searchInput.value.trim());
        }
    });

    // Load initial blocks
    try {
        const blocks = await api.getBlocks(20);
        renderBlockList(document.getElementById('block-list'), blocks);
    } catch (e) {
        document.getElementById('block-list').innerHTML = `
            <div class="error-state">Could not load blocks: ${escapeHtml(e.message)}</div>`;
    }

    // Start live block feed
    activeBlockFeed = startBlockFeed(document.getElementById('block-list'));
}

function renderBlockList(container, blocks) {
    if (!blocks || blocks.length === 0) {
        container.innerHTML = `<div class="empty-state"><p>No blocks found.</p></div>`;
        return;
    }

    let html = '';
    for (const block of blocks) {
        const height = block.height;
        const hash = block.hash;
        const txCount = block.tx_count || (block.tx ? block.tx.length : 0);
        const time = block.time ? timeAgo(block.time) : '';

        html += `
            <div class="block-card card-stagger" onclick="navigate('block', { hash: '${escapeHtml(hash)}' })">
                <div class="block-height">#${Number(height).toLocaleString()}</div>
                <div class="block-info">
                    <div class="block-hash-preview">${escapeHtml(hash)}</div>
                    <div class="block-meta">
                        <span class="block-meta-item">${time}</span>
                    </div>
                </div>
                <div class="block-tx-count">${txCount} tx</div>
            </div>`;
    }

    container.innerHTML = html;
}

function handleSearch(query) {
    if (!query) return;
    window.haptic('light');

    // Detect query type
    if (/^\d+$/.test(query)) {
        // Numeric -> block height
        navigate('block', { hash: query });
    } else if (/^[0-9a-fA-F]{64}$/.test(query)) {
        // 64 hex chars -> could be block hash or txid, try search endpoint
        performSearch(query);
    } else if (/^[DdYy]/.test(query) && query.length >= 25 && query.length <= 45) {
        // Starts with D/d/Y/y and is address-length -> address
        navigate('address', { address: query });
    } else {
        // Fallback: use search endpoint
        performSearch(query);
    }
}

async function performSearch(query) {
    try {
        const result = await api.search(query);
        if (result.type === 'block') {
            navigate('block', { hash: result.hash || query });
        } else if (result.type === 'tx') {
            navigate('tx', { txid: result.txid || query });
        } else if (result.type === 'address') {
            navigate('address', { address: result.address || query });
        } else {
            window.showToast('No results found', 'error');
        }
    } catch (e) {
        // If search endpoint fails, try as block hash, then txid
        try {
            await api.getBlock(query);
            navigate('block', { hash: query });
        } catch {
            try {
                await api.getTx(query);
                navigate('tx', { txid: query });
            } catch {
                window.showToast('Not found: ' + query.slice(0, 20) + '...', 'error');
            }
        }
    }
}

// ----- Block Detail View -----

export async function renderBlockDetail(container, hashOrHeight) {
    try {
        const block = await api.getBlock(hashOrHeight);

        let html = `<div class="view-enter">`;

        // Page header
        html += `
            <div class="page-header">
                <button class="back-btn" onclick="goBack()">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="15 18 9 12 15 6"/>
                    </svg>
                </button>
                <div class="page-title">Block #${Number(block.height).toLocaleString()}</div>
            </div>`;

        // Block info
        html += `
            <div class="card card-stagger">
                <div class="info-grid">
                    <div class="info-row">
                        <div class="info-label">Hash</div>
                        <div class="info-value info-value-mono">${escapeHtml(block.hash)}</div>
                    </div>
                    <div class="info-row">
                        <div class="info-label">Height</div>
                        <div class="info-value text-mono">${Number(block.height).toLocaleString()}</div>
                    </div>
                    <div class="info-row">
                        <div class="info-label">Time</div>
                        <div class="info-value">${formatTimestamp(block.time)}${block.time ? ' (' + timeAgo(block.time) + ')' : ''}</div>
                    </div>
                    ${block.size ? `
                    <div class="info-row">
                        <div class="info-label">Size</div>
                        <div class="info-value text-mono">${Number(block.size).toLocaleString()} bytes</div>
                    </div>` : ''}
                    <div class="info-row">
                        <div class="info-label">Transactions</div>
                        <div class="info-value text-mono">${block.tx_count || (block.transactions ? block.transactions.length : 0)}</div>
                    </div>
                </div>
            </div>`;

        // Transaction list with full inline detail
        const txList = block.transactions || [];
        if (txList.length > 0) {
            html += `<div class="section-title card-stagger">Transactions</div>`;

            for (let i = 0; i < txList.length; i++) {
                const tx = txList[i];
                const txid = tx.txid || '';
                const label = tx.label || (i === 0 ? 'Coinbase' : i === 1 ? 'Coinstake' : `Tx ${i}`);
                const hasFullData = tx.vin && tx.vout;

                html += `<div class="card card-stagger" style="padding: var(--space-md) var(--space-lg);">`;
                const rewardStr = tx.reward_divi ? `<span class="text-success text-xs" style="margin-left: 8px;">+${formatDiviFloat(parseFloat(tx.reward_divi))} reward</span>` : '';
                html += `<div class="flex-between mb-sm">
                    <span class="badge ${i <= 1 ? 'badge-neutral' : ''}">${label}${rewardStr}</span>
                    <span class="text-hint text-xs">${formatDiviFloat(parseFloat(tx.total_output_divi || '0'))} DIVI</span>
                </div>`;
                html += `<div class="tx-hash" style="cursor:pointer" onclick="navigate('tx', { txid: '${escapeHtml(txid)}' })">${escapeHtml(txid)}</div>`;

                if (hasFullData) {
                    html += `<div style="margin-top: var(--space-sm); font-size: 0.8rem;">`;

                    // Inputs with resolved addresses
                    for (const vin of tx.vin) {
                        if (vin.coinbase) {
                            html += `<div class="tx-io-row" style="padding: 3px 0;"><span class="text-hint text-xs">Coinbase (new coins)</span></div>`;
                        } else {
                            const vinAddrs = vin.addresses || [];
                            const vinVal = vin.value != null ? formatDiviFloat(vin.value) : '';
                            const isVinVault = vin.script_type === 'vault';
                            const vinAddrDisplay = vinAddrs.length > 0
                                ? vinAddrs.map(a => addressLink(a, 'text-mono text-xs text-accent')).join(', ')
                                : '<span class="text-hint">Unknown</span>';
                            if (isVinVault && vinAddrs.length >= 2) {
                                html += `<div class="tx-io-row" style="padding: 3px 0;">
                                    <span class="text-xs">From: ${addressLink(vinAddrs[0], 'text-mono text-xs text-accent')} <span class="vault-badge" style="font-size:0.6rem">Owner</span></span>
                                    ${vinVal ? `<span class="text-xs">${vinVal}</span>` : ''}
                                </div>
                                <div class="tx-io-row" style="padding: 1px 0;">
                                    <span class="text-xs text-hint">Mgr: ${addressLink(vinAddrs[1], 'text-mono text-xs text-hint')}</span>
                                </div>`;
                            } else {
                                html += `<div class="tx-io-row" style="padding: 3px 0;">
                                    <span class="text-xs">From: ${vinAddrDisplay}</span>
                                    ${vinVal ? `<span class="text-xs">${vinVal}</span>` : ''}
                                </div>`;
                            }
                        }
                    }

                    html += `<div style="padding: 4px 0; text-align: center; color: var(--hint); font-size: 0.75rem;">↓</div>`;

                    // Outputs
                    for (const vout of tx.vout) {
                        const spk = vout.scriptPubKey || {};
                        const addrs = spk.addresses || [];
                        const val = vout.value != null ? formatDiviFloat(vout.value) : '0';
                        const isVault = spk.type === 'vault';
                        const isEmpty = vout.value === 0 && addrs.length === 0;

                        if (isEmpty) {
                            html += `<div class="tx-io-row" style="padding: 3px 0;"><span class="text-hint text-xs">PoS marker</span></div>`;
                        } else if (isVault && addrs.length >= 2) {
                            // Vault output: addresses[0] = owner, addresses[1] = manager
                            html += `<div class="tx-io-row" style="padding: 3px 0;">
                                <span class="text-xs">To: ${addressLink(addrs[0], 'text-mono text-xs text-accent')} <span class="vault-badge" style="font-size:0.6rem">Owner</span></span>
                                <span class="text-xs text-success">${val}</span>
                            </div>
                            <div class="tx-io-row" style="padding: 1px 0;">
                                <span class="text-xs text-hint">Mgr: ${addressLink(addrs[1], 'text-mono text-xs text-hint')}</span>
                            </div>`;
                        } else {
                            const addrDisplay = addrs.length > 0
                                ? addrs.map(a => addressLink(a, 'text-mono text-xs text-accent')).join(', ')
                                : `<span class="text-hint text-xs">${spk.type || 'script'}</span>`;
                            html += `<div class="tx-io-row" style="padding: 3px 0;">
                                <span class="text-xs">To: ${addrDisplay}</span>
                                <span class="text-xs text-success">${val}</span>
                            </div>`;
                        }
                    }

                    html += `</div>`;
                }

                html += `</div>`;
            }
        }

        // Download button
        window.__blockDownloadData = block;
        html += `
            <div style="display:flex; gap:8px; margin-top:var(--space-lg);">
                <button class="btn btn-ghost btn-sm" onclick="downloadJson(window.__blockDownloadData, 'block-${block.height}.json')">Download JSON</button>
            </div>`;

        html += `</div>`;
        container.innerHTML = html;

    } catch (e) {
        container.innerHTML = `
            <div class="view-enter">
                <div class="error-state">
                    Could not load block: ${escapeHtml(e.message)}
                </div>
                <button class="btn btn-ghost btn-full mt-lg" onclick="goBack()">
                    Go Back
                </button>
            </div>`;
    }
}

// ----- Transaction Detail View -----

export async function renderTxDetail(container, txid) {
    try {
        const tx = await api.getTx(txid);

        let html = `<div class="view-enter">`;

        // Page header
        html += `
            <div class="page-header">
                <button class="back-btn" onclick="goBack()">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="15 18 9 12 15 6"/>
                    </svg>
                </button>
                <div class="page-title">Transaction</div>
            </div>`;

        // Txid
        html += `
            <div class="card card-stagger">
                <div class="info-row" style="border-bottom: none;">
                    <div class="info-label">Transaction ID</div>
                    <div class="info-value info-value-mono">${escapeHtml(tx.txid)}</div>
                </div>
                ${tx.blockhash ? `
                <div class="info-row" style="border-bottom: none; margin-top: var(--space-sm);">
                    <div class="info-label">Block</div>
                    <div class="info-value info-value-mono">
                        <span class="text-accent" style="cursor:pointer"
                              onclick="navigate('block', { hash: '${escapeHtml(tx.blockhash)}' })">
                            ${escapeHtml(tx.blockhash)}
                        </span>
                    </div>
                </div>` : ''}
            </div>`;

        // Inputs
        html += `
            <div class="tx-detail card-stagger">
                <div class="tx-section-title">Inputs (${tx.vin ? tx.vin.length : 0})</div>`;

        if (tx.vin && tx.vin.length > 0) {
            for (const vin of tx.vin) {
                if (vin.coinbase) {
                    html += `
                        <div class="tx-io-row">
                            <div class="tx-io-address">
                                <span class="badge badge-neutral">Coinbase</span>
                            </div>
                            <div class="tx-io-value">New coins</div>
                        </div>`;
                } else {
                    const addresses = vin.addresses || (vin.address ? [vin.address] : []);
                    const value = vin.value != null ? formatDiviFloat(vin.value) : '';

                    html += `
                        <div class="tx-io-row">
                            <div class="tx-io-address">
                                ${addresses.length > 0
                                    ? addresses.map(a => addressLink(a)).join('<br>')
                                    : '<span class="text-hint text-sm">Unknown</span>'}
                                ${vin.txid ? `<div class="text-xs text-hint mt-sm">from ${txLink(vin.txid)}:${vin.vout ?? ''}</div>` : ''}
                            </div>
                            ${value ? `<div class="tx-io-value">${value}</div>` : ''}
                        </div>`;
                }
            }
        } else {
            html += `<div class="text-sm text-hint" style="padding: var(--space-sm) 0;">No inputs</div>`;
        }

        html += `</div>`;

        // Arrow separator
        html += `
            <div class="tx-arrow">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="12" y1="5" x2="12" y2="19"/>
                    <polyline points="19 12 12 19 5 12"/>
                </svg>
            </div>`;

        // Outputs
        html += `
            <div class="tx-detail card-stagger">
                <div class="tx-section-title">Outputs (${tx.vout ? tx.vout.length : 0})</div>`;

        if (tx.vout && tx.vout.length > 0) {
            for (const vout of tx.vout) {
                const scriptType = vout.scriptPubKey?.type || vout.script_pub_key?.script_type || '';
                const isVault = scriptType === 'vault';
                const addresses = vout.scriptPubKey?.addresses
                    || vout.script_pub_key?.addresses
                    || [];
                const value = vout.value != null ? formatDiviFloat(vout.value) : '0.00000000';

                // For PoS coinbase empty outputs (value=0, nonstandard), show a cleaner label
                const isEmptyOutput = (vout.value === 0 || vout.value === 0.0) && addresses.length === 0;
                const outputLabel = addresses.length > 0
                    ? addresses.map(a => addressLink(a)).join('<br>')
                    : isEmptyOutput
                        ? '<span class="text-hint text-sm">PoS Coinbase Marker (empty)</span>'
                        : `<span class="text-hint text-sm">${scriptType || 'No address'}</span>`;

                html += `
                    <div class="tx-io-row">
                        <div class="tx-io-address">
                            ${outputLabel}
                            ${isVault ? '<span class="vault-badge" style="margin-left: 4px;">Vault</span>' : ''}
                        </div>
                        <div class="tx-io-value positive">${isEmptyOutput ? '' : value}</div>
                    </div>`;
            }
        } else {
            html += `<div class="text-sm text-hint" style="padding: var(--space-sm) 0;">No outputs</div>`;
        }

        html += `</div>`;

        // Download button
        window.__txDownloadData = tx;
        html += `
            <div style="display:flex; gap:8px; margin-top:var(--space-lg);">
                <button class="btn btn-ghost btn-sm" onclick="downloadJson(window.__txDownloadData, 'tx-${escapeHtml(txid.slice(0, 16))}.json')">Download JSON</button>
            </div>`;

        html += `</div>`;
        container.innerHTML = html;

    } catch (e) {
        container.innerHTML = `
            <div class="view-enter">
                <div class="error-state">
                    Could not load transaction: ${escapeHtml(e.message)}
                </div>
                <button class="btn btn-ghost btn-full mt-lg" onclick="goBack()">
                    Go Back
                </button>
            </div>`;
    }
}

// ----- Address Page (Explorer View) -----

export async function renderAddressPage(container, address) {
    try {
        // Fetch regular and vault balances in parallel
        const [addrResult, vaultResult] = await Promise.allSettled([
            api.getAddress(address),
            api.getVaultBalance(address),
        ]);

        const addrData = addrResult.status === 'fulfilled' ? addrResult.value : null;
        const vaultData = vaultResult.status === 'fulfilled' ? vaultResult.value : null;

        let html = `<div class="view-enter">`;

        // Page header
        html += `
            <div class="page-header">
                <button class="back-btn" onclick="goBack()">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="15 18 9 12 15 6"/>
                    </svg>
                </button>
                <div class="page-title">Address</div>
            </div>`;

        // Address
        html += `
            <div class="card card-stagger">
                <div class="info-row" style="border-bottom: none;">
                    <div class="info-label">Address</div>
                    <div class="address address-static">${escapeHtml(address)}</div>
                </div>
            </div>`;

        // Balances — prefer vault_balance_divi from addrData if vaultData unavailable
        const regularBalance = addrData?.balance ?? 0;
        const vaultBalanceSats = vaultData?.balance ?? (
            addrData?.vault_balance_divi != null
                ? Math.round(parseFloat(addrData.vault_balance_divi) * 1e8)
                : 0
        );
        const totalReceived = addrData?.received ?? 0;

        html += `
            <div class="card card-stagger">
                <div class="card-stats">
                    <div>
                        <div class="stat-value stat-value-sm">${formatDiviShort(regularBalance)}</div>
                        <div class="stat-label">Regular Balance</div>
                    </div>
                    <div>
                        <div class="stat-value stat-value-sm">
                            ${formatDiviShort(vaultBalanceSats)}
                            ${vaultBalanceSats > 0 ? '<span class="vault-badge" style="margin-left:4px;">Vault</span>' : ''}
                        </div>
                        <div class="stat-label">Vault Balance</div>
                    </div>
                </div>
                ${totalReceived > 0 ? `
                <div class="divider"></div>
                <div class="flex-between">
                    <span class="stat-label">Total Received</span>
                    <span class="text-sm text-mono text-hint">${formatDivi(totalReceived)}</span>
                </div>
                ` : ''}
                ${regularBalance === 0 && vaultBalanceSats === 0 && totalReceived === 0 ? `
                <div class="divider"></div>
                <div class="text-xs text-hint" style="padding: var(--space-sm) 0;">
                    This address has no indexed balance. It may be a <b>vault manager</b> address
                    (vault balances are indexed under the owner address, not the manager).
                </div>
                ` : ''}
            </div>`;

        // Watch this address button
        html += `
            <button class="btn btn-primary btn-full mt-lg card-stagger"
                    onclick="quickWatch('${escapeHtml(address)}')">
                Watch This Address
            </button>`;

        // Recent transactions from recent_deltas
        // Deduplicate by txid (vault deltas have both +/- entries per tx)
        const rawDeltas = addrData?.recent_deltas || [];
        const seenTxids = new Set();
        const deltas = [];
        for (const d of rawDeltas) {
            if (!seenTxids.has(d.txid)) {
                seenTxids.add(d.txid);
                deltas.push(d);
            }
        }

        html += `<div class="section-title card-stagger">Recent Transactions (${deltas.length})</div>`;

        if (deltas.length === 0) {
            html += `<div class="card card-stagger"><div class="text-sm text-hint" style="padding: var(--space-sm) 0;">No transactions found</div></div>`;
        } else {
            for (const delta of deltas) {
                // amount_divi is a string like "12946.50000000" or "-11286.50000000"
                const amountFloat = parseFloat(delta.amount_divi || '0');
                const isPositive = amountFloat >= 0;
                const amountDisplay = (isPositive ? '+' : '') + formatDiviFloat(amountFloat);
                const amountClass = isPositive ? 'text-success' : 'text-danger';
                const txid = delta.txid || '';
                const height = delta.height != null ? delta.height : null;

                html += `
                    <div class="card card-stagger" style="padding: var(--space-sm) var(--space-lg);">
                        <div class="flex-between" style="gap: var(--space-sm); align-items: center;">
                            <div style="min-width:0; flex:1;">
                                ${height != null ? `<div class="text-xs text-hint mb-sm">${blockLink(height, '#' + Number(height).toLocaleString())}</div>` : ''}
                                <div class="tx-hash" style="cursor:pointer; font-size:0.75rem;"
                                     onclick="navigate('tx', { txid: '${escapeHtml(txid)}' })">${escapeHtml(txid)}</div>
                            </div>
                            <div class="${amountClass} text-mono text-sm" style="white-space:nowrap; flex-shrink:0;">${amountDisplay}</div>
                        </div>
                    </div>`;
            }
        }

        // Download buttons
        window.__addrDownloadData = {
            address,
            balance_divi: addrData?.balance_divi,
            vault_balance_divi: addrData?.vault_balance_divi,
            received_divi: addrData?.received_divi,
            transactions: deltas,
        };

        window.__addrCsvData = deltas.map(d => [
            d.height ?? '',
            d.txid ?? '',
            d.amount_divi ?? '',
        ]);

        html += `
            <div style="display:flex; gap:8px; margin-top:var(--space-lg);">
                <button class="btn btn-ghost btn-sm" onclick="downloadJson(window.__addrDownloadData, 'address-${escapeHtml(address)}.json')">Download JSON</button>
                <button class="btn btn-ghost btn-sm" onclick="downloadCsv(window.__addrCsvData, ['height','txid','amount_divi'], 'address-${escapeHtml(address)}.csv')">Download CSV</button>
            </div>`;

        html += `</div>`;
        container.innerHTML = html;

    } catch (e) {
        container.innerHTML = `
            <div class="view-enter">
                <div class="error-state">
                    Could not load address: ${escapeHtml(e.message)}
                </div>
                <button class="btn btn-ghost btn-full mt-lg" onclick="goBack()">
                    Go Back
                </button>
            </div>`;
    }
}

// Quick-watch handler from explorer address page
window.quickWatch = async function(address) {
    try {
        await api.addWatch(address, null);
        window.haptic('success');
        window.showToast('Address added to watch list');
        navigate('address-detail', { address });
    } catch (e) {
        if (e.message.includes('409') || e.message.includes('already')) {
            window.showToast('Already watching this address', 'error');
            navigate('address-detail', { address });
        } else {
            window.haptic('error');
            window.showToast('Failed: ' + e.message, 'error');
        }
    }
};
