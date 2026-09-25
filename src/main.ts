import { createApp } from 'vue';
import { createPinia } from 'pinia';
import { invoke } from '@tauri-apps/api/core';
import App from './App.vue';
import './assets/styles/variables.css';
import './assets/styles/dark.css';
import './assets/chart-switcher.css';
import './assets/workspace.css';

// 前端构建指纹（vite define 注入）。用于确认 WebView 加载的确实是最新构建。
declare const __BULL_FRONTEND_BUILD__: string;

// Disable default browser context menu
document.addEventListener('contextmenu', (e) => e.preventDefault());

const app = createApp(App);
app.use(createPinia());
app.mount('#app');

// 启动即上报：如果日志里没有这一行，说明 WebView 加载的是旧版前端（或 IPC 通道断了）
if (typeof __BULL_FRONTEND_BUILD__ === 'string') {
  void invoke('log_frontend', {
    level: 'info',
    message: `前端已启动，前端构建=${__BULL_FRONTEND_BUILD__}`,
  }).catch(() => {
    // IPC 不可用时静默：此时任何 invoke 都不会成功，继续跑只会更乱
  });
  // 顺手暴露到 window，便于在 WebView 控制台里核对
  (window as unknown as Record<string, unknown>).__BULL_FRONTEND_BUILD__ = __BULL_FRONTEND_BUILD__;
}
