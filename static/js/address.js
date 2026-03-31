// ============================================================
// Stake Watch -- Address Detail View
// ============================================================
// Full analysis of a watched address: balance, vault status,
// health, expected frequency, Chart.js reward history, and
// recent stakes table.
// ============================================================

import { api } from './api.js';
import { createStakeChart } from './chart-config.js';
import {
    formatDivi,
    formatDiviShort,
    formatDiviFloat,
    formatDuration,
    timeAgo,
    escapeHtml,
    blockLink,
    txLink,
} from './helpers.js';

export async function renderAddressDetail(container, address) {
    try {
        const [analysis, stakes] = await Promise.allSettled([
            api.getAnalysis(address),
            api.getStakes(address, 100),
        ]);

        const a = analysis.status === 'fulfilled' ? analysis.value : null;
        const stakeList = stakes.status === 'fulfilled' ? stakes.value : [];

        let html = `<div class="view-enter">`;

        // Page header
        html += `
            <div class="page-header">
                <button class="back-btn" onclick="goBack()">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="15 18 9 12 15 6"/>
                    </svg>
                </button>
                <div class="page-title">Address Detail</div>
            </div>`;

        // Address display
        html += `
            <div class="card card-stagger">
                <div class="info-row" style="border-bottom: none;">
                    <div class="info-label">Address</div>
                    <div class="address address-static">${escapeHtml(address)}</div>
                </div>
            </div>`;

        // Analysis data
        if (a) {
            const healthClass = `health-${a.health}`;
            const healthText = a.health === 'healthy' ? 'Healthy'
                             : a.health === 'overdue' ? 'Overdue'
                             : 'No data';

            html += `
                <div class="card card-stagger">
                    <div class="card-header">
                        <div class="card-label">
                            <span class="health-dot ${a.health}"></span>
                            Staking Status
                            ${a.is_vault ? '<span class="vault-badge">Vault</span>' : ''}
                        </div>
                        <span class="health-label ${healthClass}">${healthText}</span>
                    </div>
                    <div class="card-stats card-stats-3">
                        <div>
                            <div class="stat-value stat-value-sm">${formatDiviShort(a.balance_satoshis || 0)}</div>
                            <div class="stat-label">Balance</div>
                        </div>
                        <div>
                            <div class="stat-value stat-value-sm">${a.stakes_24h ?? '--'}</div>
                            <div class="stat-label">Stakes / 24h</div>
                        </div>
                        <div>
                            <div class="stat-value stat-value-sm">
                                ${a.expected_interval_secs ? formatDuration(a.expected_interval_secs) : '--'}
                            </div>
                            <div class="stat-label">Expected Freq</div>
                        </div>
                    </div>
                    <div class="divider"></div>
                    <div class="card-stats">
                        <div>
                            <div class="stat-value stat-value-sm text-success">
                                +${formatDiviShort(a.rewards_24h_satoshis || 0)}
                            </div>
                            <div class="stat-label">24h Rewards</div>
                        </div>
                        <div>
                            <div class="stat-value stat-value-sm text-success">
                                +${formatDiviShort(a.total_rewards_satoshis || 0)}
                            </div>
                            <div class="stat-label">Total Rewards</div>
                        </div>
                    </div>
                    ${a.last_stake_time ? `
                        <div class="divider"></div>
                        <div class="flex-between">
                            <span class="stat-label">Last Stake</span>
                            <span class="text-sm text-hint">${timeAgo(a.last_stake_time)}</span>
                        </div>
                    ` : ''}
                </div>`;
        } else {
            html += `
                <div class="card card-stagger">
                    <div class="error-state">
                        Could not load analysis data for this address. It may not be in your watch list.
                    </div>
                </div>`;
        }

        // Stake chart
        if (stakeList.length > 0) {
            html += `
                <div class="chart-container card-stagger">
                    <div class="chart-title">Reward History</div>
                    <canvas id="stake-chart" height="200"></canvas>
                </div>`;
        }

        // Recent stakes table
        html += `<div class="section-title card-stagger">Recent Stakes</div>`;

        if (stakeList.length === 0) {
            html += `
                <div class="empty-state card-stagger">
                    <p>No stake events recorded yet.</p>
                </div>`;
        } else {
            html += `<div class="card card-stagger" style="padding: var(--space-md) var(--space-lg);">`;
            for (const stake of stakeList) {
                const amount = stake.amount_satoshis
                    ? formatDivi(stake.amount_satoshis)
                    : formatDiviFloat(stake.amount || 0);
                const height = stake.block_height || stake.height;
                const time = stake.detected_at
                    ? timeAgo(stake.detected_at)
                    : timeAgo(stake.time);

                html += `
                    <div class="stake-row">
                        <div class="stake-height" onclick="navigate('block', { hash: '${height}' })">
                            #${Number(height).toLocaleString()}
                        </div>
                        <div class="stake-time">${time}</div>
                        <div class="stake-amount">+${amount}</div>
                    </div>`;
            }
            html += `</div>`;
        }

        // Unwatch button
        html += `
            <button class="btn btn-danger btn-full mt-lg card-stagger"
                    onclick="unwatchAddress('${escapeHtml(address)}')">
                Remove Watch
            </button>`;

        html += `</div>`;
        container.innerHTML = html;

        // Render chart after DOM is ready
        if (stakeList.length > 0) {
            const canvas = document.getElementById('stake-chart');
            if (canvas) {
                createStakeChart(canvas, stakeList);
            }
        }

    } catch (e) {
        container.innerHTML = `
            <div class="view-enter">
                <div class="error-state">
                    Could not load address detail: ${escapeHtml(e.message)}
                </div>
                <button class="btn btn-ghost btn-full mt-lg" onclick="goBack()">
                    Go Back
                </button>
            </div>`;
    }
}

// Global handler for unwatch button
window.unwatchAddress = async function(address) {
    if (!confirm('Remove this address from your watch list?')) return;

    try {
        await api.removeWatch(address);
        window.haptic('success');
        window.showToast('Address removed');
        navigate('dashboard');
    } catch (e) {
        window.haptic('error');
        window.showToast('Failed to remove: ' + e.message, 'error');
    }
};
