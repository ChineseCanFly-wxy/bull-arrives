<script setup lang="ts">
import { onMounted, onUnmounted, ref, computed, onErrorCaptured } from 'vue';
import { NConfigProvider, darkTheme, lightTheme, NMessageProvider, type GlobalThemeOverrides } from 'naive-ui';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useSettingsStore, type MarketSessionInfo } from '@/stores/settings';
import { useWatchlistStore } from '@/stores/watchlist';
import { useQuoteStore } from '@/stores/quote';
import { useUpdaterStore } from '@/stores/updater';
import AppLayout from '@/components/layout/AppLayout.vue';
import UpdateDialog from '@/components/updater/UpdateDialog.vue';
import AlertNotifications from '@/components/settings/AlertNotifications.vue';
import { useUpdateCheck } from '@/composables/useUpdateCheck';

const settings = useSettingsStore();
const watchlist = useWatchlistStore();
const quote = useQuoteStore();
let unlistenSession: UnlistenFn | null = null;
// Initialize updater event listeners early so backend events during
// startup are not missed (Pinia stores are lazy-initialized).
useUpdaterStore().initListeners();
// Composables must be called at setup top-level (not inside lifecycle
// hooks) per Vue 3 convention — ensures hooks are registered correctly
// even if the component is activated/deactivated by <KeepAlive>.
const { performStartupCheck } = useUpdateCheck();

const initError = ref<string | null>(null);
const initReady = ref(false);
const appError = ref<string | null>(null);

// 全局错误边界 — 捕获子组件中的未处理错误，防止静默崩溃
onErrorCaptured((err, instance, info) => {
  const componentName = instance?.$?.type?.__name
    || (instance?.$ as any)?.type?.name
    || 'Unknown';
  const msg = `[${componentName}] ${String(err).slice(0, 200)}`;
  console.error('[App] onErrorCaptured:', msg, info);

  if (!appError.value) {
    appError.value = `界面错误: ${msg}`;
  }
  // 阻止错误继续传播到浏览器控制台
  return false;
});

// Match Naive UI controls to the selected brightness and visual style.
const themeOverrides = computed<GlobalThemeOverrides>(() => {
  const isDark = settings.theme === 'dark';
  const isModern = settings.visualStyle === 'modern';
  const isTrading = settings.visualStyle === 'trading';
  const accent = isModern ? (isDark ? '#81bcff' : '#1659b7')
    : isTrading ? (isDark ? '#6dd2e0' : '#086f80')
    : (isDark ? '#58a6ff' : '#0969da');
  const border = isModern ? (isDark ? '#314159' : '#d8e1ed')
    : isTrading ? (isDark ? '#254051' : '#c9d9df')
    : (isDark ? '#1e293b' : '#d0d7de');
  const hover = isModern ? (isDark ? '#acd4ff' : '#3276cf')
    : isTrading ? (isDark ? '#a6eff3' : '#138296')
    : (isDark ? '#79b8ff' : '#2180e0');
  const pressed = isModern ? (isDark ? '#388bfd' : '#104890')
    : isTrading ? (isDark ? '#40b1c1' : '#055264')
    : (isDark ? '#388bfd' : '#085bb8');
  return {
    common: {
      primaryColor: accent,
      primaryColorHover: hover,
      primaryColorPressed: pressed,
      primaryColorSuppl: accent,
      infoColor: accent,
      infoColorHover: hover,
      infoColorPressed: pressed,
      infoColorSuppl: accent,
      borderColor: border,
      dividerColor: border,
      borderRadius: isModern ? '11px' : isTrading ? '4px' : '6px',
    },
  };
});

onMounted(async () => {
  try {
    if (!await settings.fetchSettings()) throw new Error(settings.error || '加载设置失败');
    await settings.initStockDbListener();
    settings.applyTheme(settings.theme);
    await watchlist.fetchWatchlist();
    await quote.startListening();
    initReady.value = true;

    // Track the A-share session so the settings dialog can show what the
    // "auto" refresh interval currently resolves to.
    void settings.fetchMarketSession();
    unlistenSession = await listen<MarketSessionInfo>('market-session-changed', (event) => {
      settings.marketSession = event.payload;
    });

    // Startup update check (non-blocking, gated by trading session)
    performStartupCheck();
  } catch (e) {
    initError.value = `应用启动失败: ${String(e).slice(0, 200)}`;
    console.error('[App] init failed:', e);
  }
});

onUnmounted(() => {
  quote.stopListening();
  settings.stopStockDbListener();
  if (unlistenSession) unlistenSession();
});

function handleRetry() {
  initError.value = null;
  location.reload();
}
</script>

<template>
  <NConfigProvider :theme="settings.theme === 'dark' ? darkTheme : lightTheme" :theme-overrides="themeOverrides">
    <NMessageProvider>
      <AlertNotifications />
      <AppLayout
        :init-error="initError"
        :init-ready="initReady"
        :quote-error="quote.error"
        :app-error="appError"
        @retry="handleRetry"
        @dismiss-app-error="appError = null"
      />
    </NMessageProvider>
    <UpdateDialog />
  </NConfigProvider>
</template>
