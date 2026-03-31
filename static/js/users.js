// ============================================================
// Stake Watch -- Admin Users View
// ============================================================
// Displays all registered users with their watches and alert
// subscriptions. Only accessible to admin users.
// ============================================================

import { api } from './api.js';
import { escapeHtml, timeAgo } from './helpers.js';

export async function renderUsers(container) {
    try {
        const users = await api.getAdminUsers();

        let html = `<div class="view-enter">`;

        html += `
            <div class="flex-between mb-md">
                <div class="section-title" style="margin: 0;">All Users</div>
                <span class="badge badge-neutral">${users.length}</span>
            </div>`;

        if (users.length === 0) {
            html += `
                <div class="empty-state card-stagger">
                    <div class="empty-state-title">No users registered</div>
                </div>`;
        } else {
            for (const user of users) {
                const username = user.username
                    ? `@${escapeHtml(user.username)}`
                    : '<span style="opacity: 0.5;">no username</span>';
                const joinDate = user.created_at.split(' ')[0]; // YYYY-MM-DD

                html += `
                    <div class="card card-stagger admin-user-card">
                        <div class="card-header">
                            <div class="card-label">
                                ${username}
                            </div>
                            <span class="badge badge-neutral">${user.watch_count} watch${user.watch_count !== 1 ? 'es' : ''}</span>
                        </div>
                        <div style="display: flex; gap: 12px; margin-top: 4px; font-size: 12px; color: var(--hint);">
                            <span>ID: <code style="font-size: 11px;">${user.telegram_id}</code></span>
                            <span>Joined: ${escapeHtml(joinDate)}</span>
                        </div>`;

                // Watches list (collapsible)
                if (user.watches && user.watches.length > 0) {
                    html += `
                        <div style="margin-top: 10px;">
                            <div style="font-size: 11px; text-transform: uppercase; letter-spacing: 0.5px; color: var(--hint); margin-bottom: 6px;">
                                Watches
                            </div>`;
                    for (const w of user.watches) {
                        const wLabel = w.label ? escapeHtml(w.label) : '';
                        const lastStake = w.last_stake_at || 'Never';
                        html += `
                            <div style="display: flex; justify-content: space-between; align-items: center; padding: 4px 0; font-size: 12px;">
                                <div>
                                    <span class="address" style="font-size: 11px; cursor: pointer;"
                                          onclick="navigate('address-detail', { address: '${escapeHtml(w.address)}' })">
                                        ${escapeHtml(w.address.slice(0, 10))}...${escapeHtml(w.address.slice(-6))}
                                    </span>
                                    ${wLabel ? `<span style="color: var(--hint); margin-left: 6px;">${wLabel}</span>` : ''}
                                </div>
                                <span style="color: var(--hint); font-size: 11px; white-space: nowrap;">
                                    ${lastStake === 'Never' ? 'Never' : escapeHtml(lastStake)}
                                </span>
                            </div>`;
                    }
                    html += `</div>`;
                }

                // Alert subscriptions
                if (user.alert_subscriptions && user.alert_subscriptions.length > 0) {
                    html += `
                        <div style="margin-top: 8px;">
                            <div style="font-size: 11px; text-transform: uppercase; letter-spacing: 0.5px; color: var(--hint); margin-bottom: 4px;">
                                Alerts
                            </div>
                            <div style="display: flex; flex-wrap: wrap; gap: 4px;">`;
                    for (const alertType of user.alert_subscriptions) {
                        html += `<span class="badge badge-neutral" style="font-size: 10px;">${escapeHtml(alertType)}</span>`;
                    }
                    html += `</div></div>`;
                }

                html += `</div>`;
            }
        }

        html += `</div>`;
        container.innerHTML = html;

    } catch (e) {
        const msg = e.message || 'Unknown error';
        if (msg.includes('403') || msg.includes('Forbidden')) {
            container.innerHTML = `
                <div class="view-enter">
                    <div class="error-state">
                        Admin access required.
                    </div>
                </div>`;
        } else {
            container.innerHTML = `
                <div class="view-enter">
                    <div class="error-state">
                        Could not load users: ${escapeHtml(msg)}
                    </div>
                    <button class="btn btn-ghost btn-full mt-lg" onclick="navigate('users')">
                        Retry
                    </button>
                </div>`;
        }
    }
}
