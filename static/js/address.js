// ============================================================
// Stake Watch -- Address Detail View
// ============================================================
// Full analysis of a watched address: balance, vault status,
// health, expected frequency, Chart.js reward history,
// staking ROI calculator, and recent stakes table.
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

        // Staking Calculator
        if (a && a.balance_satoshis > 0 && a.expected_interval_secs) {
            const balanceDivi = a.balance_satoshis / 1e8;
            const avgRewardDivi = (a.avg_stake_satoshis || 0) / 1e8;
            const expectedSecs = a.expected_interval_secs;
            // Stakes per year = seconds_per_year / expected_interval_secs
            const stakesPerYear = (365.25 * 24 * 3600) / expectedSecs;
            const annualRewards = stakesPerYear * avgRewardDivi;
            const annualRor = balanceDivi > 0 ? (annualRewards / balanceDivi) : 0;

            html += `
                <div class="section-title card-stagger">Staking Calculator</div>
                <div class="card card-stagger calculator-card"
                     id="staking-calculator"
                     data-balance="${balanceDivi}"
                     data-annual-ror="${annualRor}"
                     data-annual-rewards="${annualRewards}">
                    <div class="calc-current">
                        <div class="card-stats card-stats-3">
                            <div>
                                <div class="stat-value stat-value-sm">${formatDiviShort(a.balance_satoshis)}</div>
                                <div class="stat-label">Balance</div>
                            </div>
                            <div>
                                <div class="stat-value stat-value-sm text-accent" id="calc-ror">
                                    ${(annualRor * 100).toFixed(1)}%
                                </div>
                                <div class="stat-label">Annual RoR</div>
                            </div>
                            <div>
                                <div class="stat-value stat-value-sm" id="calc-usd-balance">--</div>
                                <div class="stat-label">USD Value</div>
                            </div>
                        </div>
                    </div>
                    <div class="divider"></div>
                    <div class="calc-projection">
                        <div class="calc-slider-row">
                            <label class="form-label" for="calc-years-slider">Projection Period</label>
                            <span class="calc-years-label" id="calc-years-label">5 years</span>
                        </div>
                        <input type="range" id="calc-years-slider"
                               class="calc-slider"
                               min="1" max="99" value="5"
                               oninput="updateCalculator()" />
                        <div class="calc-results">
                            <div class="calc-result-row">
                                <span class="calc-result-label">Future Balance</span>
                                <span class="calc-result-value" id="calc-future-divi">--</span>
                            </div>
                            <div class="calc-result-row">
                                <span class="calc-result-label">Future USD Value</span>
                                <span class="calc-result-value" id="calc-future-usd">--</span>
                            </div>
                            <div class="calc-result-row">
                                <span class="calc-result-label">Total Rewards Earned</span>
                                <span class="calc-result-value text-success" id="calc-total-rewards">--</span>
                            </div>
                        </div>
                    </div>
                    <div class="calc-price-note text-xs text-hint" id="calc-price-note">
                        Fetching DIVI price...
                    </div>
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

        // Initialize calculator if present
        if (document.getElementById('staking-calculator')) {
            initCalculator();
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

// ----- Calculator Logic -----

let diviPriceUsd = null;

async function initCalculator() {
    // Fetch DIVI price
    try {
        const priceData = await api.getDiviPrice();
        diviPriceUsd = priceData.usd || 0;
        const noteEl = document.getElementById('calc-price-note');
        if (noteEl) {
            noteEl.textContent = diviPriceUsd > 0
                ? `DIVI price: $${diviPriceUsd.toFixed(6)} USD (via CoinGecko)`
                : 'DIVI price not available';
        }
    } catch {
        diviPriceUsd = 0;
        const noteEl = document.getElementById('calc-price-note');
        if (noteEl) {
            noteEl.textContent = 'Could not fetch DIVI price';
        }
    }

    // Show current USD balance
    updateCalculator();
}

window.updateCalculator = function() {
    const calc = document.getElementById('staking-calculator');
    if (!calc) return;

    const balance = parseFloat(calc.dataset.balance) || 0;
    const annualRor = parseFloat(calc.dataset.annualRor) || 0;
    const slider = document.getElementById('calc-years-slider');
    const years = parseInt(slider?.value || '5', 10);

    // Update year label
    const yearLabel = document.getElementById('calc-years-label');
    if (yearLabel) {
        yearLabel.textContent = years === 1 ? '1 year' : `${years} years`;
    }

    // Compound: futureBalance = balance * (1 + annualRor)^years
    const futureBalance = balance * Math.pow(1 + annualRor, years);
    const totalRewards = futureBalance - balance;

    // Update DIVI values
    const futureDiviEl = document.getElementById('calc-future-divi');
    if (futureDiviEl) {
        futureDiviEl.textContent = formatCompactNumber(futureBalance) + ' DIVI';
    }

    const totalRewardsEl = document.getElementById('calc-total-rewards');
    if (totalRewardsEl) {
        totalRewardsEl.textContent = '+' + formatCompactNumber(totalRewards) + ' DIVI';
    }

    // Update USD values if price available
    const usdBalEl = document.getElementById('calc-usd-balance');
    const futureUsdEl = document.getElementById('calc-future-usd');

    if (diviPriceUsd && diviPriceUsd > 0) {
        const currentUsd = balance * diviPriceUsd;
        const futureUsd = futureBalance * diviPriceUsd;

        if (usdBalEl) usdBalEl.textContent = '$' + formatCompactUsd(currentUsd);
        if (futureUsdEl) futureUsdEl.textContent = '$' + formatCompactUsd(futureUsd);
    } else {
        if (usdBalEl) usdBalEl.textContent = '--';
        if (futureUsdEl) futureUsdEl.textContent = '--';
    }
};

function formatCompactNumber(num) {
    if (num >= 1e9) return (num / 1e9).toFixed(2) + 'B';
    if (num >= 1e6) return (num / 1e6).toFixed(2) + 'M';
    if (num >= 1e4) return Math.round(num).toLocaleString('en-US');
    return num.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

function formatCompactUsd(num) {
    if (num >= 1e9) return (num / 1e9).toFixed(2) + 'B';
    if (num >= 1e6) return (num / 1e6).toFixed(2) + 'M';
    if (num >= 1e3) return Math.round(num).toLocaleString('en-US');
    return num.toFixed(2);
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
