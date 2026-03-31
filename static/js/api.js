// ============================================================
// Stake Watch -- API Client
// ============================================================
// Sends Telegram initData with authenticated requests.
// All user-specific endpoints require the X-Telegram-Init-Data
// header for HMAC validation on the server side.
// ============================================================

const API_BASE = '/api';

function getInitData() {
    return window.Telegram?.WebApp?.initData || '';
}

async function fetchApi(path, options = {}) {
    const headers = {
        'Content-Type': 'application/json',
        ...options.headers,
    };

    const initData = getInitData();
    if (initData) {
        headers['X-Telegram-Init-Data'] = initData;
    }

    const resp = await fetch(`${API_BASE}${path}`, {
        ...options,
        headers,
    });

    if (!resp.ok) {
        const error = await resp.text();
        throw new Error(`API error ${resp.status}: ${error}`);
    }

    return resp.json();
}

export const api = {
    // -- User endpoints (require auth) --
    getMe: () => fetchApi('/me'),

    getWatches: () => fetchApi('/watches'),

    addWatch: (address, label) => fetchApi('/watches', {
        method: 'POST',
        body: JSON.stringify({ address, label }),
    }),

    removeWatch: (address) => fetchApi(`/watches/${encodeURIComponent(address)}`, {
        method: 'DELETE',
    }),

    patchWatch: (address, data) => fetchApi(`/watches/${encodeURIComponent(address)}`, {
        method: 'PATCH',
        body: JSON.stringify(data),
    }),

    reorderWatches: (addresses) => fetchApi('/watches/reorder', {
        method: 'POST',
        body: JSON.stringify({ addresses }),
    }),

    getAnalysis: (address) => fetchApi(`/watches/${encodeURIComponent(address)}/analysis`),

    getStakes: (address, limit = 50) =>
        fetchApi(`/watches/${encodeURIComponent(address)}/stakes?limit=${limit}`),

    // -- Alerts (require auth) --
    getAlerts: () => fetchApi('/alerts'),

    addAlert: (alertType, threshold) => fetchApi('/alerts', {
        method: 'POST',
        body: JSON.stringify({ alert_type: alertType, threshold }),
    }),

    removeAlert: (alertType) => fetchApi(`/alerts/${encodeURIComponent(alertType)}`, {
        method: 'DELETE',
    }),

    // -- Admin endpoints (require auth + admin) --
    getAdminUsers: () => fetchApi('/admin/users'),

    // -- Explorer (no auth needed) --
    getBlocks: (limit = 20) => fetchApi(`/blocks?limit=${limit}`),

    getBlock: (hashOrHeight) => fetchApi(`/blocks/${encodeURIComponent(hashOrHeight)}`),

    getTx: (txid) => fetchApi(`/tx/${encodeURIComponent(txid)}`),

    getAddress: (address) => fetchApi(`/address/${encodeURIComponent(address)}`),

    getVaultBalance: (address) => fetchApi(`/address/${encodeURIComponent(address)}/vault`),

    search: (query) => fetchApi(`/search?q=${encodeURIComponent(query)}`),

    getNetwork: () => fetchApi('/network'),

    getDiviPrice: () => fetchApi('/price/divi'),
};
