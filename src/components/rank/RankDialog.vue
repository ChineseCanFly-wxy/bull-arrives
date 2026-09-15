<script setup lang="ts">
// src/components/rank/RankDialog.vue
// 推荐榜对话框：对筛选结果里最活跃的 N 只批量评分，按总分排序展示。

import { computed, h, onBeforeUnmount, ref, watch } from 'vue';
import { NButton, NDataTable, NTag, type DataTableColumns } from 'naive-ui';
import { useRankStore } from '@/stores/rank';
import { useWatchlistStore } from '@/stores/watchlist';
import AnalysisDialog from '@/components/analysis/AnalysisDialog.vue';
import {
  BOARD_LABELS,
  formatAmount,
  formatPct,
  toFullSymbol,
  type MarketFilter,
} from '@/types/universe';
import { verdictTone } from '@/types/analysis';
import type { RankItem } from '@/types/rank';

const props = defineProps<{
  show: boolean;
  filter: MarketFilter;
}>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();

const rank = useRankStore();
const watchlist = useWatchlistStore();

const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

const addedSymbols = ref<Set<string>>(new Set());
const addError = ref<string | null>(null);

// 分析详情
const showAnalysis = ref(false);
const analysisTarget = ref<{ symbol: string; name: string }>({ symbol: '', name: '' });
let scanFrame: number | null = null;

watch(
  () => props.show,
  open => {
    if (!open) {
      if (scanFrame != null) cancelAnimationFrame(scanFrame);
      scanFrame = null;
      return;
    }
    addedSymbols.value = new Set();
    addError.value = null;
    rank.reset();
    // 先渲染加载中的弹窗，再开始可能需要数十秒的联网评分。
    scanFrame = requestAnimationFrame(() => {
      scanFrame = null;
      void rank.scan(props.filter);
    });
  },
  { flush: 'post', immediate: true },
);

onBeforeUnmount(() => {
  if (scanFrame != null) cancelAnimationFrame(scanFrame);
});

function changeClass(value: number): string {
  if (value > 0) return 'up';
  if (value < 0) return 'down';
  return 'flat';
}

function rowKey(row: RankItem): string {
  return row.code;
}

function openAnalysis(row: RankItem) {
  analysisTarget.value = { symbol: toFullSymbol(row.code, row.board), name: row.name };
  showAnalysis.value = true;
}

async function handleAdd(row: RankItem) {
  const symbol = toFullSymbol(row.code, row.board);
  addError.value = null;
  try {
    await watchlist.addStock(symbol, 'CN', row.name);
    addedSymbols.value = new Set(addedSymbols.value).add(symbol);
  } catch (e) {
    addError.value = `加自选失败：${e}`;
  }
}

const columns = computed<DataTableColumns<RankItem>>(() => [
  {
    title: '#',
    key: 'rank',
    width: 48,
    align: 'center',
    render: (_row, index) => h('span', { class: ['rank-badge', index < 3 ? `top-${index + 1}` : ''] }, String(index + 1)),
  },
  {
    title: '代码',
    key: 'code',
    width: 76,
    render: row => h('span', { class: 'mono' }, row.code),
  },
  {
    title: '名称',
    key: 'name',
    width: 104,
    ellipsis: { tooltip: true },
    render: row => h('span', {}, row.name),
  },
  {
    title: '涨跌幅',
    key: 'change_pct',
    width: 82,
    align: 'right',
    render: row => h('span', { class: ['mono', changeClass(row.change_pct)] }, formatPct(row.change_pct)),
  },
  {
    title: '成交额',
    key: 'amount',
    width: 86,
    align: 'right',
    render: row => h('span', { class: 'mono muted' }, formatAmount(row.amount)),
  },
  {
    title: '评分',
    key: 'score',
    width: 76,
    align: 'right',
    render: row =>
      row.analysis
        ? h('span', { class: ['mono score', changeClass(row.analysis.total_score - 50)] }, row.analysis.total_score.toFixed(1))
        : h('span', { class: 'muted' }, '—'),
  },
  {
    title: '结论',
    key: 'verdict',
    width: 92,
    render: row =>
      row.analysis
        ? h(NTag, { type: verdictTone(row.analysis.verdict), size: 'small', bordered: false }, { default: () => row.analysis!.verdict })
        : h('span', { class: 'muted', title: row.error ?? '' }, '评分失败'),
  },
  {
    title: '板块',
    key: 'board',
    width: 78,
    render: row => h('span', { class: 'muted' }, BOARD_LABELS[row.board] ?? row.board),
  },
  {
    title: '操作',
    key: 'action',
    width: 132,
    align: 'center',
    render: row => {
      const symbol = toFullSymbol(row.code, row.board);
      const done = addedSymbols.value.has(symbol);
      return h('div', { class: 'action-cell' }, [
        h(
          NButton,
          { size: 'tiny', tertiary: true, type: done ? 'default' : 'primary', disabled: done, onClick: () => void handleAdd(row) },
          { default: () => (done ? '已添加' : '加自选') },
        ),
        h(NButton, { size: 'tiny', tertiary: true, disabled: !row.analysis, onClick: () => openAnalysis(row) }, { default: () => '详情' }),
      ]);
    },
  },
]);
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="今日推荐榜"
    :style="{ width: '92vw' }"
    :z-index="3000"
    :bordered="false"
    size="small"
  >
    <div class="rank">
      <div class="toolbar">
        <span v-if="rank.result" class="stats">
          扫描 <b>{{ rank.result.scanned }}</b> 只 → 命中
          <b>{{ rank.result.total_matched }}</b> 只 → 评分
          <b>{{ rank.result.candidates }}</b> 只 → 成功
          <b>{{ rank.result.scored }}</b> 只
          <span v-if="rank.result.failed" class="muted">（失败 {{ rank.result.failed }}）</span>
        </span>
        <n-tag v-if="rank.result?.stale" type="warning" size="small" :bordered="false">快照陈旧，结果可能滞后</n-tag>
        <span v-if="addError" class="add-error">{{ addError }}</span>
      </div>

      <div v-if="rank.loading" class="notice-line" role="status">
        正在扫描活跃股并计算推荐榜，通常需要十几秒。
      </div>

      <div v-if="rank.result?.skipped_conditions?.length" class="notice-line">
        当前数据源不提供「{{ rank.result.skipped_conditions.join('、') }}」，该条件已自动忽略。
      </div>

      <div v-if="rank.error" class="error-line">{{ rank.error }}</div>

      <div class="table-wrap">
        <!-- 同筛选器：不用 flex-height / virtual-scroll，避免布局依赖导致行不渲染 -->
        <n-data-table
          :columns="columns"
          :data="rank.result?.items ?? []"
          :loading="rank.loading"
          :row-key="rowKey"
          size="small"
          :max-height="420"
          :scroll-x="840"
        />
        <div
          v-if="!rank.loading && rank.result && !rank.result.items.length && !rank.error"
          class="empty-line"
        >
          没有符合条件的股票 —— 榜单只对最活跃的一批评分，可以放宽筛选条件再试。
        </div>
      </div>
    </div>
  </n-modal>

  <AnalysisDialog
    v-model:show="showAnalysis"
    :symbol="analysisTarget.symbol"
    :name="analysisTarget.name"
  />
</template>

<style scoped>
.rank {
  display: flex;
  flex-direction: column;
  height: calc(84vh - 96px);
  gap: var(--space-2);
}
.toolbar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.stats {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
}
.stats b {
  color: var(--color-accent);
}
.muted {
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
}
.mono {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.add-error {
  font-size: var(--text-xs);
  color: var(--color-error);
}
.error-line {
  flex-shrink: 0;
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-error-bg);
  color: var(--color-error);
  font-size: var(--text-xs);
}
.notice-line {
  flex-shrink: 0;
  padding: var(--space-1) var(--space-2);
  border-left: 2px solid var(--color-warning);
  border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
  background: var(--color-warning-bg);
  color: var(--color-warning);
  font-size: var(--text-xs);
  line-height: 1.55;
}
.empty-line {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  min-height: 120px;
  padding: 0 var(--space-3);
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  text-align: center;
}
.table-wrap {
  flex: 1;
  min-height: 0;
}
.table-wrap :deep(.rank-badge) {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  font-family: var(--font-mono);
  color: var(--color-text-tertiary);
}
.table-wrap :deep(.rank-badge.top-1) {
  background: #f85149;
  color: #fff;
}
.table-wrap :deep(.rank-badge.top-2) {
  background: #e8833a;
  color: #fff;
}
.table-wrap :deep(.rank-badge.top-3) {
  background: #d4a72c;
  color: #fff;
}
.table-wrap :deep(.up) {
  color: var(--color-up);
}
.table-wrap :deep(.down) {
  color: var(--color-down);
}
.table-wrap :deep(.flat) {
  color: var(--color-text-secondary);
}
.table-wrap :deep(.score) {
  font-weight: var(--font-weight-semibold);
}
.table-wrap :deep(.action-cell) {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
</style>
