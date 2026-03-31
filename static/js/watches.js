// ============================================================
// Stake Watch -- Watches Management View
// ============================================================
// List watched addresses with health indicators, labels,
// delete functionality, and an add-watch form.
// ============================================================

import { api } from './api.js';
import { escapeHtml } from './helpers.js';

export async function renderWatches(container) {
    try {
        const watches = await api.getWatches();

        let html = `<div class="view-enter">`;

        // Section title
        html += `
            <div class="flex-between mb-md">
                <div class="section-title" style="margin: 0;">Your Watches</div>
                <span class="badge badge-neutral">${watches.length}</span>
            </div>`;

        // Watch list
        if (watches.length === 0) {
            html += `
                <div class="empty-state card-stagger">
                    <svg class="empty-state-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/>
                        <circle cx="12" cy="12" r="3"/>
                    </svg>
                    <div class="empty-state-title">No watches yet</div>
                    <p>Add a Divi address below to start monitoring its staking activity.</p>
                </div>`;
        } else {
            for (const watch of watches) {
                const label = watch.label ? escapeHtml(watch.label) : 'Unnamed';
                html += `
                    <div class="list-item card-stagger">
                        <div class="list-item-content"
                             onclick="navigate('address-detail', { address: '${escapeHtml(watch.address)}' })"
                             style="cursor: pointer;">
                            <div class="list-item-title">${label}</div>
                            <div class="address" style="margin-top: 4px; font-size: 11px;">
                                ${escapeHtml(watch.address)}
                            </div>
                        </div>
                        <div class="list-item-actions">
                            <button class="btn btn-icon btn-danger"
                                    onclick="removeWatchFromList('${escapeHtml(watch.address)}')"
                                    title="Remove">
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <polyline points="3 6 5 6 21 6"/>
                                    <path d="M19 6l-1 14H6L5 6"/>
                                    <path d="M10 11v6"/>
                                    <path d="M14 11v6"/>
                                    <path d="M9 6V4h6v2"/>
                                </svg>
                            </button>
                        </div>
                    </div>`;
            }
        }

        // Add watch form
        html += `
            <div class="divider"></div>
            <div class="section-title card-stagger">Add New Watch</div>
            <div class="card card-stagger">
                <div class="form-group">
                    <label class="form-label" for="watch-address">Divi Address</label>
                    <input type="text"
                           id="watch-address"
                           class="form-input"
                           placeholder="D8nQRyfgS5xL7dZDC39i9s41iiCAEeq7Zk"
                           autocomplete="off"
                           autocorrect="off"
                           autocapitalize="off"
                           spellcheck="false" />
                    <div class="form-help">Enter a Divi mainnet (D...) or testnet (y...) address</div>
                </div>
                <div class="form-group">
                    <label class="form-label" for="watch-label">Label (optional)</label>
                    <input type="text"
                           id="watch-label"
                           class="form-input"
                           placeholder="My staking wallet"
                           maxlength="50" />
                </div>
                <button class="btn btn-primary btn-full" onclick="submitAddWatch()">
                    Add Watch
                </button>
            </div>`;

        html += `</div>`;
        container.innerHTML = html;

    } catch (e) {
        container.innerHTML = `
            <div class="view-enter">
                <div class="error-state">
                    Could not load watches: ${escapeHtml(e.message)}
                </div>
                <button class="btn btn-ghost btn-full mt-lg" onclick="navigate('watches')">
                    Retry
                </button>
            </div>`;
    }
}

// ----- Global Handlers -----

window.submitAddWatch = async function() {
    const addressInput = document.getElementById('watch-address');
    const labelInput = document.getElementById('watch-label');

    const address = addressInput?.value.trim();
    const label = labelInput?.value.trim() || null;

    if (!address) {
        window.haptic('warning');
        window.showToast('Please enter an address', 'error');
        addressInput?.focus();
        return;
    }

    // Basic validation
    if (!/^[DdYy][a-zA-Z0-9]{24,44}$/.test(address)) {
        window.haptic('warning');
        window.showToast('Invalid address format', 'error');
        addressInput?.focus();
        return;
    }

    try {
        await api.addWatch(address, label);
        window.haptic('success');
        window.showToast('Address added successfully');
        navigate('watches');
    } catch (e) {
        window.haptic('error');
        if (e.message.includes('409') || e.message.includes('already')) {
            window.showToast('You are already watching this address', 'error');
        } else if (e.message.includes('limit') || e.message.includes('max')) {
            window.showToast('Watch limit reached', 'error');
        } else {
            window.showToast('Failed to add: ' + e.message, 'error');
        }
    }
};

window.removeWatchFromList = async function(address) {
    if (!confirm('Remove this address from your watch list?')) return;

    try {
        await api.removeWatch(address);
        window.haptic('success');
        window.showToast('Address removed');
        navigate('watches');
    } catch (e) {
        window.haptic('error');
        window.showToast('Failed to remove: ' + e.message, 'error');
    }
};
