// src/ticker-quick.ts — independent Vue app for the floating bar's
// "quick watchlist" panel.
//
// Why a separate window instead of an overlay inside the ticker itself:
// the ticker window is a WS_EX_NOACTIVATE tool window (`apply_nonactivating_tool_window_style`)
// so it deliberately never takes keyboard focus — which is exactly what a
// search box needs.  A small dedicated window keeps the floating bar's
// carefully tuned drag / click / non-activating behaviour untouched.
import { createApp } from 'vue';
import { createPinia } from 'pinia';
import TickerQuickAdd from './components/ticker/TickerQuickAdd.vue';
import './assets/styles/variables.css';
import './assets/styles/dark.css';

// Disable default browser context menu (matches the ticker window).
document.addEventListener('contextmenu', (e) => e.preventDefault());

const app = createApp(TickerQuickAdd);
app.use(createPinia());
app.mount('#app');
