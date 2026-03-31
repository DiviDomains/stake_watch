// ============================================================
// Stake Watch -- Chart.js Configuration
// ============================================================
// Creates a reward history line chart using Telegram theme
// colors and appropriate styling for a mobile crypto dashboard.
// ============================================================

/**
 * Create a stake reward line chart on the given canvas element.
 *
 * @param {HTMLCanvasElement} canvas  The target canvas
 * @param {Array} stakes             Array of stake events (sorted newest-first from API)
 */
export function createStakeChart(canvas, stakes) {
    if (!stakes || stakes.length === 0) return null;

    // Reverse to chronological order (oldest first)
    const chronological = [...stakes].reverse();

    // Build chart data
    const labels = [];
    const rewards = [];
    const cumulative = [];
    let runningTotal = 0;

    for (const stake of chronological) {
        // Use block height as label
        const height = stake.block_height || stake.height || 0;
        labels.push('#' + Number(height).toLocaleString());

        // Convert to DIVI
        let rewardDivi;
        if (stake.amount_satoshis != null) {
            rewardDivi = stake.amount_satoshis / 1e8;
        } else if (stake.amount != null) {
            rewardDivi = stake.amount;
        } else {
            rewardDivi = 0;
        }

        rewards.push(rewardDivi);
        runningTotal += rewardDivi;
        cumulative.push(runningTotal);
    }

    // Get computed CSS variables from the document for Telegram theme colors
    const style = getComputedStyle(document.documentElement);
    const accentColor = style.getPropertyValue('--accent').trim() || '#34d399';
    const hintColor = style.getPropertyValue('--hint').trim() || '#7c7e8a';
    const borderColor = style.getPropertyValue('--border').trim() || 'rgba(255,255,255,0.1)';
    const textColor = style.getPropertyValue('--text').trim() || '#e8e9ed';

    // Parse accent color for gradient fill
    const accentRgb = hexToRgb(accentColor) || { r: 52, g: 211, b: 153 };
    const gradientFill = canvas.getContext('2d').createLinearGradient(0, 0, 0, canvas.height);
    gradientFill.addColorStop(0, `rgba(${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}, 0.3)`);
    gradientFill.addColorStop(1, `rgba(${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}, 0.02)`);

    const datasets = [];

    // Show individual rewards as bar-like points if fewer than 30 stakes
    if (chronological.length <= 30) {
        datasets.push({
            label: 'Reward (DIVI)',
            data: rewards,
            type: 'bar',
            backgroundColor: `rgba(${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}, 0.4)`,
            borderColor: accentColor,
            borderWidth: 1,
            borderRadius: 3,
            yAxisID: 'y',
            order: 2,
        });
    }

    // Cumulative line
    datasets.push({
        label: 'Total Rewards (DIVI)',
        data: cumulative,
        type: 'line',
        borderColor: accentColor,
        backgroundColor: gradientFill,
        borderWidth: 2,
        pointRadius: chronological.length > 20 ? 0 : 3,
        pointHoverRadius: 5,
        pointBackgroundColor: accentColor,
        pointBorderColor: accentColor,
        fill: true,
        tension: 0.3,
        yAxisID: 'y1',
        order: 1,
    });

    const chart = new Chart(canvas, {
        type: 'bar', // base type; datasets override
        data: {
            labels,
            datasets,
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
                    display: datasets.length > 1,
                    position: 'bottom',
                    labels: {
                        color: hintColor,
                        font: {
                            family: "'DM Sans', sans-serif",
                            size: 11,
                        },
                        padding: 12,
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
                    titleFont: {
                        family: "'JetBrains Mono', monospace",
                        size: 11,
                    },
                    bodyFont: {
                        family: "'JetBrains Mono', monospace",
                        size: 12,
                    },
                    padding: 10,
                    cornerRadius: 8,
                    displayColors: false,
                    callbacks: {
                        label: function(context) {
                            const val = context.parsed.y;
                            return context.dataset.label + ': ' + val.toLocaleString('en-US', {
                                minimumFractionDigits: 2,
                                maximumFractionDigits: 2,
                            }) + ' DIVI';
                        },
                    },
                },
            },
            scales: {
                x: {
                    display: true,
                    grid: {
                        display: false,
                    },
                    ticks: {
                        color: hintColor,
                        font: {
                            family: "'JetBrains Mono', monospace",
                            size: 9,
                        },
                        maxRotation: 45,
                        maxTicksLimit: 8,
                    },
                    border: {
                        display: false,
                    },
                },
                y: {
                    display: chronological.length <= 30,
                    position: 'left',
                    grid: {
                        color: borderColor,
                        lineWidth: 0.5,
                    },
                    ticks: {
                        color: hintColor,
                        font: {
                            family: "'JetBrains Mono', monospace",
                            size: 10,
                        },
                        callback: (v) => v.toLocaleString(),
                    },
                    border: {
                        display: false,
                    },
                },
                y1: {
                    display: true,
                    position: datasets.length > 1 ? 'right' : 'left',
                    grid: {
                        display: datasets.length <= 1,
                        color: borderColor,
                        lineWidth: 0.5,
                    },
                    ticks: {
                        color: hintColor,
                        font: {
                            family: "'JetBrains Mono', monospace",
                            size: 10,
                        },
                        callback: (v) => v.toLocaleString(),
                    },
                    border: {
                        display: false,
                    },
                },
            },
        },
    });

    return chart;
}

/**
 * Parse a hex color string (#RRGGBB or #RGB) into {r, g, b}.
 */
function hexToRgb(hex) {
    if (!hex) return null;

    // Remove leading #
    hex = hex.replace(/^#/, '');

    // Handle 3-char hex
    if (hex.length === 3) {
        hex = hex[0] + hex[0] + hex[1] + hex[1] + hex[2] + hex[2];
    }

    if (hex.length !== 6) return null;

    const num = parseInt(hex, 16);
    if (isNaN(num)) return null;

    return {
        r: (num >> 16) & 255,
        g: (num >> 8) & 255,
        b: num & 255,
    };
}
