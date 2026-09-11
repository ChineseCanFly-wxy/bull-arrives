<script setup lang="ts">
import { computed, ref, h, inject, onBeforeUnmount, onMounted, watch, defineAsyncComponent } from 'vue';
import { NButton, NDataTable, NDropdown, NModal } from 'naive-ui';
import type { DataTableColumns } from 'naive-ui';
import { invoke } from '@tauri-apps/api/core';
import { useWatchlistStore } from '@/stores/watchlist';
import { useQuoteStore } from '@/stores/quote';
import type { WatchItem } from '@/types';
import { formatPrice, formatVolume, formatCode, cnCategory } from '@/utils/format';
import AddStockDialog from './AddStockDialog.vue';
const PriceAlertDialog = defineAsyncComponent(() => import('./PriceAlertDialog.vue'));
import GroupToolbar from './GroupToolbar.vue';
const HoldingDialog = defineAsyncComponent(() => import('./HoldingDialog.vue'));
import { calculateHoldingMetrics, scaledPriceToDecimal, type Holding } from '@/utils/holdings';
import MarketTag from './MarketTag.vue';
import StockDetail from '@/components/detail/StockDetail.vue';
import { CLEAR_INDEX_DETAIL_KEY } from '@/utils/keys';

const watchlist = useWatchlistStore();
const quoteStore = useQuoteStore();
const showAddDialog = ref(false);
const showAlertDialog = ref(false);
const showHoldingDialog = ref(false);
const holdingItem = ref<WatchItem | null>(null);
const holdings = ref(new Map<number, Holding>());
async function loadHoldings() {
  try {
    const values = await invoke<Holding[]>('get_holdings');
    holdings.value = new Map(values.map(value => [value.watch_id, value]));
  } catch (error) { console.error('持仓读取失败', error); }
}
function openHolding(row: WatchItem) {
  cancelPendingRowClick();
  holdingItem.value = row;
  showHoldingDialog.value = true;
}
function holdingMetric(row: WatchItem) {
  const holding = holdings.value.get(row.id);
  const quote = quoteStore.getQuote(row.code, row.market);
  if (!holding || !quote || quote.price <= 0) return null;
  try { return calculateHoldingMetrics(holding, quote.price); }
  catch { return null; }
}

const alertDialogItem = ref<WatchItem | null>(null);
const showDeleteConfirm = ref(false);
const deleting = ref(false);
let rowClickTimer: ReturnType<typeof setTimeout> | null = null;

watch(() => watchlist.activeGroupId, () => {
  cancelPendingRowClick();
  selectedRow.value = null;
  ctxMenuItem.value = null;
  showCtxMenu.value = false;
  showDeleteConfirm.value = false;
});

const indexDetailCoord = inject<{
  clearIndexDetail: () => void;
  registerClearStockFn?: (fn: () => void) => void;
} | undefined>(CLEAR_INDEX_DETAIL_KEY);

onMounted(() => {
  void loadHoldings();
  indexDetailCoord?.registerClearStockFn?.(() => {
    selectedRow.value = null;
  });
});

// Context menu state
const ctxMenuX = ref(0);
const ctxMenuY = ref(0);
const ctxMenuItem = ref<WatchItem | null>(null);
const showCtxMenu = ref(false);

// Detail panel state
const selectedRow = ref<WatchItem | null>(null);

function cancelPendingRowClick() {
  if (rowClickTimer) {
    clearTimeout(rowClickTimer);
    rowClickTimer = null;
  }
}

function openAlertSettings(row: WatchItem) {
  cancelPendingRowClick();
  showCtxMenu.value = false;
  alertDialogItem.value = row;
  showAlertDialog.value = true;
}

function handleRowClick(row: WatchItem) {
  cancelPendingRowClick();
  // Defer the detail action until the double-click window has elapsed. This
  // prevents a brief detail-panel flash before the alert dialog opens.
  rowClickTimer = setTimeout(() => {
    rowClickTimer = null;
    if (selectedRow.value?.id === row.id) {
      selectedRow.value = null;
    } else {
      indexDetailCoord?.clearIndexDetail();
      selectedRow.value = row;
    }
  }, 220);
}

function handleRowKeydown(event: KeyboardEvent, row: WatchItem) {
  if (event.key === 'Enter' || event.key.toLowerCase() === 'a') {
    event.preventDefault();
    openAlertSettings(row);
  } else if (event.key === ' ') {
    event.preventDefault();
    handleRowClick(row);
  }
}

onBeforeUnmount(cancelPendingRowClick);

function handleContextMenu(e: MouseEvent, row: WatchItem) {
  e.preventDefault();
  // Clamp menu position to viewport so it never renders off-screen
  const menuW = 140; // approximate menu width
  const menuH = 200; // approximate menu height
  ctxMenuX.value = Math.min(e.clientX, window.innerWidth - menuW);
  ctxMenuY.value = Math.min(e.clientY, window.innerHeight - menuH);
  ctxMenuItem.value = row;
  showCtxMenu.value = true;
}

function requestDelete() {
  showCtxMenu.value = false;
  if (!ctxMenuItem.value) return;
  if (watchlist.activeGroupId === 0) {
    showDeleteConfirm.value = true;
  } else {
    void confirmDelete();
  }
}

async function confirmDelete() {
  const item = ctxMenuItem.value;
  if (!item || deleting.value) return;
  deleting.value = true;
  try {
    await watchlist.removeStock(item.code, item.market);
    selectedRow.value = null;
    showDeleteConfirm.value = false;
  } catch (e) {
    console.error('removeStock failed:', e);
  } finally {
    deleting.value = false;
  }
}

async function handleMoveTop() {
  if (!ctxMenuItem.value) return;
  try {
    await invoke('move_watch_top', { id: ctxMenuItem.value.id });
    await watchlist.fetchWatchlist();
  } catch (e) {
    console.error('move_watch_top failed:', e);
  }
  showCtxMenu.value = false;
}

async function handleMoveUp() {
  if (!ctxMenuItem.value) return;
  try {
    await invoke('move_watch_up', { id: ctxMenuItem.value.id });
    await watchlist.fetchWatchlist();
  } catch (e) {
    console.error('move_watch_up failed:', e);
  }
  showCtxMenu.value = false;
}

async function handleMoveDown() {
  if (!ctxMenuItem.value) return;
  try {
    await invoke('move_watch_down', { id: ctxMenuItem.value.id });
    await watchlist.fetchWatchlist();
  } catch (e) {
    console.error('move_watch_down failed:', e);
  }
  showCtxMenu.value = false;
}

const iconTop = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 2, style: 'vertical-align:middle;margin-right:6px' }, [
  h('path', { d: 'M8 2V14' }),
  h('polyline', { points: '4 6 8 2 12 6' }),
  h('line', { x1: 2, y1: 14, x2: 14, y2: 14 }),
]);
const iconUp = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 2, style: 'vertical-align:middle;margin-right:6px' }, [
  h('polyline', { points: '4 9 8 5 12 9' }),
]);
const iconDown = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 2, style: 'vertical-align:middle;margin-right:6px' }, [
  h('polyline', { points: '4 5 8 9 12 5' }),
]);
const iconAlert = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 1.5, style: 'vertical-align:middle;margin-right:6px' }, [
  h('path', { d: 'M3.5 11.5h9l-1.2-1.8V6.8A3.3 3.3 0 008 3.5 3.3 3.3 0 004.7 6.8v2.9z' }),
  h('path', { d: 'M6.7 12.5a1.35 1.35 0 002.6 0' }),
]);
const iconDelete = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: '#f85149', strokeWidth: 1.5, style: 'vertical-align:middle;margin-right:6px' }, [
  h('path', { d: 'M3 4h10' }),
  h('path', { d: 'M5 4V3a1 1 0 011-1h4a1 1 0 011 1v1' }),
  h('path', { d: 'M6 7v4' }),
  h('path', { d: 'M10 7v4' }),
  h('path', { d: 'M4 4l1 9h6l1-9' }),
]);

const ctxOptions = computed(() => {
  const grouped = watchlist.activeGroupId !== 0;
  return [
    { label: '设置行情提醒', key: 'alert', icon: iconAlert },
    { type: 'divider' as const, key: 'd0' },
    { label: '置顶', key: 'top', icon: iconTop },
    { label: '上移', key: 'up', icon: iconUp },
    { label: '下移', key: 'down', icon: iconDown },
    { type: 'divider' as const, key: 'd1' },
    {
      label: grouped ? '从当前分组移出' : '从全部自选删除',
      key: 'delete',
      icon: iconDelete,
    },
  ];
});

function handleCtxSelect(key: string) {
  switch (key) {
    case 'alert': if (ctxMenuItem.value) openAlertSettings(ctxMenuItem.value); break;
    case 'top': void handleMoveTop(); break;
    case 'up': void handleMoveUp(); break;
    case 'down': void handleMoveDown(); break;
    case 'delete': requestDelete(); break;
  }
}

const columns: DataTableColumns<WatchItem> = [
  {
    title: '代码', key: 'code', width: 72,
    render(row) {
      return h('span', { class: 'code-text' }, formatCode(row.code));
    }
  },
  {
    title: '名称', key: 'name', width: 168,
    sorter: (a: WatchItem, b: WatchItem) => a.name.localeCompare(b.name),
    render(row) {
      return h('div', { class: 'name-cell' }, [
        h(MarketTag, { code: row.code, category: cnCategory(row.code) }),
        h('span', { class: 'name-text' }, row.name),
      ]);
    }
  },
  {
    title: '最新价', key: 'price', width: 100,
    sorter: (a: WatchItem, b: WatchItem) => {
      const qa = quoteStore.getQuote(a.code, a.market);
      const qb = quoteStore.getQuote(b.code, b.market);
      return (qa?.price ?? 0) - (qb?.price ?? 0);
    },
    render(row) {
      const q = quoteStore.getQuote(row.code, row.market);
      if (!q) return '--';
      const v = q.change_pct;
      return h('span', { class: `pct-col ${v >= 0 ? 'up' : 'down'}` },
        formatPrice(q.price));
    }
  },
  {
    title: '涨跌幅', key: 'change_pct', width: 100,
    sorter: (a: WatchItem, b: WatchItem) => {
      const qa = quoteStore.getQuote(a.code, a.market);
      const qb = quoteStore.getQuote(b.code, b.market);
      return (qa?.change_pct ?? 0) - (qb?.change_pct ?? 0);
    },
    render(row) {
      const q = quoteStore.getQuote(row.code, row.market);
      if (!q) return '--';
      const v = q.change_pct;
      return h('span', { class: `pct-col ${v >= 0 ? 'up' : 'down'}` },
        `${v >= 0 ? '+' : ''}${v.toFixed(2)}%`);
    }
  },
  {
    title: '成本价', key: 'cost_price', width: 95,
    render(row) {
      const holding = holdings.value.get(row.id);
      const text = holding?.cost_price != null ? scaledPriceToDecimal(holding.cost_price) : '设置持仓';
      return h(NButton, { text: true, size: 'small', onClick: (event: MouseEvent) => { event.stopPropagation(); openHolding(row); } }, () => text);
    }
  },
  { title: '持股数', key: 'shares', width: 90, render: row => holdings.value.get(row.id)?.shares.toLocaleString() ?? '--' },
  { title: '持仓市值', key: 'market_value', width: 120, render: row => holdingMetric(row)?.marketValue ?? '--' },
  { title: '浮动盈亏', key: 'profit', width: 120, render(row) {
      const value = holdingMetric(row)?.profit;
      if (value == null) return '--';
      return h('span', { class: `pct-col ${value.startsWith('-') ? 'down' : value === '0.00' ? '' : 'up'}` }, value.startsWith('-') || value === '0.00' ? value : `+${value}`);
    }
  },
  { title: '盈亏比例', key: 'profit_rate', width: 100, render: row => holdingMetric(row)?.profitRate ?? '--' },
  {
    title: '涨跌额', key: 'change', width: 90,
    sorter: (a: WatchItem, b: WatchItem) => {
      const qa = quoteStore.getQuote(a.code, a.market);
      const qb = quoteStore.getQuote(b.code, b.market);
      return (qa?.change ?? 0) - (qb?.change ?? 0);
    },
    render(row) {
      const q = quoteStore.getQuote(row.code, row.market);
      if (!q) return '--';
      const v = q.change;
      return h('span', { class: `pct-col ${v >= 0 ? 'up' : 'down'}` },
        `${v >= 0 ? '+' : ''}${formatPrice(v)}`);
    }
  },
  {
    title: '成交量', key: 'volume', width: 90,
    sorter: (a: WatchItem, b: WatchItem) => {
      const qa = quoteStore.getQuote(a.code, a.market);
      const qb = quoteStore.getQuote(b.code, b.market);
      return (qa?.volume ?? 0) - (qb?.volume ?? 0);
    },
    render(row) {
      const q = quoteStore.getQuote(row.code, row.market);
      if (!q || q.volume == null) return '--';
      return h('span', formatVolume(q.volume));
    }
  },
  {
    title: '成交额', key: 'turnover', width: 90,
    sorter: (a: WatchItem, b: WatchItem) => {
      const qa = quoteStore.getQuote(a.code, a.market);
      const qb = quoteStore.getQuote(b.code, b.market);
      return (qa?.turnover ?? 0) - (qb?.turnover ?? 0);
    },
    render(row) {
      const q = quoteStore.getQuote(row.code, row.market);
      if (!q || q.turnover == null) return '--';
      // turnover is in 元; display in 万元 or 亿元
      const wan = q.turnover / 10000;
      if (wan >= 10000) return h('span', `${(wan / 10000).toFixed(2)}亿`);
      if (wan > 0) return h('span', `${wan.toFixed(2)}万`);
      return h('span', '0');
    }
  },
  {
    title: '换手率', key: 'turnover_rate', width: 80,
    sorter: (a: WatchItem, b: WatchItem) => {
      const qa = quoteStore.getQuote(a.code, a.market);
      const qb = quoteStore.getQuote(b.code, b.market);
      return (qa?.turnover_rate ?? 0) - (qb?.turnover_rate ?? 0);
    },
    render(row) {
      const q = quoteStore.getQuote(row.code, row.market);
      if (!q || q.turnover_rate == null) return '--';
      return h('span', `${q.turnover_rate.toFixed(2)}%`);
    }
  },
];

defineExpose({ clearSelection: () => { selectedRow.value = null; } });
</script>

<template>
  <div class="watchlist-container">
    <div class="watchlist-header">
      <h2 class="section-title">自选股</h2>
      <button class="add-btn" @click="showAddDialog = true" aria-label="添加自选股票">
        <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true">
          <path d="M8.75 3.25a.75.75 0 00-1.5 0V7.5H3.25a.75.75 0 000 1.5h4v4.25a.75.75 0 001.5 0V9h4.25a.75.75 0 000-1.5h-4.25V3.25z"/>
        </svg>
        添加自选
      </button>
    </div>

    <GroupToolbar />

    <div v-if="watchlist.error" class="error-state" role="alert">
      <p class="error-text">{{ watchlist.error }}</p>
      <NButton size="tiny" @click="watchlist.fetchWatchlist()">重试</NButton>
    </div>
    <div v-else-if="watchlist.items.length === 0" class="empty-state">
      <svg class="empty-icon" viewBox="0 0 32 32" width="32" height="32" fill="none" aria-hidden="true">
        <rect x="4" y="6" width="24" height="20" rx="2" stroke="currentColor" stroke-width="1.5"/>
        <line x1="4" y1="12" x2="28" y2="12" stroke="currentColor" stroke-width="1.5"/>
        <line x1="10" y1="16" x2="14" y2="16" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
        <line x1="10" y1="20" x2="18" y2="20" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
      </svg>
      <p class="empty-text">暂无自选股票</p>
      <p class="empty-hint">点击「添加自选」搜索并添加股票</p>
    </div>

    <NDataTable
      v-else
      :columns="columns"
      :scroll-x="1365"
      :data="watchlist.items"
      :bordered="false"
      :single-line="true"
      size="small"
      :row-props="(row: WatchItem) => ({
        style: `height: 36px; cursor: pointer; ${selectedRow?.id === row.id ? 'background: var(--color-bg-elevated, rgba(255,255,255,0.04))' : ''}`,
        tabindex: 0,
        title: '单击查看详情；双击或按 Enter 设置提醒',
        'aria-label': `${row.name} ${formatCode(row.code)}，单击查看详情，双击或按 Enter 设置提醒`,
        onContextmenu: (e: MouseEvent) => handleContextMenu(e, row),
        onDblclick: () => openAlertSettings(row),
        onClick: () => handleRowClick(row),
        onKeydown: (e: KeyboardEvent) => handleRowKeydown(e, row),
      })"
      flex-height
      class="watchlist-table"
    />

    <StockDetail
      v-if="selectedRow"
      :item="selectedRow"
      @close="selectedRow = null"
    />

    <HoldingDialog v-if="showHoldingDialog" v-model:show="showHoldingDialog" :item="holdingItem" @saved="loadHoldings" />
    <AddStockDialog v-model:show="showAddDialog" />
    <PriceAlertDialog
      v-if="showAlertDialog"
      v-model:show="showAlertDialog"
      :item="alertDialogItem"
    />

    <NModal
      v-model:show="showDeleteConfirm"
      preset="dialog"
      title="从全部自选删除"
      positive-text="确认删除"
      negative-text="取消"
      :loading="deleting"
      :mask-closable="!deleting"
      @positive-click="confirmDelete"
    >
      删除“{{ ctxMenuItem?.name }}”将同时清除与该股票关联的持仓与行情提醒，且无法撤销。确定继续吗？
    </NModal>

    <NDropdown
      :show="showCtxMenu"
      :x="ctxMenuX"
      :y="ctxMenuY"
      :options="ctxOptions"
      placement="bottom-start"
      trigger="manual"
      @select="handleCtxSelect"
      @clickoutside="showCtxMenu = false"
    />
  </div>
</template>

<style scoped>
.watchlist-container {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  padding: 0 var(--space-4);
}
.watchlist-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-3) 0;
  flex-shrink: 0;
}
.section-title {
  font-size: var(--text-md);
  font-weight: var(--font-weight-semibold);
  color: var(--color-text-primary);
  letter-spacing: -0.01em;
}
.add-btn {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 0 12px;
  height: 28px;
  border: none;
  border-radius: var(--radius-sm);
  background: var(--color-accent);
  color: #fff;
  font-size: var(--text-xs);
  font-family: var(--font-sans);
  font-weight: var(--font-weight-medium);
  cursor: pointer;
  transition: filter var(--transition-fast);
}
.add-btn:hover {
  filter: brightness(1.15);
}
.add-btn:active {
  filter: brightness(0.9);
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  color: var(--color-text-tertiary);
}
.empty-icon { color: var(--color-text-tertiary); opacity: 0.4; }
.empty-text { font-size: var(--text-md); font-weight: var(--font-weight-medium); color: var(--color-text-secondary); }
.empty-hint { font-size: var(--text-xs); }
.error-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-3);
}
.error-text {
  font-size: var(--text-sm);
  color: var(--color-warning);
  text-align: center;
  max-width: 300px;
}

:deep(.watchlist-table) {
  flex: 1;
}
/* P&L color classes (used via render functions) */
:deep(.pct-col) { font-weight: 500; }
:deep(.pct-col.up) { color: var(--color-up); }
:deep(.pct-col.down) { color: var(--color-down); }
/* 列渲染内容由 NDataTable 挂载，scoped 样式需用 :deep() 才能生效 */
:deep(.code-text) {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
:deep(.name-cell) {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
:deep(.name-text) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
