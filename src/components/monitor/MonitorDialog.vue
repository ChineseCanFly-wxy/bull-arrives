<script setup lang="ts">
// src/components/monitor/MonitorDialog.vue
// 持仓监控（量化自动止损/止盈）对话框 —— 所有监控规则的统一管理页。
// 用户只需选择股票，止损/止盈位由量化模型（ATR）自动计算，无需手填参数。
//
// 页面分三层：
// 1. **添加区**：可搜索任意 A 股（不限于自选股），选中后「添加监控」；
//    自选股仍作为下拉的初始候选，随手就能选。
// 2. **筛选区**：关键词 + 全部/监控中/已停止，回答「现在到底有哪些票在监控」。
// 3. **列表区**：监控中的票逐条列出代码/名称/参考价/止损/止盈/状态，
//    行内可 停止|恢复、刷新、删除。
//
// 「停止」与「删除」是两件事：
// - 停止 = 保留规则与已算好的价位，只是不再触发提醒（随时恢复，价位不变）；
// - 删除 = 不再监控这只票了。
// 不能用「删掉再开启」代替停止 —— 那会用当时的收盘价重算参考价，价位整体挪位。

import { computed, h, onUnmounted, ref, watch } from 'vue';
import {
  NButton,
  NDataTable,
  NEmpty,
  NInput,
  NModal,
  NPopconfirm,
  NRadioButton,
  NRadioGroup,
  NSelect,
  NTag,
  type DataTableColumns,
  type SelectOption,
} from 'naive-ui';
import { invoke } from '@tauri-apps/api/core';
import { useMonitorStore } from '@/stores/monitor';
import { useWatchlistStore } from '@/stores/watchlist';
import { useSettingsStore } from '@/stores/settings';
import type { Monitor } from '@/types/monitor';
import type { StockBrief } from '@/types';
import { cnCategory, formatCode } from '@/utils/format';

const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();

const monitorStore = useMonitorStore();
const watchlist = useWatchlistStore();
const settings = useSettingsStore();

/// 后台自动运行的智能监控已被设置里的开关关掉，规则保留但不会触发。
const paused = computed(() => !settings.aiEnabled || !settings.aiMonitorEnabled);

const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

// ── 添加区 ──────────────────────────────────────────────
const selectedCode = ref<string | null>(null);
/** 当前选中的标的。不能只留 code —— 搜索结果随时会被下一次输入覆盖掉 */
const selectedBrief = ref<StockBrief | null>(null);
const pickerOptions = ref<SelectOption[]>([]);
const searching = ref(false);
const adding = ref(false);
/** 当前下拉项对应的标的索引，选中时用它取 market / name */
let briefIndex = new Map<string, StockBrief>();
let searchTimer: ReturnType<typeof setTimeout> | null = null;

const formError = ref<string | null>(null);
const formNotice = ref<string | null>(null);

onUnmounted(() => {
  if (searchTimer) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
});

/** 自选股作为下拉的初始候选：不搜也能直接选 */
function selfPickBriefs(): StockBrief[] {
  return watchlist.items.map(item => ({
    code: item.code,
    market: item.market,
    name: item.name,
    category: cnCategory(item.code),
  }));
}

function setPickerOptions(briefs: StockBrief[]) {
  briefIndex = new Map(briefs.map(brief => [brief.code, brief]));
  const options: SelectOption[] = briefs.map(brief => ({
    label: `${brief.name}（${formatCode(brief.code)}）`,
    value: brief.code,
  }));
  // 已选中的票若不在当前候选里，补在最前面 ——
  // 否则 n-select 找不到对应 option，会把标签退化成裸代码
  const picked = selectedBrief.value;
  if (picked && !briefIndex.has(picked.code)) {
    briefIndex.set(picked.code, picked);
    options.unshift({ label: `${picked.name}（${formatCode(picked.code)}）`, value: picked.code });
  }
  pickerOptions.value = options;
}

/** n-select 的 value 类型是 string | number | null，这里统一收敛成 code */
function handlePick(value: string | number | null) {
  const code = value == null ? null : String(value);
  selectedCode.value = code;
  if (!code) {
    selectedBrief.value = null;
    return;
  }
  selectedBrief.value = briefIndex.get(code) ?? selectedBrief.value;
}

/** 输入即搜（远程搜索）；清空输入时退回自选股候选 */
function handleSearch(keyword: string) {
  if (searchTimer) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  const trimmed = keyword.trim();
  if (!trimmed) {
    searching.value = false;
    setPickerOptions(selfPickBriefs());
    return;
  }
  searchTimer = setTimeout(async () => {
    searching.value = true;
    try {
      const list = await invoke<StockBrief[]>('search_stocks', { keyword: trimmed });
      setPickerOptions(list);
    } catch (e) {
      setPickerOptions([]);
      formError.value = `搜索失败：${String(e).slice(0, 60)}`;
    } finally {
      searching.value = false;
    }
  }, 300);
}

function handleFocus() {
  // 打开下拉时若还没输入，先把自选股摆上
  if (!selectedCode.value && pickerOptions.value.length === 0) {
    setPickerOptions(selfPickBriefs());
  }
}

async function handleAdd() {
  formError.value = null;
  formNotice.value = null;
  const brief = selectedBrief.value;
  if (!brief) {
    formError.value = '请先搜索并选择要监控的股票';
    return;
  }
  const existed = monitorStore.monitors.some(m => m.code === brief.code && m.market === brief.market);
  adding.value = true;
  try {
    // enabled: true —— 明确要监控，对已停止的记录也一并启动
    const monitor = await monitorStore.save(brief.code, brief.market, brief.name, true);
    if (!monitor) {
      formError.value = monitorStore.error ?? '添加监控失败，请重试';
      return;
    }
    selectedCode.value = null;
    selectedBrief.value = null;
    setPickerOptions(selfPickBriefs());
    const priceText = `止损 ${fmt(monitor.stop_price)}，止盈 ${fmt(monitor.take_price)}`;
    formNotice.value = existed
      ? `「${brief.name}」已在监控中，已按最新收盘价重算并启动：${priceText}`
      : `已添加「${brief.name}」：${priceText}`;
  } finally {
    adding.value = false;
  }
}

// ── 筛选区 ──────────────────────────────────────────────
type FilterKind = 'all' | 'running' | 'stopped';
const filterKind = ref<FilterKind>('all');
const keywordFilter = ref('');

const runningCount = computed(() => monitorStore.monitors.filter(m => m.enabled).length);
const stoppedCount = computed(() => monitorStore.monitors.length - runningCount.value);

const filterOptions = computed<Array<{ label: string; value: FilterKind }>>(() => [
  { label: `全部 ${monitorStore.monitors.length}`, value: 'all' },
  { label: `监控中 ${runningCount.value}`, value: 'running' },
  { label: `已停止 ${stoppedCount.value}`, value: 'stopped' },
]);

function selectFilter(value: string | number) {
  filterKind.value = String(value) as FilterKind;
}

/** 当前筛选后要展示的监控规则 */
const filteredMonitors = computed(() => {
  const kw = keywordFilter.value.trim().toLowerCase();
  return monitorStore.monitors.filter(m => {
    if (filterKind.value === 'running' && !m.enabled) return false;
    if (filterKind.value === 'stopped' && m.enabled) return false;
    if (!kw) return true;
    return m.code.toLowerCase().includes(kw) || m.name.toLowerCase().includes(kw);
  });
});

/** 有票但被筛选条件挡住了 —— 空态文案要说清是「没有」还是「被筛掉了」 */
const filteredEmpty = computed(
  () => monitorStore.monitors.length > 0 && filteredMonitors.value.length === 0,
);

// ── 列表操作 ────────────────────────────────────────────
/** 正在切换启停的股票（`market:code`），用来给按钮上 loading */
const toggling = ref<Set<string>>(new Set());
const deleting = ref<Set<string>>(new Set());

function rowId(row: Monitor): string {
  return `${row.market}:${row.code}`;
}

function isBusy(set: Set<string>, row: Monitor): boolean {
  return set.has(rowId(row));
}

function withKey(set: Set<string>, key: string, add: boolean): Set<string> {
  const next = new Set(set);
  if (add) next.add(key);
  else next.delete(key);
  return next;
}

async function handleToggle(row: Monitor) {
  formError.value = null;
  formNotice.value = null;
  const key = rowId(row);
  const next = !row.enabled;
  // 同一只票重复点就忽略，避免连点发两次请求、状态来回翻
  if (toggling.value.has(key)) return;
  toggling.value = withKey(toggling.value, key, true);
  try {
    const ok = await monitorStore.setEnabled(row.code, row.market, next);
    if (!ok) {
      formError.value = monitorStore.error ?? '操作失败，请重试';
      return;
    }
    formNotice.value = next
      ? `已恢复监控「${row.name}」，继续按原止损 ${fmt(row.stop_price)} / 止盈 ${fmt(row.take_price)} 判断`
      : `已停止监控「${row.name}」，规则与价位都保留，随时可恢复`;
  } finally {
    toggling.value = withKey(toggling.value, key, false);
  }
}

async function handleDelete(row: Monitor) {
  formError.value = null;
  formNotice.value = null;
  const key = rowId(row);
  if (deleting.value.has(key)) return;
  deleting.value = withKey(deleting.value, key, true);
  try {
    const ok = await monitorStore.remove(row.code, row.market);
    if (!ok) {
      formError.value = monitorStore.error ?? '删除失败，请重试';
      return;
    }
    formNotice.value = `已删除「${row.name}」的监控。重新添加时会按当时的收盘价重算止损/止盈位`;
  } finally {
    deleting.value = withKey(deleting.value, key, false);
  }
}

/**
 * 重算止损/止盈位。
 *
 * `enabled` 传 null = 保持原有启停状态 —— 刷新价位不该顺手把用户停掉的监控重新打开。
 */
async function handleRefresh(row: Monitor) {
  formError.value = null;
  formNotice.value = null;
  const monitor = await monitorStore.save(row.code, row.market, row.name, null);
  if (!monitor) {
    formError.value = monitorStore.error ?? '刷新失败，请重试';
    return;
  }
  formNotice.value = `已重算「${row.name}」：止损 ${fmt(monitor.stop_price)}，止盈 ${fmt(monitor.take_price)}${
    monitor.enabled ? '' : '（仍处于停止状态）'
  }`;
}

// ── 打开时的加载 ────────────────────────────────────────
watch(
  () => props.show,
  open => {
    if (open) {
      formError.value = null;
      formNotice.value = null;
      if (!watchlist.items.length) void watchlist.fetchWatchlist();
      void monitorStore.fetchMonitors();
      setPickerOptions(selfPickBriefs());
    }
  },
  // 父组件用 v-if 挂载本组件：挂载时 show 已是 true，没有 immediate 就不会触发
  { immediate: true },
);

// 自选股是异步拉的：到货后刷新候选列表，否则下拉里空着
watch(
  () => watchlist.items,
  () => {
    if (selectedCode.value) return;
    setPickerOptions(selfPickBriefs());
  },
);

// ── 展示 ────────────────────────────────────────────────
function fmt(v: number): string {
  return Number.isFinite(v) ? v.toFixed(2) : '-';
}

/** 相对参考价的偏离百分比 */
function pct(v: number, refPrice: number): string {
  if (!Number.isFinite(v) || !Number.isFinite(refPrice) || refPrice <= 0) return '';
  return `${((v - refPrice) / refPrice * 100).toFixed(1)}%`;
}

function statusTag(m: Monitor) {
  // 停用的排在前面：状态列要回答「它现在会不会提醒我」，
  // 而「已经止盈过」是历史，停用与否更紧要
  if (!m.enabled) {
    return h(
      NTag,
      {
        type: 'warning',
        size: 'small',
        bordered: false,
        title: '已停止：规则与止损/止盈位都保留着，点「恢复」即可继续按原价位判断',
      },
      { default: () => '已停止' },
    );
  }
  if (m.last_triggered === 'stop_loss') {
    return h(NTag, { type: 'error', size: 'small', bordered: false, title: '已触发止损（一次性，重算价位后重置）' }, { default: () => '已止损' });
  }
  if (m.last_triggered === 'take_profit') {
    return h(NTag, { type: 'success', size: 'small', bordered: false, title: '已触发止盈（一次性，重算价位后重置）' }, { default: () => '已止盈' });
  }
  return h(NTag, { type: 'default', size: 'small', bordered: false }, { default: () => '监控中' });
}

const columns = computed<DataTableColumns<Monitor>>(() => [
  { title: '代码', key: 'code', width: 92, render: row => h('span', { class: 'mono' }, formatCode(row.code)) },
  { title: '名称', key: 'name', width: 108, ellipsis: { tooltip: true }, render: row => h('span', {}, row.name) },
  { title: '参考价', key: 'reference_price', width: 84, align: 'right', render: row => h('span', { class: 'mono' }, fmt(row.reference_price)) },
  {
    title: '止损价',
    key: 'stop',
    width: 118,
    align: 'right',
    render: row =>
      h('span', { class: 'mono down' }, `${fmt(row.stop_price)}（${pct(row.stop_price, row.reference_price)}）`),
  },
  {
    title: '止盈价',
    key: 'take',
    width: 118,
    align: 'right',
    render: row =>
      h('span', { class: 'mono up' }, `${fmt(row.take_price)}（+${pct(row.take_price, row.reference_price)}）`),
  },
  { title: '状态', key: 'status', width: 84, align: 'center', render: row => statusTag(row) },
  {
    title: '操作',
    key: 'action',
    width: 176,
    align: 'center',
    fixed: 'right',
    render: row => {
      const busy = isBusy(toggling.value, row);
      return h('div', { class: 'action-cell' }, [
        h(
          NButton,
          {
            size: 'tiny',
            tertiary: true,
            loading: busy,
            onClick: () => void handleToggle(row),
          },
          { default: () => (row.enabled ? '停止' : '恢复') },
        ),
        h(
          NButton,
          {
            size: 'tiny',
            tertiary: true,
            disabled: busy,
            onClick: () => void handleRefresh(row),
          },
          { default: () => '刷新' },
        ),
        // 删除不可撤销，包一层气泡确认；再套 n-modal 对话框会和本弹窗抢层级
        h(
          NPopconfirm,
          {
            positiveText: '删除',
            negativeText: '取消',
            onPositiveClick: () => void handleDelete(row),
          },
          {
            trigger: () =>
              h(
                NButton,
                { size: 'tiny', tertiary: true, type: 'error', loading: isBusy(deleting.value, row) },
                { default: () => '删除' },
              ),
            default: () =>
              `不再监控「${row.name}」？删除后已算好的止损/止盈位不再保留，重新添加会按当时的收盘价重算。`,
          },
        ),
      ]);
    },
  },
]);

function rowKey(row: Monitor): string {
  return rowId(row);
}
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="持仓监控 · 量化自动止损/止盈"
    :style="{ width: 'min(900px, calc(100vw - 24px))' }"
    :bordered="false"
    size="small"
  >
    <div class="monitor">
      <div class="form">
        <div class="form-row">
          <n-select
            :value="selectedCode"
            :options="pickerOptions"
            placeholder="搜索代码/名称添加监控，或直接选自选股"
            filterable
            remote
            clearable
            size="small"
            :loading="searching"
            style="width: 320px"
            @update:value="handlePick"
            @search="handleSearch"
            @focus="handleFocus"
          />
          <n-button
            type="primary"
            size="small"
            :loading="adding"
            :disabled="!selectedCode"
            @click="handleAdd"
          >
            添加监控
          </n-button>
        </div>
        <div v-if="formError" class="form-error">{{ formError }}</div>
        <div v-if="formNotice" class="form-notice">{{ formNotice }}</div>
        <div class="hint muted">
          止损/止盈位由量化模型自动计算（ATR 波动止损：止损 = 参考价 − 2×ATR14，止盈 = 参考价 + 3×ATR14），无需手动设置
        </div>
        <div class="hint muted">
          行里的「停止」只是不再触发提醒 —— 止损/止盈位与触发状态都留着，点「恢复」按原价位继续；
          「刷新」重算价位时也不会把停掉的监控自动打开。确实不再看这只票了才用「删除」。
        </div>
      </div>

      <div v-if="paused" class="paused-line">
        智能监控已在「设置 · 智能」中关闭，规则会保留但不会触发提醒。
      </div>

      <div v-if="monitorStore.error" class="error-line">{{ monitorStore.error }}</div>

      <div class="filter-row">
        <n-input
          v-model:value="keywordFilter"
          size="small"
          clearable
          placeholder="筛选监控中的票（代码或名称）"
          style="width: 220px"
        />
        <n-radio-group :value="filterKind" size="small" @update:value="selectFilter">
          <n-radio-button
            v-for="option in filterOptions"
            :key="option.value"
            :value="option.value"
          >
            {{ option.label }}
          </n-radio-button>
        </n-radio-group>
        <span v-if="monitorStore.monitors.length" class="count muted">
          当前显示 {{ filteredMonitors.length }} / {{ monitorStore.monitors.length }} 条
        </span>
        <n-button
          size="tiny"
          tertiary
          :loading="monitorStore.loading"
          @click="monitorStore.fetchMonitors()"
        >
          重新加载
        </n-button>
      </div>

      <div class="table-wrap">
        <n-data-table
          v-if="filteredMonitors.length"
          :columns="columns"
          :data="filteredMonitors"
          :loading="monitorStore.loading"
          :row-key="rowKey"
          size="small"
          :max-height="340"
          :scroll-x="780"
          :row-props="(row: Monitor) => ({
            style: row.enabled ? '' : 'opacity: 0.62',
            title: row.enabled
              ? `监控中：跌破 ${row.stop_price.toFixed(2)} 提醒止损，突破 ${row.take_price.toFixed(2)} 提醒止盈`
              : '已停止：不会触发提醒，规则与价位都保留',
          })"
        />
        <n-empty
          v-else-if="!monitorStore.loading && filteredEmpty"
          description="没有符合当前筛选条件的监控"
          size="small"
        />
        <n-empty
          v-else-if="!monitorStore.loading"
          description="还没有监控任何股票，在上方搜索并添加"
          size="small"
        />
      </div>
    </div>
  </n-modal>
</template>

<style scoped>
.monitor {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
:global([data-style="trading"]) .monitor,
:global([data-style="modern"]) .monitor { gap: var(--space-3); }
:global([data-style="trading"]) .monitor .form,
:global([data-style="modern"]) .monitor .form { padding: var(--panel-padding); border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-1); }
.form {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.form-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.form-error {
  font-size: var(--text-xs);
  color: var(--color-error);
}
.form-notice {
  font-size: var(--text-xs);
  color: var(--color-up);
}
.hint {
  font-size: var(--text-xs);
  line-height: 1.5;
}
.muted {
  color: var(--color-text-tertiary);
}
.error-line {
  flex-shrink: 0;
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-error-bg);
  color: var(--color-error);
  font-size: var(--text-xs);
}
.paused-line {
  flex-shrink: 0;
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-warning-bg, var(--color-surface-2));
  color: var(--color-warning, var(--color-text-secondary));
  font-size: var(--text-xs);
}
.filter-row {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
  padding-top: var(--space-1);
  border-top: 1px solid var(--color-border-0);
}
.count {
  font-size: var(--text-xs);
}
.table-wrap {
  min-height: 120px;
}
.table-wrap :deep(.mono) {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.table-wrap :deep(.up) {
  color: var(--color-up);
}
.table-wrap :deep(.down) {
  color: var(--color-down);
}
.table-wrap :deep(.action-cell) {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
</style>
