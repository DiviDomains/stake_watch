// ============================================================
// Stake Watch -- Live Block Feed (SSE)
// ============================================================
// Connects to the server-sent events endpoint and prepends
// new blocks to the explorer block list in real time.
// ============================================================

import { escapeHtml, timeAgo } from './helpers.js';

/**
 * Start an SSE connection to the block feed endpoint.
 * New blocks are prepended to the given container as block-card elements.
 *
 * Returns the EventSource instance so the caller can close it
 * when navigating away.
 */
export function startBlockFeed(container) {
    let evtSource;

    try {
        evtSource = new EventSource('/api/feed');
    } catch (e) {
        // SSE not supported or endpoint not available
        console.warn('Block feed SSE not available:', e);
        return null;
    }

    evtSource.onmessage = (event) => {
        let block;
        try {
            block = JSON.parse(event.data);
        } catch {
            return;
        }

        const height = block.height;
        const hash = block.hash;
        const txCount = block.tx ? block.tx.length : (block.tx_count || 0);
        const time = block.time ? timeAgo(block.time) : 'Just now';

        const el = document.createElement('div');
        el.className = 'block-card card-stagger';
        el.style.animationDelay = '0ms';
        el.onclick = () => window.navigate('block', { hash });
        el.innerHTML = `
            <div class="block-height">#${Number(height).toLocaleString()}</div>
            <div class="block-info">
                <div class="block-hash-preview">${escapeHtml(hash)}</div>
                <div class="block-meta">
                    <span class="block-meta-item">${time}</span>
                </div>
            </div>
            <div class="block-tx-count">${txCount} tx</div>`;

        // Remove the loading indicator if it's still there
        const loading = container.querySelector('.loading');
        if (loading) loading.remove();

        // Prepend the new block
        container.insertBefore(el, container.firstChild);

        // Keep only the most recent 50 blocks in the DOM
        while (container.children.length > 50) {
            container.removeChild(container.lastChild);
        }

        // Haptic feedback for new block
        if (window.haptic) {
            window.haptic('light');
        }
    };

    evtSource.onerror = () => {
        // SSE will auto-reconnect; no action needed.
        // Suppress console noise.
    };

    return evtSource;
}
