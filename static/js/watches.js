// ============================================================
// Stake Watch -- Watches Management View
// ============================================================
// List watched addresses with health indicators, labels,
// delete functionality, reorder buttons, portfolio toggle,
// and an add-watch form.
// ============================================================

import { api } from './api.js';
import { chainConfig } from './chain.js';
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
                    <p>Add a ${chainConfig.name} address below to start monitoring its staking activity.</p>
                </div>`;
        } else {
            html += `<div id="watch-list">`;
            for (let i = 0; i < watches.length; i++) {
                const watch = watches[i];
                const label = watch.label ? escapeHtml(watch.label) : 'Unnamed';
                const isFirst = i === 0;
                const isLast = i === watches.length - 1;
                const portfolioChecked = watch.include_in_portfolio ? 'checked' : '';
                const portfolioTitle = watch.include_in_portfolio
                    ? 'Included in portfolio totals'
                    : 'Excluded from portfolio totals';

                html += `
                    <div class="list-item card-stagger" data-address="${escapeHtml(watch.address)}">
                        <div class="list-item-reorder">
                            <button class="btn btn-icon btn-ghost btn-reorder"
                                    onclick="moveWatchUp('${escapeHtml(watch.address)}')"
                                    title="Move up"
                                    ${isFirst ? 'disabled' : ''}>
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                    <polyline points="18 15 12 9 6 15"/>
                                </svg>
                            </button>
                            <button class="btn btn-icon btn-ghost btn-reorder"
                                    onclick="moveWatchDown('${escapeHtml(watch.address)}')"
                                    title="Move down"
                                    ${isLast ? 'disabled' : ''}>
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                    <polyline points="6 9 12 15 18 9"/>
                                </svg>
                            </button>
                        </div>
                        <div class="list-item-content"
                             onclick="navigate('address-detail', { address: '${escapeHtml(watch.address)}' })"
                             style="cursor: pointer;">
                            <div class="list-item-title">${label}</div>
                            <div class="address" style="margin-top: 4px; font-size: 11px;">
                                ${escapeHtml(watch.address)}
                            </div>
                        </div>
                        <div class="list-item-actions">
                            <label class="portfolio-toggle" title="${portfolioTitle}">
                                <input type="checkbox" ${portfolioChecked}
                                       onchange="togglePortfolio('${escapeHtml(watch.address)}', this.checked)" />
                                <span class="portfolio-toggle-icon">
                                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                        <path d="M12 2L2 7l10 5 10-5-10-5z"/>
                                        <path d="M2 17l10 5 10-5"/>
                                        <path d="M2 12l10 5 10-5"/>
                                    </svg>
                                </span>
                            </label>
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
            html += `</div>`;
        }

        // Add watch form
        html += `
            <div class="divider"></div>
            <div class="section-title card-stagger">Add New Watch</div>`;

        // Suggested watches (from chain config)
        if (chainConfig.suggested_watches && chainConfig.suggested_watches.length > 0) {
            const watchedAddrs = new Set(watches.map(w => w.address));
            const available = chainConfig.suggested_watches.filter(s => !watchedAddrs.has(s.address));
            if (available.length > 0) {
                html += `
                    <div class="card card-stagger" style="margin-bottom: 12px;">
                        <div class="form-label" style="margin-bottom: 8px;">Quick Add</div>
                        <div style="display: flex; flex-wrap: wrap; gap: 8px;">`;
                for (const s of available) {
                    html += `<button class="btn btn-ghost" style="font-size: 12px; padding: 6px 12px;"
                                     onclick="addSuggestedWatch('${escapeHtml(s.address)}', '${escapeHtml(s.label)}')"
                                     title="${escapeHtml(s.address)}">${escapeHtml(s.label)}</button>`;
                }
                html += `</div></div>`;
            }
        }

        html += `
            <div class="card card-stagger">
                <div class="form-group">
                    <label class="form-label" for="watch-address">${chainConfig.name} Address</label>
                    <input type="text"
                           id="watch-address"
                           class="form-input"
                           placeholder="D8nQRyfgS5xL7dZDC39i9s41iiCAEeq7Zk"
                           autocomplete="off"
                           autocorrect="off"
                           autocapitalize="off"
                           spellcheck="false" />
                    <div class="form-help">Enter a ${chainConfig.name} address</div>
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

    // Basic validation using chain-configured prefixes
    const prefixes = (chainConfig.address_prefixes || ['D', 'y']).join('');
    const prefixRegex = new RegExp(`^[${prefixes}][a-zA-Z0-9]{24,44}$`);
    if (!prefixRegex.test(address)) {
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

window.addSuggestedWatch = async function(address, label) {
    try {
        await api.addWatch(address, label);
        window.haptic('success');
        window.showToast(`Added ${label}`);
        navigate('watches');
    } catch (e) {
        window.haptic('error');
        if (e.message.includes('409') || e.message.includes('already')) {
            window.showToast('Already watching this address', 'error');
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

window.togglePortfolio = async function(address, include) {
    try {
        await api.patchWatch(address, { include_in_portfolio: include });
        window.haptic('light');
        window.showToast(include ? 'Included in portfolio' : 'Excluded from portfolio');
    } catch (e) {
        window.haptic('error');
        window.showToast('Failed to update: ' + e.message, 'error');
        // Revert checkbox
        navigate('watches');
    }
};

window.moveWatchUp = async function(address) {
    await reorderWatch(address, -1);
};

window.moveWatchDown = async function(address) {
    await reorderWatch(address, 1);
};

async function reorderWatch(address, direction) {
    const list = document.getElementById('watch-list');
    if (!list) return;

    const items = Array.from(list.querySelectorAll('.list-item[data-address]'));
    const addresses = items.map(el => el.dataset.address);
    const idx = addresses.indexOf(address);

    if (idx < 0) return;
    const newIdx = idx + direction;
    if (newIdx < 0 || newIdx >= addresses.length) return;

    // Swap
    [addresses[idx], addresses[newIdx]] = [addresses[newIdx], addresses[idx]];

    try {
        await api.reorderWatches(addresses);
        window.haptic('light');
        navigate('watches');
    } catch (e) {
        window.haptic('error');
        window.showToast('Failed to reorder: ' + e.message, 'error');
    }
}
