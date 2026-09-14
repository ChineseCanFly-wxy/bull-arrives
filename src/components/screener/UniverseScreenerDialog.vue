<script setup lang="ts">
// src/components/screener/UniverseScreenerDialog.vue
// 全市场筛选器（漏斗 L0 + L1）
//
// 设计说明：
// - 筛选规则与预设全部由 Rust 侧下发，前端只负责展示与微调
// - 快照在 Rust 侧带 60 秒缓存，重复点击「筛选」不会打爆数据源
// - 结果表支持本地排序；「加自选」会自动把 6 位代码转成 sh/sz/bj 前缀的完整符号

import { computed, h, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {
  NButton,
  NCheckbox,
  NCheckboxGroup,
  NDataTable,
  NInputNumber,
  NModal,
  NSelect,
  NTag,
  type DataTableColumns,
} from 'naive-ui';
import { useUniverseStore } from '@/stores/universe';
import { useWatchlistStore } from '@/stores/watchlist';
import AnalysisDialog from '@/components/analysis/AnalysisDialog.vue';
import RankDialog from '@/components/rank/RankDialog.vue';
import type { StockAnalysis } from '@/types/analysis';
import {
  BOARD_LABELS,
  SELECTABLE_BOARDS,
  SOURCE_OPTIONS,
  formatAmount,
  formatPct,
  toFullSymbol,
  toYi,
  type SnapshotRow,
  type UniverseResponse,
} from '@/types/universe';

const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();

const universe = useUniverseStore();
const watchlist = useWatchlistStore();

const addedSymbols = ref<Set<string>>(new Set());
const addError = ref<string | null>(null);

// 分析对话框状态
const showAnalysis = ref(false);
const analysisTarget = ref<{ symbol: string; name: string }>({ symbol: '', name: '' });

// 推荐榜对话框状态
const showRank = ref(false);

function openAnalysis(row: SnapshotRow) {
  analysisTarget.value = { symbol: toFullSymbol(row.code, row.board), name: row.name };
  showAnalysis.value = true;
}

const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

const SCORE_CONCURRENCY = 3;
const scoreByRow = ref<Map<string, number | null>>(new Map());
const scoreSortActive = ref(false);
const scoring = ref(false);
const scoreProgress = ref({ done: 0, total: 0 });
const scoreError = ref<string | null>(null);
const rankedRows = ref<SnapshotRow[] | null>(null);
const rankedPage = ref(1);
const rankedPageSize = ref(20);
let scoreRun = 0;

function scoreKey(row: SnapshotRow): string {
  return `${row.board}:${row.code}`;
}

function scoreOf(row: SnapshotRow): number {
  return scoreByRow.value.get(scoreKey(row)) ?? -1;
}

const globalScoreActive = computed(() => rankedRows.value !== null);

const sortedRows = computed(() => {
  const rows = [...(rankedRows.value ?? universe.rows)];
  if (!scoreSortActive.value) return rows;
  return rows.sort((a, b) => scoreOf(b) - scoreOf(a) || a.code.localeCompare(b.code));
});

const displayedRows = computed(() => {
  const rows = sortedRows.value;
  if (!globalScoreActive.value) return rows;
  const start = (rankedPage.value - 1) * rankedPageSize.value;
  return rows.slice(start, start + rankedPageSize.value);
});

watch(() => universe.rows, () => {
  scoreRun += 1;
  scoreByRow.value = new Map();
  scoreSortActive.value = false;
  scoring.value = false;
  scoreError.value = null;
  rankedRows.value = null;
  rankedPage.value = 1;
});

async function scoreAllAndSort() {
  if (!universe.totalMatched || scoring.value) return;

  const run = ++scoreRun;
  const next = new Map<string, number | null>();
  scoring.value = true;
  scoreSortActive.value = false;
  scoreError.value = null;
  scoreByRow.value = new Map();
  rankedRows.value = null;
  scoreProgress.value = { done: 0, total: universe.totalMatched };

  try {
    // 仅在用户主动要求全量评分时绕过默认 500 行截断；复用当前快照保证结果一致。
    const response = await invoke<UniverseResponse>('get_market_universe', {
      filter: universe.filter,
      limit: universe.totalMatched,
      reuseSnapshot: true,
      source: universe.sourceMode,
    });
    if (run !== scoreRun) return;

    const rows = response.rows;
    rankedRows.value = rows;
    rankedPage.value = 1;
    rankedPageSize.value = universe.pageSize;
    scoreProgress.value = { done: 0, total: rows.length };

    const scoreRow = async (row: SnapshotRow) => {
      try {
        const analysis = await invoke<StockAnalysis>('analyze_stock', { symbol: toFullSymbol(row.code, row.board) });
        next.set(scoreKey(row), analysis.total_score);
      } catch (error) {
        console.warn(`[universe] 量化评分失败 ${row.code}:`, error);
        next.set(scoreKey(row), null);
      } finally {
        if (run !== scoreRun) return;
        scoreByRow.value = new Map(next);
        scoreProgress.value = { ...scoreProgress.value, done: scoreProgress.value.done + 1 };
      }
    };

    try {
      for (let start = 0; start < rows.length; start += SCORE_CONCURRENCY) {
        if (run !== scoreRun) return;
        await Promise.all(rows.slice(start, start + SCORE_CONCURRENCY).map(scoreRow));
      }
    } catch (error) {
      if (run === scoreRun) scoreError.value = `全量评分失败：${error}`;
    }
  } catch (error) {
    if (run === scoreRun) scoreError.value = `读取全部筛选结果失败：${error}`;
  } finally {
    if (run === scoreRun) {
      scoreSortActive.value = rankedRows.value !== null;
      scoring.value = false;
    }
  }
}

// ── 结果表分页 ──
// 普通筛选走服务端分页；全量评分后改为已排序结果的本地分页。
const pagination = computed(() => {
  const global = globalScoreActive.value;
  const itemCount = global ? rankedRows.value!.length : universe.totalMatched;
  return {
    page: global ? rankedPage.value : universe.page,
    pageSize: global ? rankedPageSize.value : universe.pageSize,
    pageSizes: [20, 50, 100],
    showSizePicker: true,
    itemCount,
    prefix: (info: { startIndex: number; endIndex: number; itemCount?: number }) =>
      `第 ${info.startIndex + 1}–${info.endIndex + 1} 条 · 共 ${info.itemCount ?? itemCount} 条`,
    onChange: (value: number) => {
      if (global) rankedPage.value = value;
      else void universe.fetchPage(value);
    },
    onPageSizeChange: (size: number) => {
      if (global) {
        rankedPageSize.value = size;
        rankedPage.value = 1;
      } else {
        void universe.setPageSize(size);
      }
    },
  };
});

// 每次打开都先恢复上次的条件；首次（或缓存已失效）时再自动拉一次
/**
 * 初始化时机（⚠️ 关键，踩过坑）：
 *
 * 父组件是 `<UniverseScreenerDialog v-if="showScreener" v-model:show="showScreener" />` ——
 * 组件在**挂载时 `props.show` 就已经是 true**，而 `watch(() => props.show)` 不加
 * `immediate` 永远不会为初始值触发。之前初始化就因此从未执行过：
 * 预设不加载、上次的板块/排除不恢复，表现为「进去什么都没有、怎么改都不生效」。
 *
 * 所以这里必须用 onMounted / onBeforeUnmount，而不是 watch props.show。
 */
onMounted(() => {
  void universe.hydrate();
});

onBeforeUnmount(() => {
  // 关闭对话框时立即落盘：防抖的 400ms 可能在用户直接退出程序时被带走
  universe.flushPersist();
});

const activeDesc = computed(
  () =>
    universe.presets.find(p => p.id === universe.activePresetId)?.description ??
    '自定义条件：不套用预设，完全按下方勾选与数值区间筛选',
);

/** 涨跌配色：A 股习惯 —— 红涨绿跌 */
function changeClass(value: number): string {
  if (value > 0) return 'up';
  if (value < 0) return 'down';
  return 'flat';
}

function num(value: number, digits = 2): string {
  return Number.isFinite(value) ? value.toFixed(digits) : '-';
}

// 虚拟滚动要求提供 row-key；模板里不能写 TS 类型注解，所以放在 script 中
function rowKey(row: SnapshotRow): string {
  return row.code;
}

async function handleAdd(row: SnapshotRow) {
  const symbol = toFullSymbol(row.code, row.board);
  addError.value = null;
  try {
    await watchlist.addStock(symbol, 'CN', row.name);
    addedSymbols.value = new Set(addedSymbols.value).add(symbol);
  } catch (e) {
    addError.value = `加自选失败：${e}`;
  }
}

const columns = computed<DataTableColumns<SnapshotRow>>(() => [
  {
    title: '代码',
    key: 'code',
    width: 76,
    sorter: 'default',
    render: row => h('span', { class: 'mono' }, row.code),
  },
  {
    title: '名称',
    key: 'name',
    width: 104,
    ellipsis: { tooltip: true },
    render: row =>
      row.is_st
        ? h('span', { class: 'name-cell' }, [
            row.name,
            h('span', { class: 'flag flag-st' }, 'ST'),
          ])
        : h('span', { class: 'name-cell' }, row.name),
  },
  {
    title: '最新价',
    key: 'price',
    width: 78,
    align: 'right',
    sorter: 'default',
    render: row => h('span', { class: 'num mono' }, num(row.price)),
  },
  {
    title: '涨跌幅',
    key: 'change_pct',
    width: 84,
    align: 'right',
    sorter: 'default',
    render: row =>
      h('span', { class: ['num mono', changeClass(row.change_pct)] }, formatPct(row.change_pct)),
  },
  {
    title: scoreSortActive.value ? '评分 ↓' : '评分',
    key: 'quant_score',
    width: 76,
    align: 'right',
    render: row => {
      const score = scoreByRow.value.get(scoreKey(row));
      return h('span', { class: ['num mono', score != null ? changeClass(score - 50) : 'muted'] }, score == null ? '--' : score.toFixed(1));
    },
  },
  {
    title: '量比',
    key: 'volume_ratio',
    width: 66,
    align: 'right',
    sorter: 'default',
    render: row => h('span', { class: 'num mono' }, num(row.volume_ratio)),
  },
  {
    title: '换手率',
    key: 'turnover_rate',
    width: 76,
    align: 'right',
    sorter: 'default',
    render: row => h('span', { class: 'num mono' }, `${num(row.turnover_rate)}%`),
  },
  {
    title: '振幅',
    key: 'amplitude_pct',
    width: 68,
    align: 'right',
    sorter: 'default',
    render: row => h('span', { class: 'num mono' }, `${num(row.amplitude_pct)}%`),
  },
  {
    title: '成交额',
    key: 'amount',
    width: 86,
    align: 'right',
    sorter: 'default',
    render: row => h('span', { class: 'num mono' }, formatAmount(row.amount)),
  },
  {
    title: '总市值',
    key: 'total_market_cap',
    width: 92,
    align: 'right',
    sorter: 'default',
    render: row => h('span', { class: 'num mono' }, `${num(toYi(row.total_market_cap), 1)}亿`),
  },
  {
    title: '板块',
    key: 'board',
    width: 86,
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
          {
            size: 'tiny',
            tertiary: true,
            type: done ? 'default' : 'primary',
            disabled: done,
            onClick: () => void handleAdd(row),
          },
          { default: () => (done ? '已添加' : '加自选') },
        ),
        h(
          NButton,
          {
            size: 'tiny',
            tertiary: true,
            onClick: () => openAnalysis(row),
          },
          { default: () => '分析' },
        ),
      ]);
    },
  },
]);
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="全市场筛选器"
    :style="{ width: 'min(1120px, calc(100vw - 24px))' }"
    :content-style="{ maxHeight: 'calc(100vh - 120px)', overflow: 'auto' }"
    :bordered="false"
    size="small"
  >
    <div class="screener">
      <!-- 预设方案 -->
      <div class="preset-bar">
        <div class="chips">
          <button
            v-for="preset in universe.presets"
            :key="preset.id"
            class="chip"
            :class="{ active: universe.activePresetId === preset.id }"
            :title="preset.description"
            @click="universe.applyPreset(preset.id)"
          >
            {{ preset.label }}
          </button>
          <button
            class="chip"
            :class="{ active: universe.activePresetId === 'custom' }"
            title="保留当前条件，仅把手动微调标记为自定义"
            @click="universe.applyPreset('custom')"
          >
            自定义
          </button>
          <button
            v-if="universe.presetsError"
            class="chip chip-error"
            :title="`加载失败：${universe.presetsError}`"
            :disabled="universe.presetsLoading"
            @click="universe.retryPresets"
          >
            {{ universe.presetsLoading ? '预设加载中…' : '预设加载失败，点击重试' }}
          </button>
          <button
            v-if="universe.restoreError"
            class="chip chip-error"
            :title="`恢复失败：${universe.restoreError}`"
            @click="universe.hydrate"
          >
            上次条件恢复失败，点击重试
          </button>
        </div>
        <span class="preset-desc">{{ activeDesc }}</span>
      </div>

      <!-- 筛选条件 -->
      <div class="filter-panel">
        <div class="field">
          <span class="field-label">板块</span>
          <n-checkbox-group v-model:value="universe.filter.boards" @update:value="universe.markCustom">
            <n-checkbox v-for="b in SELECTABLE_BOARDS" :key="b" :value="b" :label="BOARD_LABELS[b]" />
          </n-checkbox-group>
        </div>

        <div class="field">
          <span class="field-label">排除</span>
          <div class="checks">
            <n-checkbox v-model:checked="universe.filter.exclude_st" @update:checked="universe.markCustom">ST</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.exclude_delisting" @update:checked="universe.markCustom">退市</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.exclude_suspended" @update:checked="universe.markCustom">停牌</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.exclude_limit_locked" @update:checked="universe.markCustom">一字板</n-checkbox>
          </div>
        </div>

        <div class="ranges">
          <div class="range">
            <span class="range-label">价格(元)</span>
            <n-input-number v-model:value="universe.filter.price_min" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.price_max" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label">市值(亿)</span>
            <n-input-number v-model:value="universe.filter.market_cap_min_yi" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.market_cap_max_yi" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label">换手率(%)</span>
            <n-input-number v-model:value="universe.filter.turnover_min" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.turnover_max" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label">涨跌幅(%)</span>
            <n-input-number v-model:value="universe.filter.change_pct_min" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.change_pct_max" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label">量比 ≥</span>
            <n-input-number
              v-model:value="universe.filter.volume_ratio_min"
              size="small"
              :show-button="false"
              :disabled="!universe.volumeRatioSupported"
              :placeholder="universe.volumeRatioSupported ? '不限' : '数据源不支持'"
              clearable
              @update:value="universe.markCustom"
            />
          </div>

          <div class="range">
            <span class="range-label">成交额 ≥(万)</span>
            <n-input-number v-model:value="universe.filter.amount_min_wan" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>
        </div>
      </div>

      <!-- 工具条 -->
      <div class="toolbar">
        <n-select
          :value="universe.sourceMode"
          :options="SOURCE_OPTIONS"
          size="small"
          class="source-select"
          :disabled="universe.loading"
          @update:value="universe.setSourceMode"
        />
        <n-button type="primary" size="small" :loading="universe.loading" @click="universe.search(false)">
          开始筛选
        </n-button>
        <n-button size="small" :disabled="universe.loading" @click="universe.search(true)">
          强制刷新
        </n-button>
        <n-button size="small" quaternary :disabled="universe.loading" @click="universe.reset()">
          重置条件
        </n-button>
        <n-button size="small" type="warning" secondary :disabled="universe.loading" @click="showRank = true">
          生成推荐榜
        </n-button>

        <span v-if="universe.hasLoaded" class="stats">
          全市场 <b>{{ universe.totalAll }}</b> 只 → 命中
          <b>{{ universe.totalMatched }}</b> 只
          <span class="muted">（{{ universe.sourceLabel }}）</span>
        </span>

        <!-- 筛完先只给统计，用户点了才把表格放出来（带命中数，一眼可见） -->
        <n-button
          v-if="universe.hasLoaded && universe.totalMatched > 0"
          size="small"
          type="primary"
          :loading="universe.pageLoading"
          @click="universe.toggleResults"
        >
          {{ universe.resultsVisible ? '收起明细' : `展示数据（${universe.totalMatched} 只）` }}
        </n-button>
        <n-button
          v-if="universe.resultsVisible && universe.totalMatched > 0"
          size="small"
          secondary
          :loading="scoring"
          @click="scoreAllAndSort"
        >
          {{ scoring ? `评分 ${scoreProgress.done}/${scoreProgress.total}` : globalScoreActive ? '重新全量评分排序' : '全部量化评分排序' }}
        </n-button>
        <n-tag v-if="globalScoreActive && !scoring" type="success" size="small" :bordered="false">
          全部 {{ rankedRows?.length }} 只已按评分分页
        </n-tag>

        <n-tag v-if="universe.stale" type="warning" size="small" :bordered="false">
          刷新失败，显示的是旧数据
        </n-tag>
        <span v-if="addError" class="add-error">{{ addError }}</span>
      </div>

      <!-- 数据源能力提示：条件被自动忽略时必须说清楚，否则用户只会看到 0 结果 -->
      <div v-if="universe.hasLoaded && universe.skippedConditions.length" class="notice-line">
        当前数据源（{{ universe.sourceLabel }}）不提供「{{ universe.skippedConditions.join('、') }}」，
        该条件已自动忽略。需要它请把左上角数据源切到「东方财富」。
      </div>

      <div v-if="universe.error" class="error-line">{{ universe.error }}</div>
      <div v-if="scoreError" class="error-line">{{ scoreError }}</div>

      <!-- 结果表：筛完先只显示统计，点「展示数据」才出现 -->
      <div v-if="universe.resultsVisible" class="table-wrap">
        <!--
          这里刻意**不用 virtual-scroll**：虚拟滚动要求外层容器有确定高度，
          一旦布局计算出 0 高度，表格会渲染成一片空白 —— 表现就是「点了筛选什么都没有」。
          也不用 flex-height：同属"依赖外层高度"的方案（曾导致行不渲染）。
          服务端分页每页最多 100 行，用固定 max-height + 内部滚动最稳。
        -->
        <n-data-table
          :columns="columns"
          :data="displayedRows"
          :loading="universe.loading || universe.pageLoading"
          :row-key="rowKey"
          size="small"
          :remote="true"
          :max-height="'min(420px, 40vh)'"
          :scroll-x="1128"
          :pagination="pagination"
        />
        <div
          v-if="!universe.loading && !universe.pageLoading && !universe.rows.length && !universe.error"
          class="empty-line"
        >
          没有股票命中当前条件。可以放宽数值区间，或点上方「重置条件」回到默认。
        </div>
      </div>
      <div v-else-if="universe.hasLoaded && universe.totalMatched > 0" class="preview-line">
        已按当前条件匹配 <b>{{ universe.totalMatched }}</b> 只 · 点上方「展示数据」查看明细
      </div>
      <div v-else-if="!universe.loading" class="preview-line">
        点「开始筛选」从全市场匹配股票 —— 先显示命中数量，需要时再展开明细
      </div>
    </div>
  </n-modal>

  <AnalysisDialog
    v-model:show="showAnalysis"
    :symbol="analysisTarget.symbol"
    :name="analysisTarget.name"
  />

  <RankDialog v-model:show="showRank" :filter="universe.filter" />
</template>

<style scoped>
.screener {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: var(--space-2);
}

/* ── 预设条 ── */
.preset-bar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}
.chips {
  display: inline-flex;
  gap: var(--space-1);
  flex-wrap: wrap;
}
.chip {
  padding: 3px 10px;
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
  font-family: var(--font-sans);
  cursor: pointer;
  transition: background var(--transition-fast), color var(--transition-fast),
    border-color var(--transition-fast);
}
.chip:hover {
  color: var(--color-accent);
  border-color: var(--color-accent-dim);
}
.chip.active {
  background: var(--color-accent-dim);
  border-color: var(--color-accent);
  color: var(--color-accent);
}
.preset-desc {
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}

/* ── 筛选面板 ── */
.filter-panel {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md);
  background: var(--color-bg-card);
}
.field {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}
.field-label {
  flex-shrink: 0;
  width: 40px;
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}
.checks {
  display: inline-flex;
  gap: var(--space-4);
  flex-wrap: wrap;
}
.ranges {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: var(--space-2) var(--space-4);
}
.range {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
.range-label {
  flex-shrink: 0;
  width: 78px;
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}
.range :deep(.n-input-number) {
  width: 74px;
}
.tilde {
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
}

/* ── 工具条 ── */
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
  font-weight: var(--font-weight-semibold);
}
.muted {
  color: var(--color-text-tertiary);
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
.source-select {
  width: 136px;
}
.chip-error {
  border-color: var(--color-warning);
  color: var(--color-warning);
}
.chip-error:hover {
  background: var(--color-warning-bg);
}
.empty-line {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  min-height: 120px;
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  text-align: center;
}
.preview-line {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 120px;
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
}
.preview-line b {
  color: var(--color-text-primary);
  margin: 0 2px;
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

/* ── 结果表 ── */
.table-wrap {
  flex: 1;
  min-height: 0;
  min-width: 0;
}
.table-wrap :deep(.mono) {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.table-wrap :deep(.num) {
  display: inline-block;
  width: 100%;
  text-align: right;
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
.table-wrap :deep(.name-cell) {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.table-wrap :deep(.flag) {
  flex-shrink: 0;
  padding: 0 3px;
  border-radius: var(--radius-sm);
  font-size: 10px;
  line-height: 14px;
}
.table-wrap :deep(.flag-st) {
  background: var(--color-warning-bg);
  color: var(--color-warning);
  border: 1px solid var(--color-warning-border);
}
.table-wrap :deep(.action-cell) {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

@media (max-width: 560px) {
  .ranges {
    grid-template-columns: minmax(0, 1fr);
  }
  .preset-desc,
  .range-label {
    width: 100%;
  }
  .range {
    flex-wrap: wrap;
  }
  .range :deep(.n-input-number) {
    flex: 1 1 0;
    min-width: 0;
    width: 0;
  }
}
</style>
