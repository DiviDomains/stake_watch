// ============================================================
// Stake Watch -- Main SPA Router
// ============================================================
// Initializes Telegram WebApp, handles navigation between views,
// supports hash-based deep links for explorer pages.
// ============================================================

import { api } from './api.js';
import { renderDashboard } from './dashboard.js';
import { renderExplorer, renderBlockDetail, renderTxDetail, renderAddressPage } from './explorer.js';
import { renderWatches } from './watches.js';
import { renderAlerts } from './alerts.js';
import { renderAddressDetail } from './address.js';

// ----- Telegram WebApp Initialization -----

const tg = window.Telegram?.WebApp;
if (tg) {
    tg.ready();
    tg.expand();
    tg.enableClosingConfirmation();

    // Apply Telegram header color to match the app
    if (tg.setHeaderColor) {
        tg.setHeaderColor('secondary_bg_color');
    }
    if (tg.setBackgroundColor) {
        tg.setBackgroundColor('secondary_bg_color');
    }
}

// ----- DOM References -----

const content = document.getElementById('content');
const navBtns = document.querySelectorAll('.nav-btn');

// Track current view for back button behavior
let viewHistory = [];
let currentView = null;

// ----- Navigation -----

function navigate(view, params = {}, pushHistory = true) {
    // Push current state to history for back navigation
    if (pushHistory && currentView) {
        viewHistory.push({ view: currentView.view, params: currentView.params });
    }

    currentView = { view, params };

    // Update nav active state (only for top-level views)
    const topViews = ['dashboard', 'explorer', 'watches', 'alerts'];
    navBtns.forEach(btn => {
        if (topViews.includes(view)) {
            btn.classList.toggle('active', btn.dataset.view === view);
        }
    });

    // Show Telegram back button for detail views
    if (tg?.BackButton) {
        if (!topViews.includes(view)) {
            tg.BackButton.show();
            tg.BackButton.onClick(() => goBack());
        } else {
            tg.BackButton.hide();
            viewHistory = []; // Clear history when navigating to top-level
        }
    }

    // Clear and show loading
    content.innerHTML = '<div class="loading">Loading...</div>';

    // Render the target view
    switch (view) {
        case 'dashboard':
            renderDashboard(content);
            break;
        case 'explorer':
            renderExplorer(content);
            break;
        case 'watches':
            renderWatches(content);
            break;
        case 'alerts':
            renderAlerts(content);
            break;
        case 'block':
            renderBlockDetail(content, params.hash);
            break;
        case 'tx':
            renderTxDetail(content, params.txid);
            break;
        case 'address':
            renderAddressPage(content, params.address);
            break;
        case 'address-detail':
            renderAddressDetail(content, params.address);
            break;
        default:
            renderDashboard(content);
    }

    // Scroll to top on navigation
    content.scrollTo(0, 0);
}

function goBack() {
    if (viewHistory.length > 0) {
        const prev = viewHistory.pop();
        navigate(prev.view, prev.params, false);
    } else {
        navigate('dashboard', {}, false);
    }
}

// Make navigate globally accessible for onclick handlers
window.navigate = navigate;
window.goBack = goBack;

// ----- Nav Button Clicks -----

navBtns.forEach(btn => {
    btn.addEventListener('click', () => {
        navigate(btn.dataset.view);
    });
});

// ----- Hash-Based Routing for Deep Links -----

function handleHash() {
    const hash = window.location.hash.slice(1);
    if (!hash) {
        navigate('dashboard', {}, false);
        return;
    }

    if (hash.startsWith('/block/')) {
        navigate('block', { hash: hash.slice(7) }, false);
    } else if (hash.startsWith('/tx/')) {
        navigate('tx', { txid: hash.slice(4) }, false);
    } else if (hash.startsWith('/address/')) {
        navigate('address', { address: hash.slice(9) }, false);
    } else if (hash.startsWith('/watch/')) {
        navigate('address-detail', { address: hash.slice(7) }, false);
    } else {
        navigate('dashboard', {}, false);
    }
}

window.addEventListener('hashchange', handleHash);

// ----- Toast Notifications -----

window.showToast = function showToast(message, type = 'success', duration = 3000) {
    // Remove existing toast if any
    const existing = document.querySelector('.toast');
    if (existing) existing.remove();

    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;
    toast.textContent = message;
    document.body.appendChild(toast);

    setTimeout(() => {
        toast.classList.add('toast-exit');
        setTimeout(() => toast.remove(), 200);
    }, duration);
};

// ----- Utility: Haptic Feedback -----

window.haptic = function haptic(type = 'light') {
    if (tg?.HapticFeedback) {
        switch (type) {
            case 'light':
                tg.HapticFeedback.impactOccurred('light');
                break;
            case 'medium':
                tg.HapticFeedback.impactOccurred('medium');
                break;
            case 'heavy':
                tg.HapticFeedback.impactOccurred('heavy');
                break;
            case 'success':
                tg.HapticFeedback.notificationOccurred('success');
                break;
            case 'error':
                tg.HapticFeedback.notificationOccurred('error');
                break;
            case 'warning':
                tg.HapticFeedback.notificationOccurred('warning');
                break;
        }
    }
};

// ----- Initial Load -----

handleHash();
