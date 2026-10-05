<script setup lang="ts">
import { computed, ref, defineAsyncComponent, onMounted, onUnmounted } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '@/stores/settings';
import { useRankStore } from '@/stores/rank';
import { NIcon, NDropdown, useMessage } from 'naive-ui';
import type { MainlineNavigationTarget } from '@/types/navigation';
const SettingsDialog = defineAsyncComponent(() => import('@/components/settings/SettingsDialog.vue'));
const UniverseScreenerDialog = defineAsyncComponent(() => import('@/components/screener/UniverseScreenerDialog.vue'));
const MonitorDialog = defineAsyncComponent(() => import('@/components/monitor/MonitorDialog.vue'));
const SectorDialog = defineAsyncComponent(() => import('@/components/sector/SectorDialog.vue'));
const MarketMainlineDialog = defineAsyncComponent(() => import('@/components/sector/MarketMainlineDialog.vue'));
const showMainline = ref(false);
const mainlineTarget = ref<MainlineNavigationTarget | null>(null);
const SimulationDialog = defineAsyncComponent(() => import('@/components/simulation/SimulationDialog.vue'));
const ResearchCenter = defineAsyncComponent(() => import('@/components/research/ResearchCenter.vue'));
const showResearch = ref(false);
const RankDialog = defineAsyncComponent(() => import('@/components/rank/RankDialog.vue'));

const settings = useSettingsStore();
const rank = useRankStore();
const message = useMessage();
let unlistenQuitBlocked: UnlistenFn | null = null;
let unlistenNavigation: UnlistenFn | null = null;
onMounted(async () => {
  unlistenQuitBlocked = await listen('stockdb-quit-blocked', () => {
    message.warning('数据更新正在进行，请等待更新窗口关闭后再退出应用');
  });
  unlistenNavigation = await listen('quick-navigation', () => { void consumeNavigation(); });
  await consumeNavigation();
});
onUnmounted(() => { unlistenQuitBlocked?.(); unlistenNavigation?.(); });
const stockDbUpdating = computed(() =>
  ['updating', 'restarting'].includes(settings.stockDbStatus?.state || '')
);
const stockDbUpdateTitle = computed(() => {
  if (!settings.stockDbStatus?.updaterAvailable) return '请先在设置中选择“数据更新.exe”';
  return stockDbUpdating.value ? settings.stockDbStatus?.message || '正在更新本地数据' : '更新本地 stockdb 数据';
});

async function updateStockDb() {
  try {
    const result = await settings.runStockDbUpdate();
    message.success(result);
  } catch (error) {
    message.error(String(error), { duration: 6000 });
  }
}

const dsDisplayName = computed(() => {
  const found = settings.datasources.find(([id]) => id === settings.activeDatasource);
  return found ? found[1] : settings.activeDatasource;
});

const dsOptions = computed(() =>
  settings.datasources.map(([id, name]) => ({
    label: name,
    key: id,
  }))
);

function handleDsSelect(key: string) {
  settings.switchDatasource(key);
}

const showSettings = ref(false);
const settingsDialog = ref<{ close(): boolean } | null>(null);
const settingsSection = ref<string | undefined>();
function openSettings(section?: string) {
  settingsSection.value = typeof section === 'string' ? section : undefined;
  showSettings.value = true;
}
const quickMenu = ref(false);
const quickX = ref(0);
const quickY = ref(0);
const quickOptions = [
  { label: '研究中心', key: 'research' },
  { label: '市场主线', key: 'mainline' },
  { label: '市场筛选', key: 'screener' },
  { label: '模拟账户', key: 'simulation' },
  { type: 'divider' as const, key: 'divider' },
  { label: '设置', key: 'settings' },
];
function openQuickMenu(event: MouseEvent) {
  quickX.value = event.clientX;
  quickY.value = event.clientY;
  quickMenu.value = true;
}
function navigate(destination: string) {
  quickMenu.value = false;
  const toSettings = destination === 'settings' || destination.startsWith('settings:');
  // Keep the current settings instance when changing sections: drafts survive.
  if (showSettings.value && !toSettings && !settingsDialog.value?.close()) return false;
  showResearch.value = showScreener.value = showMonitor.value = showSector.value = showSimulation.value = showMainline.value = false;
  if (toSettings) openSettings(destination === 'settings' ? undefined : destination.slice(9));
  else {
    showSettings.value = false;
    if (destination === 'research') showResearch.value = true;
    else if (destination === 'mainline') { mainlineTarget.value = null; showMainline.value = true; }
    else if (destination === 'sector') showSector.value = true;
    else if (destination === 'monitor') showMonitor.value = true;
    else if (destination === 'screener') showScreener.value = true;
    else if (destination === 'simulation') showSimulation.value = true;
  }
  return true;
}
function openMainline(target?: MainlineNavigationTarget) {
  if (navigate('mainline')) mainlineTarget.value = target ? { ...target } : null;
}
async function consumeNavigation() {
  try {
    const destination = await invoke<string | null>('take_pending_navigation');
    if (destination) navigate(destination);
  } catch (error) {
    message.error('打开快捷入口失败：' + String(error));
  }
}

const showScreener = ref(false);
function openScreener() {
  navigate('screener');
}

const showMonitor = ref(false);
function openMonitor() {
  navigate('monitor');
}

const showSector = ref(false);
function openSector() {
  navigate('sector');
}

const showSimulation = ref(false);
function openSimulation() {
  navigate('simulation');
}
</script>

<template>
  <header class="top-bar">
    <div class="brand-block" aria-label="Bull Arrives 行情工作台">
      <span class="classic-slogan">Bull Arrives · 自选行情</span>
      <span class="brand-mark" aria-hidden="true">↗</span>
      <span class="brand-name">Bull Arrives</span>
      <span class="brand-slogan">行情工作台</span>
    </div>

    <nav class="top-bar-right" aria-label="工作台导航" @contextmenu.prevent="openQuickMenu">
      <n-dropdown
        trigger="click"
        :options="dsOptions"
        @select="handleDsSelect"
      >
        <button class="ds-tag" type="button" :aria-label="`切换数据源，当前为${dsDisplayName}`" title="点击切换数据源">
          <span class="ds-label">{{ dsDisplayName }}</span>
          <n-icon :size="14" class="ds-swap-icon">
            <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M7 3 3 7l4 4" />
              <path d="M17 11v1a4 4 0 0 1-4 4H3" />
              <path d="M13 17 17 13l-4-4" />
              <path d="M3 9V8a4 4 0 0 1 4-4h10" />
            </svg>
          </n-icon>
        </button>
      </n-dropdown>

      <button class="nav-primary" title="策略研究、选股与模拟验证；右键打开快捷菜单" @click="navigate('research')"><svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M7 2h6M8 2v6l-5 8a1 1 0 0 0 1 2h12a1 1 0 0 0 1-2l-5-8V2M6 13h8" /></svg><span>研究中心</span></button>

      <button class="nav-primary" aria-label="打开市场主线" title="查看全市场主线候选、领涨观察与盘后提醒" @click="openMainline()"><svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M2 16h16M3 13l4-5 4 3 6-8M13 3h4v4" /></svg><span>市场主线</span></button>

      <button
        class="nav-primary"
        aria-label="打开模拟账户"
        title="模拟账户"
        @click="openSimulation"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <path d="M3 15.5h14M4.5 13V8.5M8.2 13V5.5M11.8 13V9.5M15.5 13V3" />
        </svg>
      <span>模拟账户</span>
      </button>

      <button
        class="nav-primary"
        aria-label="打开市场板块"
        title="行业/概念板块"
        @click="openSector"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2.5" y="2.5" width="6" height="6" rx="1" />
          <rect x="11.5" y="2.5" width="6" height="6" rx="1" />
          <rect x="2.5" y="11.5" width="6" height="6" rx="1" />
          <rect x="11.5" y="11.5" width="6" height="6" rx="1" />
        </svg>
      <span>市场板块</span>
      </button>

      <button
        class="nav-primary"
        aria-label="打开持仓监控"
        title="持仓监控（止损/止盈）"
        @click="openMonitor"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10 3a7 7 0 0 1 7 7c0 2.4-1.2 4.5-3 5.7V18l-1.6-1.2L10 18l-2.4-1.2L6 18v-2.3A7 7 0 0 1 3 10a7 7 0 0 1 7-7Z" />
          <path d="M7.5 10.5l1.7 1.7 3.3-3.4" />
        </svg>
      <span>持仓监控</span>
      </button>

      <button
        class="nav-primary"
        aria-label="打开全市场筛选器"
        title="全市场筛选器"
        @click="openScreener"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <path d="M2.5 4h15l-5.8 6.8V17l-3.4-2V10.8L2.5 4Z" />
        </svg>
      <span>市场筛选</span>
      </button>

      <button
        v-if="settings.localHistoryEnabled && settings.stockDbStatus?.platformSupported !== false"
        class="nav-primary"
        :class="{ spinning: stockDbUpdating }"
        aria-label="更新本地 stockdb 数据"
        :title="stockDbUpdateTitle"
        :disabled="stockDbUpdating || settings.stockDbStatus?.busy || !settings.stockDbStatus?.updaterAvailable"
        @click="updateStockDb"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <ellipse cx="10" cy="4" rx="6" ry="2.5" />
          <path d="M4 4v4c0 1.4 2.7 2.5 6 2.5M16 4v3" />
          <path d="M4 8v4c0 1.4 2.7 2.5 6 2.5" />
          <path d="M13 11.5h4v4" />
          <path d="M17 11.5a5 5 0 0 1-7 5" />
        </svg>
      <span>更新数据</span>
      </button>

      <button
        class="nav-primary"
        :aria-label="`打开设置 (${settings.tickerHotkey})`"
        :title="`设置 (悬浮窗快捷键: ${settings.tickerHotkey})`"
        @click="navigate('settings')"
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" aria-hidden="true">
          <circle cx="12" cy="12" r="7" />
          <circle cx="12" cy="12" r="2.7" />
          <path d="M12 2v3m0 14v3M2 12h3m14 0h3M4.9 4.9 7 7m10 10 2.1 2.1M4.9 19.1 7 17m10-10 2.1-2.1" />
        </svg>
      <span>设置</span>
      </button>
    </nav>

    <NDropdown trigger="manual" :show="quickMenu" :x="quickX" :y="quickY" :options="quickOptions" @select="navigate" @clickoutside="quickMenu = false" />
    <SettingsDialog ref="settingsDialog" v-if="showSettings" v-model:show="showSettings" :initial-section="settingsSection" />
    <UniverseScreenerDialog v-if="showScreener" v-model:show="showScreener" />
    <MonitorDialog v-if="showMonitor" v-model:show="showMonitor" />
    <SectorDialog v-if="showSector" v-model:show="showSector" />
    <MarketMainlineDialog v-if="showMainline" v-model:show="showMainline" :initial-target="mainlineTarget" />
    <SimulationDialog v-if="showSimulation" v-model:show="showSimulation" />
    <ResearchCenter v-if="showResearch" v-model:show="showResearch" @open-settings="navigate('settings:' + $event)" />
    <RankDialog
      v-if="rank.visible && rank.activeFilter"
      :show="rank.visible"
      :filter="rank.activeFilter"
      @update:show="rank.setVisible"
    />
  </header>
</template>

<style scoped>
.top-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  height: var(--header-height);
  min-height: var(--header-height);
  padding: 0 var(--workspace-gutter);
  background: var(--color-surface-1);
  border-bottom: 1px solid var(--color-border-0);
  -webkit-app-region: drag;
}
.classic-slogan { display: none; }
html[data-style="classic"] .top-bar { gap: 0; padding: 0 var(--space-4); }
html[data-style="classic"] .brand-block { gap: 0; }
html[data-style="classic"] .classic-slogan { display: inline; font-size: var(--text-xs); color: var(--color-text-tertiary); letter-spacing: .02em; }
html[data-style="classic"] .brand-mark { display: none; }
html[data-style="classic"] .brand-name { display: none; }
html[data-style="classic"] .brand-slogan { display: none; }
html[data-style="classic"] .nav-primary { height: 22px; padding: 0 10px; border-color: transparent; background: transparent; color: var(--color-text-tertiary); font: 400 12px var(--font-sans); }
html[data-style="classic"] .nav-primary:hover { color: var(--color-accent); background: var(--color-accent-dim); }

html[data-style="classic"] .ds-tag { height: 20px; padding: 0 var(--space-2); gap: 4px; border-color: transparent; background: var(--color-accent-dim); color: var(--color-accent); font: 400 var(--text-xs) var(--font-sans); }
.brand-block {
  display: flex;
  align-items: center;
  flex: 0 0 auto;
  min-width: 0;
  gap: var(--space-2);
  white-space: nowrap;
}
.brand-mark {
  display: grid;
  place-items: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-sm);
  background: var(--color-accent);
  color: var(--color-accent-contrast);
  font-weight: 700;
}
.brand-name {
  font-size: var(--text-md);
  font-weight: var(--font-weight-bold);
  letter-spacing: -.02em;
}
.brand-slogan {
  padding-left: var(--space-2);
  border-left: 1px solid var(--color-border-1);
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
}
.top-bar-right {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  margin-left: auto;
  max-width: 100%;
  flex: 0 1 auto;
  min-width: 0;
  gap: var(--space-2);
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: thin;
  -webkit-app-region: no-drag;
}
.top-bar-right > * { flex: 0 0 auto; }
.nav-primary,
.ds-tag {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: var(--control-height);
  padding: 0 var(--space-3);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-sm);
  background: var(--color-surface-1);
  color: var(--color-text-secondary);
  font: 600 var(--text-sm) var(--font-sans);
  white-space: nowrap;
  cursor: pointer;
}
.nav-primary:hover:not(:disabled),
.ds-tag:hover { background: var(--color-surface-hover); color: var(--color-accent); }
.nav-primary:disabled { opacity: .45; cursor: not-allowed; }
.nav-primary svg { flex-shrink: 0; }
@media (max-width: 780px) {
  .brand-slogan { display: none; }
  .top-bar { gap: var(--space-2); }
}
@media (max-width: 580px) {
  .brand-name, .classic-slogan { display: none !important; }
}

.ds-tag {
  user-select: none;
  -webkit-app-region: no-drag;
}
.ds-swap-icon {
  color: inherit;
  flex-shrink: 0;
}

html[data-appearance="elegant"] .nav-primary,
html[data-appearance="elegant"] .ds-tag {
  height: 36px;
  padding-inline: var(--space-3);
  gap: var(--space-2);
  border-radius: var(--radius-full);
  font: 500 14px var(--font-sans);
}
html[data-appearance="elegant"] .nav-primary svg,
html[data-appearance="elegant"] .ds-swap-icon { width: 16px; height: 16px; }

.nav-primary.spinning svg {
  animation: stockdb-spin 0.9s linear infinite;
}
@keyframes stockdb-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) {
  .nav-primary.spinning svg { animation: none; }
}
</style>
