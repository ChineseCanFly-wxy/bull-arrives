<script setup lang="ts">
import { computed, ref, defineAsyncComponent, onMounted, onUnmounted } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useSettingsStore } from '@/stores/settings';
import { useRankStore } from '@/stores/rank';
import { NIcon, NDropdown, useMessage } from 'naive-ui';
const SettingsDialog = defineAsyncComponent(() => import('@/components/settings/SettingsDialog.vue'));
const UniverseScreenerDialog = defineAsyncComponent(() => import('@/components/screener/UniverseScreenerDialog.vue'));
const MonitorDialog = defineAsyncComponent(() => import('@/components/monitor/MonitorDialog.vue'));
const SectorDialog = defineAsyncComponent(() => import('@/components/sector/SectorDialog.vue'));
const SimulationDialog = defineAsyncComponent(() => import('@/components/simulation/SimulationDialog.vue'));
const ResearchCenter = defineAsyncComponent(() => import('@/components/research/ResearchCenter.vue'));
const showResearch = ref(false);
const RankDialog = defineAsyncComponent(() => import('@/components/rank/RankDialog.vue'));

const settings = useSettingsStore();
const rank = useRankStore();
const message = useMessage();
let unlistenQuitBlocked: UnlistenFn | null = null;
onMounted(async () => {
  unlistenQuitBlocked = await listen('stockdb-quit-blocked', () => {
    message.warning('数据更新正在进行，请等待更新窗口关闭后再退出应用');
  });
});
onUnmounted(() => unlistenQuitBlocked?.());
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
function openSettings() {
  showSettings.value = true;
}

const showScreener = ref(false);
function openScreener() {
  showScreener.value = true;
}

const showMonitor = ref(false);
function openMonitor() {
  showMonitor.value = true;
}

const showSector = ref(false);
function openSector() {
  showSector.value = true;
}

const showSimulation = ref(false);
function openSimulation() {
  showSimulation.value = true;
}
</script>

<template>
  <header class="top-bar">
    <div class="brand-block" aria-label="Bull Arrives 行情工作台">
      <span class="classic-slogan">实时行情 · 多源切换 · 免费高效</span>
      <span class="brand-mark" aria-hidden="true">↗</span>
      <span class="brand-name">Bull Arrives</span>
      <span class="brand-slogan">行情工作台</span>
    </div>

    <nav class="top-bar-right" aria-label="工作台导航">
      <button class="nav-primary" title="策略研究、自动选股与模拟验证" @click="showResearch=true">研究中心</button>
      <n-dropdown
        trigger="click"
        :options="dsOptions"
        @select="handleDsSelect"
      >
        <button class="ds-tag" type="button" :aria-label="`切换数据源，当前为${dsDisplayName}`" title="点击切换数据源">
          <span class="ds-label">{{ dsDisplayName }}</span>
          <n-icon :size="12" class="ds-swap-icon">
            <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M7 3 3 7l4 4" />
              <path d="M17 11v1a4 4 0 0 1-4 4H3" />
              <path d="M13 17 17 13l-4-4" />
              <path d="M3 9V8a4 4 0 0 1 4-4h10" />
            </svg>
          </n-icon>
        </button>
      </n-dropdown>

      <button
        class="cog-btn"
        aria-label="打开模拟账户"
        title="模拟账户"
        @click="openSimulation"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <path d="M3 15.5h14M4.5 13V8.5M8.2 13V5.5M11.8 13V9.5M15.5 13V3" />
        </svg>
      </button>

      <button
        class="cog-btn"
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
      </button>

      <button
        class="cog-btn"
        aria-label="打开持仓监控"
        title="持仓监控（止损/止盈）"
        @click="openMonitor"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10 3a7 7 0 0 1 7 7c0 2.4-1.2 4.5-3 5.7V18l-1.6-1.2L10 18l-2.4-1.2L6 18v-2.3A7 7 0 0 1 3 10a7 7 0 0 1 7-7Z" />
          <path d="M7.5 10.5l1.7 1.7 3.3-3.4" />
        </svg>
      </button>

      <button
        class="cog-btn"
        aria-label="打开全市场筛选器"
        title="全市场筛选器"
        @click="openScreener"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <path d="M2.5 4h15l-5.8 6.8V17l-3.4-2V10.8L2.5 4Z" />
        </svg>
      </button>

      <button
        v-if="settings.localHistoryEnabled && settings.stockDbStatus?.platformSupported !== false"
        class="cog-btn"
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
      </button>

      <button
        class="cog-btn"
        :aria-label="`打开设置 (${settings.tickerHotkey})`"
        :title="`设置 (悬浮窗快捷键: ${settings.tickerHotkey})`"
        @click="openSettings"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="10" cy="10" r="2.4" />
          <path d="M16.4 12.4l1.3 1.3a1 1 0 0 1 0 1.4l-1.1 1.1a1 1 0 0 1-1.4 0l-1.3-1.3a6.5 6.5 0 0 1-1.6.7l-.2 1.7a1 1 0 0 1-1 .9h-2.2a1 1 0 0 1-1-.9l-.2-1.7a6.5 6.5 0 0 1-1.6-.7L5 15.6a1 1 0 0 1-1.4 0L2.5 14.5a1 1 0 0 1 0-1.4l1.3-1.3a6.5 6.5 0 0 1-.7-1.6L1.4 10a1 1 0 0 1-.9-1V6.8a1 1 0 0 1 .9-1l1.7-.2a6.5 6.5 0 0 1 .7-1.6L2.5 2.7a1 1 0 0 1 0-1.4L3.6.2a1 1 0 0 1 1.4 0L6.3 1.5a6.5 6.5 0 0 1 1.6-.7l.2-1.7a1 1 0 0 1 1-.9h2.2a1 1 0 0 1 1 .9l.2 1.7a6.5 6.5 0 0 1 1.6.7l1.3-1.3a1 1 0 0 1 1.4 0l1.1 1.1a1 1 0 0 1 0 1.4l-1.3 1.3a6.5 6.5 0 0 1 .7 1.6l1.7.2a1 1 0 0 1 .9 1V9a1 1 0 0 1-.9 1l-1.7.2a6.5 6.5 0 0 1-.7 1.6Z" />
        </svg>
      </button>
    </nav>

    <SettingsDialog v-if="showSettings" v-model:show="showSettings" />
    <UniverseScreenerDialog v-if="showScreener" v-model:show="showScreener" />
    <MonitorDialog v-if="showMonitor" v-model:show="showMonitor" />
    <SectorDialog v-if="showSector" v-model:show="showSector" />
    <SimulationDialog v-if="showSimulation" v-model:show="showSimulation" />
    <ResearchCenter v-if="showResearch" v-model:show="showResearch" />
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
:global([data-style="classic"]) .top-bar { gap: 0; padding: 0 var(--space-4); }
:global([data-style="classic"]) .brand-block { gap: 0; }
:global([data-style="classic"]) .classic-slogan { display: inline; font-size: var(--text-xs); color: var(--color-text-tertiary); letter-spacing: .02em; }
:global([data-style="classic"]) .brand-mark,
:global([data-style="classic"]) .brand-name,
:global([data-style="classic"]) .brand-slogan { display: none; }
:global([data-style="classic"]) .top-bar-right { overflow: visible; }
:global([data-style="classic"]) .nav-primary { height: 22px; padding: 0 10px; border-color: transparent; background: transparent; color: var(--color-text-tertiary); font: 400 12px var(--font-sans); }
:global([data-style="classic"]) .nav-primary:hover { color: var(--color-accent); background: var(--color-accent-dim); }
:global([data-style="classic"]) .cog-btn { width: 22px; height: 22px; }
:global([data-style="classic"]) .ds-tag { height: 20px; border-color: transparent; }
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
  justify-content: flex-end;
  flex: 0 1 auto;
  min-width: 0;
  gap: var(--space-2);
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: thin;
  -webkit-app-region: no-drag;
}
.top-bar-right > * { flex: 0 0 auto; }
.nav-primary {
  height: var(--control-height);
  padding: 0 var(--space-3);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-sm);
  background: var(--color-accent-dim);
  color: var(--color-accent);
  font: 600 var(--text-sm) var(--font-sans);
  white-space: nowrap;
  cursor: pointer;
}
.nav-primary:hover { background: var(--color-surface-hover); }
@media (max-width: 780px) {
  .brand-slogan { display: none; }
  .top-bar { gap: var(--space-2); }
  :global([data-style="classic"]) .top-bar-right { overflow-x: auto; overflow-y: hidden; }
}
@media (max-width: 580px) {
  .brand-name { display: none; }
}

/* ── Data source tag ── */
.ds-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: var(--control-height);
  padding: 0 var(--space-2);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-sm);
  background: var(--color-accent-dim);
  color: var(--color-accent);
  font-size: var(--text-xs);
  cursor: pointer;
  user-select: none;
  transition: background var(--transition-fast), color var(--transition-fast);
  -webkit-app-region: no-drag;
}
.ds-tag:hover {
  filter: brightness(1.4);
}
.ds-label {
  line-height: 1;
}
.ds-swap-icon {
  color: inherit;
  flex-shrink: 0;
}

/* ── Settings cog button ── */
.cog-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: var(--control-height);
  height: var(--control-height);
  flex: 0 0 auto;
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-tertiary);
  cursor: pointer;
  transition: color var(--transition-fast), background var(--transition-fast), border-color var(--transition-fast);
}
.cog-btn:hover:not(:disabled) {
  color: var(--color-accent);
  background: var(--color-accent-dim);
  border-color: var(--color-accent-dim);
}
.cog-btn:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}
.cog-btn.spinning svg {
  animation: stockdb-spin 0.9s linear infinite;
}
@keyframes stockdb-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) {
  .cog-btn.spinning svg { animation: none; }
}
</style>
