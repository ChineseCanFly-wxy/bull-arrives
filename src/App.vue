<script setup lang="ts">
import { onMounted, onUnmounted, ref, computed, onErrorCaptured } from 'vue';
import { NConfigProvider, darkTheme, lightTheme, NMessageProvider, type GlobalThemeOverrides } from 'naive-ui';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useSettingsStore, SETTING_CHANGED_EVENT, type SettingChangedPayload, type MarketSessionInfo } from '@/stores/settings';
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
let unlistenSettings: UnlistenFn | null = null;
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
  const isModern = settings.visualStyle !== 'classic';
  if (settings.visualStyle === 'elegant') {
    const tokens = getComputedStyle(document.documentElement);
    const color = (name: string) => tokens.getPropertyValue('--color-' + name).trim();
    const accent = color('accent');
    return {
      common: {
        fontFamily: 'var(--font-sans)', fontFamilyMono: 'var(--font-numeric)',
        fontSize: '14px', fontSizeSmall: '13px', fontSizeMedium: '14px', fontSizeLarge: '16px',
        fontWeightStrong: '600', lineHeight: '1.7', heightSmall: '32px', heightMedium: '36px',
        primaryColor: accent, primaryColorHover: color('focus-ring'), primaryColorPressed: accent,
        primaryColorSuppl: accent, infoColor: accent, infoColorHover: color('focus-ring'),
        infoColorPressed: accent, infoColorSuppl: accent,
        textColor1: color('text-primary'), textColor2: color('text-primary'), textColor3: color('text-secondary'),
        placeholderColor: color('text-tertiary'), bodyColor: color('surface-0'),
        cardColor: color('surface-1'), modalColor: color('surface-1'), popoverColor: color('surface-1'),
        inputColor: color('surface-1'), tableColor: color('surface-1'), tableHeaderColor: color('surface-2'),
        tableColorHover: color('surface-hover'), hoverColor: color('surface-hover'),
        borderColor: color('border-0'), dividerColor: color('border-0'), borderRadius: '6px', borderRadiusSmall: '4px',
      },
      Card: {
        paddingSmall: '20px 24px', paddingMedium: '24px 28px',
        titleFontSizeSmall: '20px', titleFontSizeMedium: '20px', titleFontWeight: '600', borderRadius: '8px',
      },
      DataTable: {
        thPaddingSmall: '12px 16px', tdPaddingSmall: '10px 16px',
        thPaddingMedium: '12px 16px', tdPaddingMedium: '10px 16px',
        thColor: color('surface-2'), tdColor: color('surface-1'), tdColorHover: color('surface-hover'),
        thTextColor: color('text-secondary'), tdTextColor: color('text-primary'), borderColor: color('border-0'),
      },
    };
  }
  const accent = isModern ? (isDark ? '#81bcff' : '#1659b7')
    : (isDark ? '#58a6ff' : '#0969da');
  const border = isModern ? (isDark ? '#314159' : '#d8e1ed')
    : (isDark ? '#1e293b' : '#d0d7de');
  const hover = isModern ? (isDark ? '#acd4ff' : '#3276cf')
    : (isDark ? '#79b8ff' : '#2180e0');
  const pressed = isModern ? (isDark ? '#388bfd' : '#104890')
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
      borderRadius: isModern ? '11px' : '6px',
    },
  };
});

onMounted(async () => {
  try {
    unlistenSettings = await listen<SettingChangedPayload>(SETTING_CHANGED_EVENT, ({ payload }) => {
      settings.applyRemoteSetting(payload.key, payload.value);
    });
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
  unlistenSettings?.();
});

function handleRetry() {
  initError.value = null;
  location.reload();
}
</script>

<template>
  <NConfigProvider :theme="settings.theme === 'dark' ? darkTheme : lightTheme" :theme-overrides="themeOverrides" :style="settings.visualStyle === 'elegant' ? { fontFamily: 'var(--font-sans)', lineHeight: 'var(--line-height-body)' } : undefined">
    <NMessageProvider>
      <AlertNotifications v-if="initReady" />
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
