<script setup lang="ts">
// src/components/monitor/MonitorDialog.vue
// 持仓监控（量化自动止损/止盈）对话框。
// 用户只需选择股票，止损/止盈位由量化模型（ATR）自动计算，无需手填参数。

import { computed, h, ref, watch } from 'vue';
import {
  NButton,
  NDataTable,
  NEmpty,
  NModal,
  NSelect,
  NTag,
  type DataTableColumns,
} from 'naive-ui';
import { useMonitorStore } from '@/stores/monitor';
import { useWatchlistStore } from '@/stores/watchlist';
import { useSettingsStore } from '@/stores/settings';
import type { Monitor } from '@/types/monitor';

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

const selectedCode = ref<string | null>(null);
const enabling = ref(false);
const formError = ref<string | null>(null);
const formNotice = ref<string | null>(null);

const stockOptions = computed(() =>
  watchlist.items.map(item => ({
    label: `${item.name}（${item.code}）`,
    value: item.code,
  })),
);

watch(
  () => props.show,
  open => {
    if (open) {
      if (!watchlist.items.length) void watchlist.fetchWatchlist();
      void monitorStore.fetchMonitors();
    }
  },
  // 父组件用 v-if 挂载本组件：挂载时 show 已是 true，没有 immediate 就不会触发
  { immediate: true },
);

async function handleEnable() {
  formError.value = null;
  formNotice.value = null;
  const item = watchlist.items.find(i => i.code === selectedCode.value);
  if (!item) {
    formError.value = '请选择自选股（先在自选股里添加）';
    return;
  }
  enabling.value = true;
  try {
    const monitor = await monitorStore.save(item.code, item.market, item.name);
    if (!monitor) {
      formError.value = monitorStore.error ?? '开启监控失败，请重试';
      return;
    }
    selectedCode.value = null;
    formNotice.value = `已开启“${item.name}”：止损 ${fmt(monitor.stop_price)}，止盈 ${fmt(monitor.take_price)}`;
  } finally {
    enabling.value = false;
  }
}

function fmt(v: number): string {
  return Number.isFinite(v) ? v.toFixed(2) : '-';
}

/** 相对参考价的偏离百分比 */
function pct(v: number, refPrice: number): string {
  if (!Number.isFinite(v) || !Number.isFinite(refPrice) || refPrice <= 0) return '';
  return `${((v - refPrice) / refPrice * 100).toFixed(1)}%`;
}

function triggeredTag(m: Monitor) {
  if (m.last_triggered === 'stop_loss') return h(NTag, { type: 'error', size: 'small', bordered: false }, { default: () => '已止损' });
  if (m.last_triggered === 'take_profit') return h(NTag, { type: 'success', size: 'small', bordered: false }, { default: () => '已止盈' });
  return h(NTag, { type: 'default', size: 'small', bordered: false }, { default: () => '监控中' });
}

const columns = computed<DataTableColumns<Monitor>>(() => [
  { title: '代码', key: 'code', width: 90, render: row => h('span', { class: 'mono' }, row.code) },
  { title: '名称', key: 'name', width: 96, ellipsis: { tooltip: true }, render: row => h('span', {}, row.name) },
  { title: '参考价', key: 'reference_price', width: 80, align: 'right', render: row => h('span', { class: 'mono' }, fmt(row.reference_price)) },
  {
    title: '止损价',
    key: 'stop',
    width: 120,
    align: 'right',
    render: row =>
      h('span', { class: 'mono down' }, `${fmt(row.stop_price)}（${pct(row.stop_price, row.reference_price)}）`),
  },
  {
    title: '止盈价',
    key: 'take',
    width: 120,
    align: 'right',
    render: row =>
      h('span', { class: 'mono up' }, `${fmt(row.take_price)}（+${pct(row.take_price, row.reference_price)}）`),
  },
  { title: '状态', key: 'status', width: 82, align: 'center', render: row => triggeredTag(row) },
  {
    title: '操作',
    key: 'action',
    width: 120,
    align: 'center',
    render: row =>
      h('div', { class: 'action-cell' }, [
        h(NButton, { size: 'tiny', tertiary: true, onClick: () => void monitorStore.save(row.code, row.market, row.name) }, { default: () => '刷新' }),
        h(NButton, { size: 'tiny', tertiary: true, type: 'error', onClick: () => void monitorStore.remove(row.code, row.market) }, { default: () => '删除' }),
      ]),
  },
]);

function rowKey(row: Monitor): string {
  return row.code;
}
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="智能监控 · 量化自动止损/止盈"
    :style="{ width: '760px' }"
    :bordered="false"
    size="small"
  >
    <div class="monitor">
      <div class="form">
        <div class="form-row">
          <n-select
            v-model:value="selectedCode"
            :options="stockOptions"
            placeholder="选择自选股"
            filterable
            size="small"
            style="width: 200px"
          />
          <n-button type="primary" size="small" :loading="enabling" @click="handleEnable">
            开启监控
          </n-button>
        </div>
        <div v-if="formError" class="form-error">{{ formError }}</div>
        <div v-if="formNotice" class="form-notice">{{ formNotice }}</div>
        <div class="hint muted">
          止损/止盈位由量化模型自动计算（ATR 波动止损：止损 = 参考价 − 2×ATR14，止盈 = 参考价 + 3×ATR14），无需手动设置
        </div>
      </div>

      <div v-if="paused" class="paused-line">
        智能监控已在「设置 · 智能」中关闭，规则会保留但不会触发提醒。
      </div>

      <div v-if="monitorStore.error" class="error-line">{{ monitorStore.error }}</div>

      <div class="table-wrap">
        <n-data-table
          v-if="monitorStore.monitors.length"
          :columns="columns"
          :data="monitorStore.monitors"
          :loading="monitorStore.loading"
          :row-key="rowKey"
          size="small"
          flex-height
          :scroll-x="720"
        />
        <n-empty v-else-if="!monitorStore.loading" description="还没有监控，从上方选择股票开启" size="small" />
      </div>
    </div>
  </n-modal>
</template>

<style scoped>
.monitor {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-height: 220px;
}
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
.table-wrap {
  flex: 1;
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
