// ============================================================
// Stake Watch -- Chart.js Configuration
// ============================================================
// Creates a reward history chart showing daily rewards as bars
// and cumulative total as a line.
// ============================================================

import { chainConfig } from './chain.js';

/**
 * Create a stake reward history chart.
 * @param {HTMLCanvasElement} canvas
 * @param {Array} stakes - Array of stake events (sorted newest-first from API)
 */
export function createStakeChart(canvas, stakes) {
    if (!stakes || stakes.length === 0) return null;

    // Sort oldest first
    const chronological = [...stakes].reverse();

    // Aggregate rewards by day (using block height as proxy: 1440 blocks ≈ 1 day)
    const BLOCKS_PER_DAY = 1440;
    const firstHeight = chronological[0].block_height || chronological[0].height || 0;
    const dailyMap = new Map(); // dayIndex -> { totalReward, count }

    for (const stake of chronological) {
        const height = stake.block_height || stake.height || 0;
        let rewardDivi;
        if (stake.amount_satoshis != null) {
            rewardDivi = stake.amount_satoshis / 1e8;
        } else if (stake.amount_divi != null) {
            rewardDivi = parseFloat(stake.amount_divi);
        } else {
            rewardDivi = 0;
        }

        const dayIndex = Math.floor((height - firstHeight) / BLOCKS_PER_DAY);
        if (!dailyMap.has(dayIndex)) {
            dailyMap.set(dayIndex, { totalReward: 0, count: 0, height });
        }
        const day = dailyMap.get(dayIndex);
        day.totalReward += rewardDivi;
        day.count += 1;
    }

    // Build chart data from daily aggregates
    const days = Array.from(dailyMap.entries()).sort((a, b) => a[0] - b[0]);
    const labels = [];
    const dailyRewards = [];
    const cumulative = [];
    let runningTotal = 0;

    for (const [dayIdx, data] of days) {
        const daysAgo = days[days.length - 1][0] - dayIdx;
        labels.push(daysAgo === 0 ? 'Today' : daysAgo === 1 ? '1d ago' : `${daysAgo}d ago`);
        dailyRewards.push(Math.round(data.totalReward * 100) / 100);
        runningTotal += data.totalReward;
        cumulative.push(Math.round(runningTotal * 100) / 100);
    }

    // Theme colors
    const style = getComputedStyle(document.documentElement);
    const accentColor = style.getPropertyValue('--accent').trim() || '#34d399';
    const hintColor = style.getPropertyValue('--hint').trim() || '#7c7e8a';
    const borderColor = style.getPropertyValue('--border').trim() || 'rgba(255,255,255,0.1)';
    const textColor = style.getPropertyValue('--text').trim() || '#e8e9ed';

    const accentRgb = hexToRgb(accentColor) || { r: 52, g: 211, b: 153 };
    const gradientFill = canvas.getContext('2d').createLinearGradient(0, 0, 0, canvas.height);
    gradientFill.addColorStop(0, `rgba(${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}, 0.25)`);
    gradientFill.addColorStop(1, `rgba(${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}, 0.02)`);

    const chart = new Chart(canvas, {
        type: 'bar',
        data: {
            labels,
            datasets: [
                {
                    label: `Daily Rewards (${chainConfig.ticker})`,
                    data: dailyRewards,
                    type: 'bar',
                    backgroundColor: `rgba(${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}, 0.5)`,
                    borderColor: accentColor,
                    borderWidth: 1,
                    borderRadius: 4,
                    yAxisID: 'y',
                    order: 2,
                },
                {
                    label: `Cumulative (${chainConfig.ticker})`,
                    data: cumulative,
                    type: 'line',
                    borderColor: `rgba(${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}, 0.6)`,
                    backgroundColor: gradientFill,
                    borderWidth: 2,
                    pointRadius: 0,
                    pointHoverRadius: 4,
                    fill: true,
                    tension: 0.3,
                    yAxisID: 'y1',
                    order: 1,
                },
            ],
        },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            interaction: {
                mode: 'index',
                intersect: false,
            },
            plugins: {
                legend: {
                    display: true,
                    position: 'bottom',
                    labels: {
                        color: hintColor,
                        font: { family: "'DM Sans', sans-serif", size: 10 },
                        padding: 10,
                        usePointStyle: true,
                        pointStyleWidth: 8,
                    },
                },
                tooltip: {
                    backgroundColor: 'rgba(0, 0, 0, 0.85)',
                    titleColor: textColor,
                    bodyColor: textColor,
                    borderColor: borderColor,
                    borderWidth: 1,
                    titleFont: { family: "'JetBrains Mono', monospace", size: 11 },
                    bodyFont: { family: "'JetBrains Mono', monospace", size: 12 },
                    padding: 10,
                    cornerRadius: 8,
                    callbacks: {
                        label: function(ctx) {
                            const val = ctx.parsed.y;
                            return ctx.dataset.label + ': ' + val.toLocaleString('en-US', {
                                minimumFractionDigits: 2,
                                maximumFractionDigits: 2,
                            }) + ' ' + chainConfig.ticker;
                        },
                    },
                },
            },
            scales: {
                x: {
                    display: true,
                    grid: { display: false },
                    ticks: {
                        color: hintColor,
                        font: { family: "'JetBrains Mono', monospace", size: 9 },
                        maxRotation: 0,
                        maxTicksLimit: 7,
                    },
                    border: { display: false },
                },
                y: {
                    display: true,
                    position: 'left',
                    title: {
                        display: true,
                        text: 'Daily',
                        color: hintColor,
                        font: { size: 9 },
                    },
                    grid: { color: borderColor, lineWidth: 0.5 },
                    ticks: {
                        color: hintColor,
                        font: { family: "'JetBrains Mono', monospace", size: 10 },
                        callback: (v) => v.toLocaleString(),
                    },
                    border: { display: false },
                },
                y1: {
                    display: true,
                    position: 'right',
                    title: {
                        display: true,
                        text: 'Total',
                        color: hintColor,
                        font: { size: 9 },
                    },
                    grid: { display: false },
                    ticks: {
                        color: hintColor,
                        font: { family: "'JetBrains Mono', monospace", size: 10 },
                        callback: (v) => v >= 1000 ? (v / 1000).toFixed(0) + 'K' : v.toLocaleString(),
                    },
                    border: { display: false },
                },
            },
        },
    });

    return chart;
}

/**
 * Convert a hex color string to { r, g, b }.
 */
function hexToRgb(hex) {
    if (!hex) return null;
    hex = hex.replace(/^#/, '');
    if (hex.length === 3) {
        hex = hex[0] + hex[0] + hex[1] + hex[1] + hex[2] + hex[2];
    }
    const num = parseInt(hex, 16);
    if (isNaN(num)) return null;
    return {
        r: (num >> 16) & 255,
        g: (num >> 8) & 255,
        b: num & 255,
    };
}
