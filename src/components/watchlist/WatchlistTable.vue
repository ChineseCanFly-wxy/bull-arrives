<script setup lang="ts">
import { computed, ref, h, inject, onBeforeUnmount, onMounted, watch, defineAsyncComponent } from 'vue';
import { NButton, NDataTable, NDropdown, NModal, NTag, useMessage } from 'naive-ui';
import type { DataTableColumns } from 'naive-ui';
import { invoke } from '@tauri-apps/api/core';
import { useWatchlistStore } from '@/stores/watchlist';
import { useMonitorStore } from '@/stores/monitor';
import type { Monitor } from '@/types/monitor';
import { useQuoteStore } from '@/stores/quote';
import { useSettingsStore } from '@/stores/settings';
import type { WatchItem } from '@/types';
import type { StockAnalysis } from '@/types/analysis';
import { formatPrice, formatVolume, formatCode, cnCategory } from '@/utils/format';
import AddStockDialog from './AddStockDialog.vue';
const PriceAlertDialog = defineAsyncComponent(() => import('./PriceAlertDialog.vue'));
import GroupToolbar from './GroupToolbar.vue';
const HoldingDialog = defineAsyncComponent(() => import('./HoldingDialog.vue'));
import { calculateHoldingMetrics, scaledPriceToDecimal, type Holding } from '@/utils/holdings';
import MarketTag from './MarketTag.vue';
import StockDetail from '@/components/detail/StockDetail.vue';
import AnalysisDialog from '@/components/analysis/AnalysisDialog.vue';
import { CLEAR_INDEX_DETAIL_KEY } from '@/utils/keys';

const watchlist = useWatchlistStore();
const quoteStore = useQuoteStore();
const settings = useSettingsStore();
const monitorStore = useMonitorStore();
const message = useMessage();
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

/**
 * 打开个股量化分析（含操作计划：买点 / 止损 / 止盈 / 仓位、规则历史回测、支撑压力位）。
 *
 * 自选股此前只能「整表评分排序」，看不到单只股票为什么是这个分、更看不到买卖点 ——
 * 这里补上逐只的分析入口。入口有两处：「操作」列的按钮（固定在表格右侧，
 * 不会被横向滚动藏掉）与右键菜单。
 *
 * 规则**交给后端按这只股票自身的状态自动匹配**，与筛选器结果表里点「分析」
 * 走的是同一条路（筛选器传的也是 'auto'）—— 自选股不属于任何筛选策略，
 * 不存在「策略配套规则」可继承，硬套一条反而是错配。
 */
const showAnalysisDialog = ref(false);
const analysisRow = ref<WatchItem | null>(null);
function openAnalysis(row: WatchItem) {
  cancelPendingRowClick();
  showCtxMenu.value = false;
  analysisRow.value = row;
  showAnalysisDialog.value = true;
}
function holdingMetric(row: WatchItem) {
  const holding = holdings.value.get(row.id);
  const quote = quoteStore.getQuote(row.code, row.market);
  if (!holding || !quote || quote.price <= 0) return null;
  try { return calculateHoldingMetrics(holding, quote.price); }
  catch { return null; }
}

/** 该自选股对应的监控规则（未开启为 undefined） */
function monitorOf(row: WatchItem): Monitor | undefined {
  return monitorStore.monitors.find(m => m.code === row.code && m.market === row.market);
}

/**
 * 右键菜单里的「开启监控 / 停止监控 / 恢复监控」。
 *
 * 三态由当前状态决定，用户不用先想清楚"我现在是哪一步"：
 * - 没有规则 → 开启（后端拉日 K 算 ATR 止损/止盈位，要等一次网络往返）
 * - 有规则且在跑 → **停止**（只翻开关，价位原样留着）
 * - 有规则但已停 → 恢复
 *
 * 停止/恢复走 `set_monitor_enabled`，**不重算价位** —— 这是它与「开启」的关键区别：
 * 重算会用当时的收盘价当参考价，止损/止盈整体挪位，等于换了一套规则。
 */
async function toggleMonitor(row: WatchItem | null) {
  if (!row) return;
  showCtxMenu.value = false;
  const existing = monitorOf(row);
  if (!existing) {
    const created = await monitorStore.save(row.code, row.market, row.name, true);
    if (!created) {
      message.error(monitorStore.error ?? '开启监控失败');
      return;
    }
    message.success(`已开启“${row.name}”监控：止损 ${created.stop_price.toFixed(2)}，止盈 ${created.take_price.toFixed(2)}`);
    return;
  }
  const next = !existing.enabled;
  const ok = await monitorStore.setEnabled(row.code, row.market, next);
  if (!ok) {
    message.error(monitorStore.error ?? `${next ? '恢复' : '停止'}监控失败`);
    return;
  }
  message.success(
    next
      ? `已恢复监控“${row.name}”，继续按原止损 ${existing.stop_price.toFixed(2)} / 止盈 ${existing.take_price.toFixed(2)} 判断`
      : `已停止监控“${row.name}”，规则与价位都保留，可随时恢复`,
  );
}

const alertDialogItem = ref<WatchItem | null>(null);
const showDeleteConfirm = ref(false);
const deleting = ref(false);
const selectedRowKeys = ref<number[]>([]);
const pendingDeleteItems = ref<WatchItem[]>([]);
const pendingDeleteGroupId = ref(0);
let rowClickTimer: ReturnType<typeof setTimeout> | null = null;

watch(() => watchlist.activeGroupId, () => {
  cancelPendingRowClick();
  selectedRow.value = null;
  ctxMenuItem.value = null;
  showCtxMenu.value = false;
  showDeleteConfirm.value = false;
  selectedRowKeys.value = [];
  pendingDeleteItems.value = [];
  pendingDeleteGroupId.value = 0;
  scoreSortActive.value = false;
  scoreById.value = new Map();
  scoreGeneration++;
  scoring.value = false;
});

watch(() => watchlist.items, items => {
  const visibleIds = new Set(items.map(item => item.id));
  selectedRowKeys.value = selectedRowKeys.value.filter(id => visibleIds.has(id));
});

const indexDetailCoord = inject<{
  clearIndexDetail: () => void;
  registerClearStockFn?: (fn: () => void) => void;
} | undefined>(CLEAR_INDEX_DETAIL_KEY);

onMounted(() => {
  void loadHoldings();
  // 监控列表只在监控面板打开时才会拉取；主表要显示监控状态列，启动时先拉一次
  if (!monitorStore.hasLoaded) void monitorStore.fetchMonitors();
  indexDetailCoord?.registerClearStockFn?.(() => {
    cancelPendingRowClick();
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

const SCORE_CONCURRENCY = 3;
const scoreById = ref<Map<number, number | null>>(new Map());
const scoreSortActive = ref(false);
const scoring = ref(false);
let scoreGeneration = 0;
const scoreProgress = ref({ done: 0, total: 0, failed: 0 });

function scoreOf(row: WatchItem): number {
  return scoreById.value.get(row.id) ?? -1;
}

const displayedItems = computed(() => {
  const items = [...watchlist.items];
  if (!scoreSortActive.value) return items;
  return items.sort((a, b) => scoreOf(b) - scoreOf(a) || a.id - b.id);
});

const selectedItems = computed(() => {
  const selected = new Set(selectedRowKeys.value);
  return displayedItems.value.filter(item => selected.has(item.id));
});

function updateSelectedRowKeys(keys: Array<string | number>) {
  selectedRowKeys.value = keys.map(Number).filter(Number.isInteger);
}

function rowKey(row: WatchItem): number {
  return row.id;
}

async function scoreAndSort() {
  const items = [...watchlist.items];
  if (!items.length || scoring.value) return;

  const generation = ++scoreGeneration;
  scoring.value = true;
  scoreSortActive.value = true;
  scoreProgress.value = { done: 0, total: items.length, failed: 0 };
  const next = new Map<number, number | null>();

  const scoreItem = async (item: WatchItem) => {
    try {
      const analysis = await invoke<StockAnalysis>('analyze_stock', { symbol: item.code });
      if (generation !== scoreGeneration) return;
      next.set(item.id, analysis.total_score);
    } catch (error) {
      if (generation !== scoreGeneration) return;
      console.warn(`[watchlist] 量化评分失败 ${item.code}:`, error);
      next.set(item.id, null);
      scoreProgress.value = { ...scoreProgress.value, failed: scoreProgress.value.failed + 1 };
    } finally {
      if (generation === scoreGeneration) {
        scoreById.value = new Map(next);
        scoreProgress.value = { ...scoreProgress.value, done: scoreProgress.value.done + 1 };
      }
    }
  };

  try {
    for (let start = 0; start < items.length && generation === scoreGeneration; start += SCORE_CONCURRENCY) {
      await Promise.all(items.slice(start, start + SCORE_CONCURRENCY).map(scoreItem));
    }
  } finally {
    if (generation === scoreGeneration) scoring.value = false;
  }
}

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

function isSelectionEvent(event: Event): boolean {
  return event.target instanceof Element && event.target.closest('[role="checkbox"]') !== null;
}

onBeforeUnmount(() => {
  cancelPendingRowClick();
  scoreGeneration++;
});

function handleContextMenu(e: MouseEvent, row: WatchItem) {
  e.preventDefault();
  // Clamp menu position to viewport so it never renders off-screen
  const menuW = 150; // approximate menu width
  const menuH = 380; // approximate menu height（最多 10 项 + 3 条分隔线）
  ctxMenuX.value = Math.min(e.clientX, window.innerWidth - menuW);
  ctxMenuY.value = Math.min(e.clientY, window.innerHeight - menuH);
  ctxMenuItem.value = row;
  showCtxMenu.value = true;
}

function requestDelete() {
  showCtxMenu.value = false;
  const item = ctxMenuItem.value;
  if (!item) return;
  pendingDeleteItems.value = [item];
  pendingDeleteGroupId.value = watchlist.activeGroupId;
  if (pendingDeleteGroupId.value === 0) {
    showDeleteConfirm.value = true;
  } else {
    void confirmDelete();
  }
}

function requestBulkDelete() {
  if (!selectedItems.value.length || deleting.value) return;
  pendingDeleteItems.value = [...selectedItems.value];
  pendingDeleteGroupId.value = watchlist.activeGroupId;
  if (pendingDeleteGroupId.value === 0) {
    showDeleteConfirm.value = true;
  } else {
    void confirmDelete();
  }
}

async function confirmDelete() {
  const items = pendingDeleteItems.value;
  const groupId = pendingDeleteGroupId.value;
  if (!items.length || deleting.value) return;
  deleting.value = true;
  try {
    await watchlist.removeStocks(items, groupId);
    selectedRow.value = null;
    showDeleteConfirm.value = false;
    selectedRowKeys.value = [];
    pendingDeleteItems.value = [];
  } catch (e) {
    console.error('removeStocks failed:', e);
  } finally {
    deleting.value = false;
  }
}

/**
 * 删除监控规则（自选股本身不受影响）。
 *
 * 与「停止监控」的区别：停止只翻开关、价位与触发状态原样留着；
 * 删除是把这条规则拿掉，重新开启会按当时的收盘价重算止损/止盈位。
 * 所以这里要确认一次。
 */
const monitorDeleteRow = ref<WatchItem | null>(null);
const showMonitorDeleteConfirm = ref(false);
const monitorDeleting = ref(false);

function requestMonitorDelete() {
  showCtxMenu.value = false;
  const row = ctxMenuItem.value;
  if (!row) return;
  monitorDeleteRow.value = row;
  showMonitorDeleteConfirm.value = true;
}

async function confirmMonitorDelete() {
  const row = monitorDeleteRow.value;
  if (!row || monitorDeleting.value) return;
  monitorDeleting.value = true;
  try {
    const ok = await monitorStore.remove(row.code, row.market);
    if (!ok) {
      message.error(monitorStore.error ?? '删除监控失败');
      return;
    }
    showMonitorDeleteConfirm.value = false;
    monitorDeleteRow.value = null;
    message.success(`已删除“${row.name}”的监控`);
  } finally {
    monitorDeleting.value = false;
  }
}

const deleteDialogTitle = computed(() => pendingDeleteGroupId.value === 0 ? '从全部自选删除' : '从当前分组移出');

const deleteDialogContent = computed(() => {
  const count = pendingDeleteItems.value.length;
  const subject = count === 1 ? `“${pendingDeleteItems.value[0]?.name ?? ''}”` : `所选 ${count} 只股票`;
  if (pendingDeleteGroupId.value === 0) {
    return `删除${subject}将同时清除关联的持仓与行情提醒，且无法撤销。确定继续吗？`;
  }
  return `从当前分组移出${subject}；股票仍保留在“全部”自选、其他分组、持仓和提醒中。确定继续吗？`;
});

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
// 个股分析：柱状图 + 放大镜，和「评分/分析」的语义对齐
const iconAnalyze = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 1.4, style: 'vertical-align:middle;margin-right:6px' }, [
  h('path', { d: 'M2.5 13.5h11' }),
  h('path', { d: 'M4.5 11V8.5' }),
  h('path', { d: 'M7.5 11V5.5' }),
  h('path', { d: 'M10.5 11V7.5' }),
  h('circle', { cx: 12.4, cy: 4.2, r: 2.1 }),
  h('path', { d: 'M14 5.8l1.1 1.1' }),
]);
// 停止监控：暂停（方块）—— 语义是"停一下"，不是"删掉"
const iconMonitorPause = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 1.5, style: 'vertical-align:middle;margin-right:6px' }, [
  h('circle', { cx: 8, cy: 8, r: 5.5 }),
  h('path', { d: 'M6.6 5.8v4.4' }),
  h('path', { d: 'M9.4 5.8v4.4' }),
]);
// 开启 / 恢复监控：播放三角形
const iconMonitorPlay = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 1.5, style: 'vertical-align:middle;margin-right:6px' }, [
  h('circle', { cx: 8, cy: 8, r: 5.5 }),
  h('path', { d: 'M6.6 5.6l4 2.4-4 2.4z' }),
]);
// 删除监控：圆圈打叉 —— 与「暂停」（两条竖线）、「播放」（三角）同一套形状语言，
// 和下面「从全部自选删除」的垃圾桶也区分得开：删的是监控规则，不是这只股票
const iconMonitorDelete = () => h('svg', { viewBox: '0 0 16 16', width: 14, height: 14, fill: 'none', stroke: 'currentColor', strokeWidth: 1.5, style: 'vertical-align:middle;margin-right:6px' }, [
  h('circle', { cx: 8, cy: 8, r: 5.5 }),
  h('path', { d: 'M6.1 6.1l3.8 3.8' }),
  h('path', { d: 'M9.9 6.1l-3.8 3.8' }),
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
  const row = ctxMenuItem.value;
  const monitored = row ? monitorOf(row) : undefined;
  // 监控项跟着当前状态走，不出现点了没反应的空项：
  //   未监控 → 开启监控
  //   监控中 → 停止监控 + 删除监控
  //   已停止 → 恢复监控 + 删除监控
  // 「开启」与「恢复」走同一个 key：对没有规则的票是新建，对已停的票是翻回启用，
  // 后端 `save_monitor` 两种都处理得对，前端不必分岔。
  const monitorItems = monitored
    ? [
        monitored.enabled
          ? { label: '停止监控', key: 'monitor-off', icon: iconMonitorPause }
          : { label: '恢复监控', key: 'monitor-on', icon: iconMonitorPlay },
        { label: '删除监控', key: 'monitor-delete', icon: iconMonitorDelete },
      ]
    : [{ label: '开启监控', key: 'monitor-on', icon: iconMonitorPlay }];
  return [
    { label: '个股分析', key: 'analyze', icon: iconAnalyze },
    { label: '设置行情提醒', key: 'alert', icon: iconAlert },
    { type: 'divider' as const, key: 'd0' },
    ...monitorItems,
    { type: 'divider' as const, key: 'd1' },
    { label: '置顶', key: 'top', icon: iconTop },
    { label: '上移', key: 'up', icon: iconUp },
    { label: '下移', key: 'down', icon: iconDown },
    { type: 'divider' as const, key: 'd2' },
    {
      label: grouped ? '从当前分组移出' : '从全部自选删除',
      key: 'delete',
      icon: iconDelete,
    },
  ];
});

function handleCtxSelect(key: string) {
  switch (key) {
    case 'analyze': if (ctxMenuItem.value) openAnalysis(ctxMenuItem.value); break;
    case 'alert': if (ctxMenuItem.value) openAlertSettings(ctxMenuItem.value); break;
    case 'monitor-on': void toggleMonitor(ctxMenuItem.value); break;
    case 'monitor-off': void toggleMonitor(ctxMenuItem.value); break;
    case 'monitor-delete': requestMonitorDelete(); break;
    case 'top': void handleMoveTop(); break;
    case 'up': void handleMoveUp(); break;
    case 'down': void handleMoveDown(); break;
    case 'delete': requestDelete(); break;
  }
}

const columns: DataTableColumns<WatchItem> = [
  { type: 'selection' },
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
    title: '评分', key: 'quant_score', width: 76,
    sorter: (a: WatchItem, b: WatchItem) => scoreOf(a) - scoreOf(b),
    render(row) {
      const score = scoreById.value.get(row.id);
      return h('span', { class: ['quant-score', score != null && score >= 60 ? 'up' : score != null && score < 45 ? 'down' : ''] }, score == null ? '--' : score.toFixed(1));
    }
  },
  {
    // 智能监控状态：不开监控面板也能一眼看到哪只票在监控、是否已触发；
    // 悬停显示参考价 / 止损 / 止盈和相对现价的距离。
    title: '监控', key: 'monitor', width: 88, align: 'center',
    sorter: (a: WatchItem, b: WatchItem) => Number(monitorOf(b) != null) - Number(monitorOf(a) != null),
    render(row) {
      const m = monitorOf(row);
      if (!m) return h('span', { class: 'monitor-none' }, '--');
      const q = quoteStore.getQuote(row.code, row.market);
      const tipLines = [
        `参考价 ${m.reference_price.toFixed(2)}`,
        `止损 ${m.stop_price.toFixed(2)}`,
        `止盈 ${m.take_price.toFixed(2)}`,
      ];
      if (q && q.price > 0) {
        tipLines.push(
          `距止损 ${((q.price - m.stop_price) / q.price * 100).toFixed(1)}%`,
          `距止盈 ${((m.take_price - q.price) / q.price * 100).toFixed(1)}%`,
        );
      }
      // 已停止的票把重点放在"它现在不会提醒你"上，价位信息退到悬停里 ——
      // 状态列回答的是"会不会响"，不是"价位是多少"
      if (!m.enabled) {
        return h(
          NTag,
          {
            type: 'warning',
            size: 'small',
            bordered: false,
            title: `已停止监控（规则保留，右键可恢复）\n${tipLines.join('\n')}`,
          },
          { default: () => '已停止' },
        );
      }
      const tip = tipLines.join('\n');
      if (m.last_triggered === 'stop_loss') {
        return h(NTag, { type: 'error', size: 'small', bordered: false, title: tip }, { default: () => '已止损' });
      }
      if (m.last_triggered === 'take_profit') {
        return h(NTag, { type: 'success', size: 'small', bordered: false, title: tip }, { default: () => '已止盈' });
      }
      return h(NTag, { type: 'default', size: 'small', bordered: false, title: tip }, { default: () => '监控中' });
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
  {
    // ⚠️ 必须 fixed: 'right'：这张表一共 16 列、横向要滚，固定在右边才不会被滚出去。
    // 之前它跟着表格滚，窄窗口下用户根本看不到这里有个「分析」——入口等于不存在。
    title: '操作', key: 'action', width: 76, align: 'center', fixed: 'right',
    render(row) {
      return h(
        NButton,
        {
          size: 'tiny',
          tertiary: true,
          // 行上的单击/双击另有含义（看详情 / 设提醒），必须阻止冒泡
          onClick: (event: MouseEvent) => {
            event.stopPropagation();
            openAnalysis(row);
          },
        },
        { default: () => '分析' },
      );
    }
  },
];

defineExpose({ clearSelection: () => { cancelPendingRowClick(); selectedRow.value = null; } });
</script>

<template>
  <div class="watchlist-container">
    <div class="watchlist-header">
      <div class="section-heading"><span class="section-kicker">WATCHLIST</span><h2 class="section-title">自选股 <small class="watch-count">{{ watchlist.items.length }}</small></h2></div>
      <div class="watchlist-actions">
        <NButton
          v-if="selectedRowKeys.length"
          size="small"
          type="error"
          secondary
          :loading="deleting"
          @click="requestBulkDelete"
        >
          {{ watchlist.activeGroupId === 0 ? '删除' : '移出' }}所选 ({{ selectedRowKeys.length }})
        </NButton>
        <NButton size="small" secondary :disabled="!watchlist.items.length" :loading="scoring" @click="scoreAndSort">
          {{ scoring ? `评分 ${scoreProgress.done}/${scoreProgress.total}` : scoreSortActive ? '重新评分排序' : '量化评分排序' }}
        </NButton>
        <button class="add-btn" @click="showAddDialog = true" aria-label="添加自选股票">
          <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true">
            <path d="M8.75 3.25a.75.75 0 00-1.5 0V7.5H3.25a.75.75 0 000 1.5h4v4.25a.75.75 0 000-1.5h-4.25V3.25z"/>
          </svg>
          添加自选
        </button>
      </div>
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
      :scroll-x="1600"
      :data="displayedItems"
      :bordered="false"
      :single-line="true"
      size="small"
      :row-props="(row: WatchItem) => ({
        style: `height: var(--table-row-height); cursor: pointer; ${selectedRow?.id === row.id ? `background: ${settings.visualStyle === 'classic' ? 'var(--color-bg-elevated)' : 'var(--color-accent-dim)'}` : ''}`,
        tabindex: 0,
        title: '单击查看详情；双击或按 Enter 设置提醒',
        'aria-label': `${row.name} ${formatCode(row.code)}，单击查看详情，双击或按 Enter 设置提醒`,
        onContextmenu: (e: MouseEvent) => handleContextMenu(e, row),
        onDblclick: (e: MouseEvent) => { if (!isSelectionEvent(e)) openAlertSettings(row); },
        onClick: (e: MouseEvent) => { if (!isSelectionEvent(e)) handleRowClick(row); },
        onKeydown: (e: KeyboardEvent) => { if (!isSelectionEvent(e)) handleRowKeydown(e, row); },
      })"
      :row-key="rowKey"
      :checked-row-keys="selectedRowKeys"
      @update:checked-row-keys="updateSelectedRowKeys"
      flex-height
      class="watchlist-table"
    />

    <StockDetail
      v-if="selectedRow"
      :item="selectedRow"
      @close="selectedRow = null"
    />

    <AnalysisDialog
      v-if="analysisRow"
      v-model:show="showAnalysisDialog"
      :symbol="analysisRow.code"
      :name="analysisRow.name"
      rule="auto"
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
      :title="deleteDialogTitle"
      positive-text="确认删除"
      negative-text="取消"
      :loading="deleting"
      :mask-closable="!deleting"
      @positive-click="confirmDelete"
    >
      {{ deleteDialogContent }}
    </NModal>

    <NModal
      v-model:show="showMonitorDeleteConfirm"
      preset="dialog"
      title="删除监控"
      positive-text="确认删除"
      negative-text="取消"
      :loading="monitorDeleting"
      :mask-closable="!monitorDeleting"
      @positive-click="confirmMonitorDelete"
    >
      不再监控“{{ monitorDeleteRow?.name ?? '' }}”？删除后已算好的止损/止盈位不再保留，
      以后重新开启会按当时的收盘价重算。自选股与持仓数据不受影响。
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
  overflow: hidden;
  padding: 0 var(--workspace-gutter);
}
.watchlist-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) 0 var(--space-2);
  flex-shrink: 0;
}
.section-heading { display: flex; align-items: baseline; gap: var(--space-2); min-width: 0; }
.section-kicker { font: 600 10px var(--font-mono); letter-spacing: .08em; color: var(--color-accent); }
:global([data-style="classic"]) .section-kicker,
:global([data-style="classic"]) .watch-count { display: none; }
:global([data-style="classic"]) .watchlist-container { padding-inline: var(--space-4); }
:global([data-style="classic"]) .watchlist-header { gap: 0; padding: var(--space-3) 0; }
:global([data-style="classic"]) .watchlist-actions { flex-wrap: nowrap; }
:global([data-style="classic"]) .add-btn { height: 28px; color: #fff; }
.section-title small { color: var(--color-text-tertiary); font-size: var(--text-xs); font-weight: 400; }
.watchlist-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
  justify-content: flex-end;
}
.section-title {
  margin: 0;
  white-space: nowrap;
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
  height: var(--control-height);
  border: none;
  border-radius: var(--radius-sm);
  background: var(--color-accent);
  color: var(--color-accent-contrast);
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
  flex: 1 1 44%;
  min-height: 160px;
}
:global([data-style="trading"]) .watchlist-container :deep(.watchlist-table .n-data-table-th),
:global([data-style="modern"]) .watchlist-container :deep(.watchlist-table .n-data-table-th) { background: var(--color-surface-2); color: var(--color-text-secondary); }
:global([data-style="trading"]) .watchlist-container :deep(.watchlist-table .n-data-table-td),
:global([data-style="modern"]) .watchlist-container :deep(.watchlist-table .n-data-table-td) { border-color: var(--color-border-0); }
:global([data-style="modern"]) .watchlist-container :deep(.watchlist-table) { border: 1px solid var(--color-border-0); border-radius: var(--radius-md); overflow: hidden; }
/* P&L color classes (used via render functions) */
:deep(.pct-col) { font-weight: 500; }
:deep(.pct-col.up) { color: var(--color-up); }
:deep(.pct-col.down) { color: var(--color-down); }
:deep(.quant-score) { font-family: var(--font-mono); font-variant-numeric: tabular-nums; }
:deep(.monitor-none) { color: var(--color-text-tertiary); }
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
@media (max-width: 680px) {
  .watchlist-header { align-items: flex-start; flex-wrap: wrap; }
  .watchlist-actions { justify-content: flex-start; }
  .section-kicker { display: none; }
}
@media (max-height: 550px) {
  .watchlist-header { padding-block: 4px; }
  :deep(.watchlist-table) { min-height: 110px; }
}
</style>
