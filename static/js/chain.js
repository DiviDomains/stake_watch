// ============================================================
// Stake Watch -- Chain Configuration Store
// ============================================================
// Separate module to avoid circular dependency with app.js.
// View modules import chainConfig from here, app.js populates it.
// ============================================================

import { api } from './api.js';

export let chainConfig = { name: 'Divi', ticker: 'DIVI', has_lottery: true, has_vaults: true };

export async function loadChainConfig() {
    try {
        chainConfig = await api.getChainConfig();
    } catch (e) {
        console.warn('Could not load chain config, using defaults:', e.message);
    }
}
