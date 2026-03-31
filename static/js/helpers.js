// ============================================================
// Stake Watch -- Shared Helpers
// ============================================================
// Formatting utilities shared across all view modules.
// ============================================================

/**
 * Format a satoshi amount as a full DIVI string with 8 decimal places.
 * Example: 701777600000000 -> "7,017,776.00000000"
 */
export function formatDivi(satoshis) {
    const abs = Math.abs(satoshis);
    const whole = Math.floor(abs / 1e8);
    const frac = Math.round(abs % 1e8);
    const sign = satoshis < 0 ? '-' : '';
    const wholeStr = whole.toLocaleString('en-US');
    const fracStr = String(frac).padStart(8, '0');
    return `${sign}${wholeStr}.${fracStr}`;
}

/**
 * Format a satoshi amount as a short DIVI string with 2 decimal places.
 * Example: 701777600000000 -> "7,017,776.00"
 */
export function formatDiviShort(satoshis) {
    const value = satoshis / 1e8;
    if (Math.abs(value) >= 1e6) {
        return (value / 1e6).toFixed(2) + 'M';
    }
    if (Math.abs(value) >= 1e4) {
        return value.toLocaleString('en-US', {
            minimumFractionDigits: 0,
            maximumFractionDigits: 0,
        });
    }
    return value.toLocaleString('en-US', {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
    });
}

/**
 * Format a DIVI float value (not satoshis) with commas.
 * Example: 7017776.0 -> "7,017,776.00000000"
 */
export function formatDiviFloat(value) {
    const whole = Math.floor(Math.abs(value));
    const frac = Math.round((Math.abs(value) - whole) * 1e8);
    const sign = value < 0 ? '-' : '';
    const wholeStr = whole.toLocaleString('en-US');
    const fracStr = String(frac).padStart(8, '0');
    return `${sign}${wholeStr}.${fracStr}`;
}

/**
 * Produce a human-friendly relative time string.
 * Accepts an ISO 8601 date string or a Unix timestamp (seconds).
 */
export function timeAgo(dateStr) {
    if (!dateStr) return 'Never';

    let timestamp;
    if (typeof dateStr === 'number') {
        timestamp = dateStr * 1000; // Unix seconds -> ms
    } else {
        timestamp = new Date(dateStr).getTime();
    }

    const now = Date.now();
    const diffMs = now - timestamp;
    if (diffMs < 0) return 'Just now';

    const secs = Math.floor(diffMs / 1000);
    if (secs < 60) return 'Just now';

    const mins = Math.floor(secs / 60);
    if (mins < 60) return mins === 1 ? '1 min ago' : `${mins} min ago`;

    const hours = Math.floor(mins / 60);
    if (hours < 24) return hours === 1 ? '1 hour ago' : `${hours} hours ago`;

    const days = Math.floor(hours / 24);
    return days === 1 ? '1 day ago' : `${days} days ago`;
}

/**
 * Format a duration in seconds into a compact human-readable string.
 * Example: 18000 -> "5h 0m", 90061 -> "1d 1h"
 */
export function formatDuration(seconds) {
    if (!seconds || seconds <= 0 || !isFinite(seconds)) return '--';

    const s = Math.round(seconds);
    const days = Math.floor(s / 86400);
    const hours = Math.floor((s % 86400) / 3600);
    const minutes = Math.floor((s % 3600) / 60);

    const parts = [];
    if (days > 0) parts.push(`${days}d`);
    if (hours > 0) parts.push(`${hours}h`);
    if (minutes > 0 && days === 0) parts.push(`${minutes}m`);

    return parts.length > 0 ? parts.join(' ') : '<1m';
}

/**
 * Format a Unix timestamp (seconds) as a local date+time string.
 */
export function formatTimestamp(unixSeconds) {
    if (!unixSeconds) return '--';
    const d = new Date(unixSeconds * 1000);
    return d.toLocaleString('en-US', {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
        hour12: false,
    });
}

/**
 * Escape HTML special characters to prevent XSS.
 */
export function escapeHtml(str) {
    if (!str) return '';
    const div = document.createElement('div');
    div.textContent = String(str);
    return div.innerHTML;
}

/**
 * Create a clickable address span that navigates to the address page.
 */
export function addressLink(address, cssClass = 'address') {
    return `<span class="${cssClass}" style="cursor:pointer;text-decoration:underline;text-decoration-style:dotted" onclick="event.stopPropagation(); navigate('address', { address: '${escapeHtml(address)}' })">${escapeHtml(address)}</span>`;
}

/**
 * Create a clickable tx hash span.
 */
export function txLink(txid, cssClass = 'tx-hash') {
    return `<span class="${cssClass}" onclick="navigate('tx', { txid: '${escapeHtml(txid)}' })">${escapeHtml(txid)}</span>`;
}

/**
 * Create a clickable block height span.
 */
export function blockLink(heightOrHash, displayText, cssClass = '') {
    const text = displayText || heightOrHash;
    return `<span class="${cssClass || 'text-accent text-mono'}" style="cursor:pointer" onclick="navigate('block', { hash: '${escapeHtml(String(heightOrHash))}' })">${escapeHtml(String(text))}</span>`;
}

/**
 * Trigger a file download. Uses data URI instead of blob URL
 * because Telegram WebView doesn't support blob: URLs.
 */
function triggerDownload(content, mimeType, filename) {
    const dataUri = `data:${mimeType};charset=utf-8,` + encodeURIComponent(content);

    // In Telegram WebView, use openLink to open in external browser
    if (window.Telegram?.WebApp?.openLink) {
        window.Telegram.WebApp.openLink(dataUri);
        return;
    }

    // Standard browser: use anchor element
    const a = document.createElement('a');
    a.href = dataUri;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
}

/**
 * Trigger a JSON file download.
 */
export function downloadJson(data, filename) {
    triggerDownload(JSON.stringify(data, null, 2), 'application/json', filename);
}

/**
 * Trigger a CSV file download.
 */
export function downloadCsv(rows, headers, filename) {
    const escape = (v) => {
        const s = String(v ?? '');
        return s.includes(',') || s.includes('"') || s.includes('\n')
            ? `"${s.replace(/"/g, '""')}"`
            : s;
    };
    const csv = [headers.map(escape).join(','), ...rows.map(r => r.map(escape).join(','))].join('\n');
    triggerDownload(csv, 'text/csv', filename);
}
