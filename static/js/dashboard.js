// ============================================================
// Stake Watch -- Dashboard View
// ============================================================
// Portfolio summary with aggregated balances, today's rewards,
// and per-address cards with health indicators.
// ============================================================

import { api } from './api.js';
import { formatDivi, formatDiviShort, timeAgo, formatDuration, escapeHtml } from './helpers.js';

export async function renderDashboard(container) {
    try {
        const watches = await api.getWatches();

        if (!watches || watches.length === 0) {
            container.innerHTML = `
                <div class="view-enter">
                    <div class="empty-state">
                        <svg class="empty-state-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/>
                            <circle cx="12" cy="12" r="3"/>
                        </svg>
                        <div class="empty-state-title">No addresses watched</div>
                        <p>Add your first Divi staking address to start monitoring rewards.</p>
                        <button class="btn btn-primary mt-lg" onclick="navigate('watches')">
                            Add Watch
                        </button>
                    </div>
                </div>`;
            return;
        }

        // Fetch analysis for each watched address in parallel
        const analyses = await Promise.allSettled(
            watches.map(w => api.getAnalysis(w.address))
        );

        // Build enriched data
        const enriched = watches.map((watch, i) => {
            const analysis = analyses[i].status === 'fulfilled' ? analyses[i].value : null;
            return { ...watch, analysis };
        });

        // Compute portfolio aggregates
        let totalBalance = 0;
        let todayRewards = 0;
        let activeCount = 0;

        for (const item of enriched) {
            if (item.analysis) {
                totalBalance += item.analysis.balance_satoshis || 0;
                todayRewards += item.analysis.rewards_24h_satoshis || 0;
                if (item.analysis.health !== 'nodata') {
                    activeCount++;
                }
            }
        }

        let html = `<div class="view-enter">`;

        // Portfolio summary card
        html += `
            <div class="portfolio-summary card-stagger">
                <div class="portfolio-title">Total Portfolio</div>
                <div class="portfolio-balance">
                    ${formatDiviShort(totalBalance)}<span class="currency">DIVI</span>
                </div>
                <div class="portfolio-row">
                    <div>
                        <div class="stat-value stat-value-sm text-success">
                            +${formatDiviShort(todayRewards)}
                        </div>
                        <div class="stat-label">24h Rewards</div>
                    </div>
                    <div>
                        <div class="stat-value stat-value-sm">${watches.length}</div>
                        <div class="stat-label">Watching</div>
                    </div>
                    <div>
                        <div class="stat-value stat-value-sm">${activeCount}</div>
                        <div class="stat-label">Active</div>
                    </div>
                </div>
            </div>`;

        // Address cards
        html += `<div class="section-title">Watched Addresses</div>`;

        for (const item of enriched) {
            const a = item.analysis;
            const label = item.label ? escapeHtml(item.label) : 'Unnamed';
            const healthClass = a ? `health-${a.health}` : 'health-nodata';
            const healthText = a ? getHealthText(a.health) : 'No data';
            const balance = a ? formatDiviShort(a.balance_satoshis || 0) : '--';
            const isVault = a?.is_vault || false;
            const stakesDay = a?.stakes_24h ?? '--';
            const expectedFreq = a?.expected_interval_secs
                ? formatDuration(a.expected_interval_secs)
                : '--';
            const lastStake = a?.last_stake_time ? timeAgo(a.last_stake_time) : 'Never';

            html += `
                <div class="card card-clickable card-stagger ${healthClass}"
                     onclick="navigate('address-detail', { address: '${escapeHtml(item.address)}' })">
                    <div class="card-header">
                        <div class="card-label">
                            <span class="health-dot"></span>
                            ${label}
                            ${isVault ? '<span class="vault-badge">Vault</span>' : ''}
                        </div>
                        <span class="health-label">${healthText}</span>
                    </div>
                    <div class="address" onclick="event.stopPropagation(); navigate('address', { address: '${escapeHtml(item.address)}' })">
                        ${escapeHtml(item.address)}
                    </div>
                    <div class="card-stats">
                        <div>
                            <div class="stat-value stat-value-sm">${balance}</div>
                            <div class="stat-label">Balance</div>
                        </div>
                        <div>
                            <div class="stat-value stat-value-sm">${stakesDay}</div>
                            <div class="stat-label">Stakes / 24h</div>
                        </div>
                        <div>
                            <div class="stat-value stat-value-sm">${expectedFreq}</div>
                            <div class="stat-label">Expected</div>
                        </div>
                        <div>
                            <div class="stat-value stat-value-sm text-xs">${lastStake}</div>
                            <div class="stat-label">Last Stake</div>
                        </div>
                    </div>
                </div>`;
        }

        html += `</div>`;
        container.innerHTML = html;

    } catch (e) {
        container.innerHTML = `
            <div class="view-enter">
                <div class="error-state">
                    Could not load dashboard: ${escapeHtml(e.message)}
                </div>
                <button class="btn btn-ghost btn-full mt-lg" onclick="navigate('dashboard')">
                    Retry
                </button>
            </div>`;
    }
}

function getHealthText(health) {
    switch (health) {
        case 'healthy': return 'Healthy';
        case 'overdue': return 'Overdue';
        case 'nodata': return 'No data';
        default: return health;
    }
}
