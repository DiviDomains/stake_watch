// ============================================================
// Stake Watch -- Alerts Configuration View
// ============================================================
// Manage blockchain alert subscriptions: large transactions,
// unusual scripts, large blocks, etc.
// ============================================================

import { api } from './api.js';
import { escapeHtml } from './helpers.js';

// Alert type metadata for display
const ALERT_TYPES = {
    large_tx:          { label: 'Large Transaction',    icon: 'arrow-up',    description: 'Alert when a transaction exceeds the threshold (in DIVI).' },
    large_block:       { label: 'Large Block',          icon: 'package',     description: 'Alert when a block exceeds the threshold size (in bytes).' },
    many_inputs:       { label: 'Many Inputs',          icon: 'git-merge',   description: 'Alert when a transaction has more inputs than the threshold.' },
    many_outputs:      { label: 'Many Outputs',         icon: 'git-branch',  description: 'Alert when a transaction has more outputs than the threshold.' },
    op_return:         { label: 'OP_RETURN Data',       icon: 'file-text',   description: 'Alert when a transaction contains OP_RETURN data.' },
    unusual_script:    { label: 'Unusual Script',       icon: 'alert-triangle', description: 'Alert on non-standard script types.' },
    anything_unusual:  { label: 'Anything Unusual',     icon: 'zap',         description: 'Catch-all: alert on any anomalous blockchain activity.' },
    lottery_block:     { label: 'Lottery Block',         icon: 'gift',        description: 'Summary of all lottery block winners when a lottery block is found.' },
};

export async function renderAlerts(container) {
    try {
        const alerts = await api.getAlerts();

        let html = `<div class="view-enter">`;

        // Section title
        html += `
            <div class="flex-between mb-md">
                <div class="section-title" style="margin: 0;">Your Alerts</div>
                <span class="badge badge-neutral">${alerts.length}</span>
            </div>`;

        // Active alerts
        if (alerts.length === 0) {
            html += `
                <div class="empty-state card-stagger">
                    <svg class="empty-state-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/>
                        <path d="M13.73 21a2 2 0 0 1-3.46 0"/>
                    </svg>
                    <div class="empty-state-title">No active alerts</div>
                    <p>Subscribe to blockchain events below to receive Telegram notifications.</p>
                </div>`;
        } else {
            for (const alert of alerts) {
                const meta = ALERT_TYPES[alert.alert_type] || {
                    label: alert.alert_type,
                    description: '',
                };
                const threshold = alert.threshold > 0
                    ? `Threshold: ${Number(alert.threshold).toLocaleString()}`
                    : 'No threshold';

                html += `
                    <div class="list-item card-stagger">
                        <div class="list-item-content">
                            <div class="list-item-title">${escapeHtml(meta.label)}</div>
                            <div class="list-item-subtitle">${threshold}</div>
                        </div>
                        <div class="list-item-actions">
                            <button class="btn btn-icon btn-danger"
                                    onclick="removeAlertSub('${escapeHtml(alert.alert_type)}')"
                                    title="Remove">
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <line x1="18" y1="6" x2="6" y2="18"/>
                                    <line x1="6" y1="6" x2="18" y2="18"/>
                                </svg>
                            </button>
                        </div>
                    </div>`;
            }
        }

        // Add alert form
        html += `
            <div class="divider"></div>
            <div class="section-title card-stagger">Add Alert</div>
            <div class="card card-stagger">
                <div class="form-group">
                    <label class="form-label" for="alert-type">Alert Type</label>
                    <select id="alert-type" class="form-select" onchange="updateAlertDescription()">
                        <option value="">Select an alert type...</option>`;

        // Only show types not already subscribed
        const subscribedTypes = new Set(alerts.map(a => a.alert_type));
        for (const [type, meta] of Object.entries(ALERT_TYPES)) {
            if (!subscribedTypes.has(type)) {
                html += `<option value="${type}">${escapeHtml(meta.label)}</option>`;
            }
        }

        html += `
                    </select>
                    <div id="alert-description" class="form-help"></div>
                </div>
                <div class="form-group" id="threshold-group" style="display: none;">
                    <label class="form-label" for="alert-threshold">Threshold</label>
                    <input type="number"
                           id="alert-threshold"
                           class="form-input"
                           placeholder="0"
                           min="0"
                           step="any" />
                    <div class="form-help" id="threshold-help">Minimum value to trigger the alert</div>
                </div>
                <button class="btn btn-primary btn-full" onclick="submitAddAlert()">
                    Subscribe
                </button>
            </div>`;

        // Info about where alerts are delivered
        html += `
            <div class="card card-stagger" style="margin-top: var(--space-lg);">
                <div class="flex gap-sm" style="align-items: flex-start;">
                    <svg style="width:18px;height:18px;flex-shrink:0;color:var(--hint);margin-top:2px;" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <circle cx="12" cy="12" r="10"/>
                        <line x1="12" y1="16" x2="12" y2="12"/>
                        <line x1="12" y1="8" x2="12.01" y2="8"/>
                    </svg>
                    <div class="text-sm text-hint">
                        Alerts are delivered as Telegram messages from the Stake Watch bot.
                        Make sure you have started the bot in your private chat.
                    </div>
                </div>
            </div>`;

        html += `</div>`;
        container.innerHTML = html;

    } catch (e) {
        container.innerHTML = `
            <div class="view-enter">
                <div class="error-state">
                    Could not load alerts: ${escapeHtml(e.message)}
                </div>
                <button class="btn btn-ghost btn-full mt-lg" onclick="navigate('alerts')">
                    Retry
                </button>
            </div>`;
    }
}

// ----- Global Handlers -----

window.updateAlertDescription = function() {
    const typeSelect = document.getElementById('alert-type');
    const descEl = document.getElementById('alert-description');
    const thresholdGroup = document.getElementById('threshold-group');
    const thresholdHelp = document.getElementById('threshold-help');

    const type = typeSelect?.value;
    const meta = ALERT_TYPES[type];

    if (meta) {
        descEl.textContent = meta.description;

        // Show threshold for types that use numeric thresholds
        const needsThreshold = ['large_tx', 'large_block', 'many_inputs', 'many_outputs'];
        if (needsThreshold.includes(type)) {
            thresholdGroup.style.display = 'block';
            if (type === 'large_tx') {
                thresholdHelp.textContent = 'Minimum transaction value in DIVI';
            } else if (type === 'large_block') {
                thresholdHelp.textContent = 'Minimum block size in bytes';
            } else {
                thresholdHelp.textContent = 'Minimum count to trigger alert';
            }
        } else {
            thresholdGroup.style.display = 'none';
        }
    } else {
        descEl.textContent = '';
        thresholdGroup.style.display = 'none';
    }
};

window.submitAddAlert = async function() {
    const typeSelect = document.getElementById('alert-type');
    const thresholdInput = document.getElementById('alert-threshold');

    const alertType = typeSelect?.value;
    if (!alertType) {
        window.haptic('warning');
        window.showToast('Please select an alert type', 'error');
        return;
    }

    const threshold = parseFloat(thresholdInput?.value) || 0;

    try {
        await api.addAlert(alertType, threshold);
        window.haptic('success');
        window.showToast('Alert subscription added');
        navigate('alerts');
    } catch (e) {
        window.haptic('error');
        window.showToast('Failed: ' + e.message, 'error');
    }
};

window.removeAlertSub = async function(alertType) {
    if (!confirm('Remove this alert subscription?')) return;

    try {
        await api.removeAlert(alertType);
        window.haptic('success');
        window.showToast('Alert removed');
        navigate('alerts');
    } catch (e) {
        window.haptic('error');
        window.showToast('Failed: ' + e.message, 'error');
    }
};
