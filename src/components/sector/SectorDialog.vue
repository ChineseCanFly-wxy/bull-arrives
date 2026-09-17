<script setup lang="ts">
import { computed, h, onBeforeUnmount, ref, watch } from 'vue';
import {
  NButton,
  NDataTable,
  NEmpty,
  NInput,
  NModal,
  NPagination,
  NSpin,
  NTabPane,
  NTabs,
  NTag,
  type DataTableColumns,
} from 'naive-ui';
import { useSectorStore } from '@/stores/sector';
import { useWatchlistStore } from '@/stores/watchlist';
import type { SectorKind, SectorMember, SectorSummary } from '@/types/sector';
import type { WatchItem } from '@/types';
import { formatPrice } from '@/utils/format';
import StockDetail from '@/components/detail/StockDetail.vue';
import AnalysisDialog from '@/components/analysis/AnalysisDialog.vue';

const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();
const sector = useSectorStore();
const watchlist = useWatchlistStore();
const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

let searchTimer: ReturnType<typeof setTimeout> | null = null;
watch(
  () => sector.keyword,
  () => {
    if (searchTimer) clearTimeout(searchTimer);
    sector.page = 1;
    searchTimer = setTimeout(() => void sector.fetchSummaries(), 300);
  },
);

watch(
  () => props.show,
  open => {
    if (open) {
      void sector.fetchSummaries();
    } else {
      sector.clearSector();
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (searchTimer) clearTimeout(searchTimer);
});

function changeClass(value: number | null): string {
  if (value == null || value === 0) return 'flat';
  return value > 0 ? 'up' : 'down';
}

function percent(value: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--';
  return `${value >= 0 ? '+' : ''}${value.toFixed(2)}%`;
}

function amount(value: number | null): string {
  if (value == null || !Number.isFinite(value) || value <= 0) return '--';
  if (value >= 100_000_000) return `${(value / 100_000_000).toFixed(2)}亿`;
  if (value >= 10_000) return `${(value / 10_000).toFixed(2)}万`;
  return value.toFixed(0);
}

function count(value: number | null): string {
  return value == null || !Number.isFinite(value) ? '--' : value.toFixed(0);
}

function rowKey(row: SectorSummary): string {
  return row.code;
}

function stockSymbol(row: SectorMember): string {
  return `${row.market}${row.code}`;
}

function memberKey(row: SectorMember): string {
  return stockSymbol(row);
}

const addedSymbols = computed(() => new Set(watchlist.items.map(item => item.code)));
const addingCode = ref<string | null>(null);
const addError = ref<string | null>(null);

const columns = computed<DataTableColumns<SectorSummary>>(() => [
  { title: '排名', key: 'rank', width: 62, align: 'right' },
  { title: '板块名称', key: 'name', minWidth: 150, ellipsis: { tooltip: true } },
  { title: '最新价', key: 'latest', width: 90, align: 'right', render: row => formatPrice(row.latest) },
  {
    title: '涨跌幅', key: 'change_pct', width: 92, align: 'right',
    sorter: (a, b) => (a.change_pct ?? -Infinity) - (b.change_pct ?? -Infinity),
    render: row => h('span', { class: changeClass(row.change_pct) }, percent(row.change_pct)),
  },
  {
    title: '成交额', key: 'amount', width: 100, align: 'right',
    sorter: (a, b) => (a.amount ?? -Infinity) - (b.amount ?? -Infinity),
    render: row => amount(row.amount),
  },
  {
    title: '换手率', key: 'turnover_rate', width: 88, align: 'right',
    sorter: (a, b) => (a.turnover_rate ?? -Infinity) - (b.turnover_rate ?? -Infinity),
    render: row => row.turnover_rate == null ? '--' : `${row.turnover_rate.toFixed(2)}%`,
  },
  { title: '上涨', key: 'up_count', width: 68, align: 'right', render: row => count(row.up_count) },
  { title: '下跌', key: 'down_count', width: 68, align: 'right', render: row => count(row.down_count) },
  {
    title: '领涨股票', key: 'leader_name', minWidth: 140, ellipsis: { tooltip: true },
    render: row => row.leader_name ?? '--',
  },
]);

const memberColumns = computed<DataTableColumns<SectorMember>>(() => [
  { title: '代码', key: 'code', width: 78, render: row => h('span', { class: 'mono' }, row.code) },
  { title: '名称', key: 'name', minWidth: 130, ellipsis: { tooltip: true } },
  { title: '最新价', key: 'price', width: 86, align: 'right', render: row => formatPrice(row.price) },
  {
    title: '涨跌幅', key: 'change_pct', width: 88, align: 'right',
    sorter: (a, b) => (a.change_pct ?? -Infinity) - (b.change_pct ?? -Infinity),
    render: row => h('span', { class: changeClass(row.change_pct) }, percent(row.change_pct)),
  },
  { title: '成交额', key: 'amount', width: 100, align: 'right', render: row => amount(row.amount) },
  {
    title: '换手率', key: 'turnover_rate', width: 86, align: 'right',
    render: row => row.turnover_rate == null ? '--' : `${row.turnover_rate.toFixed(2)}%`,
  },
  { title: '市盈率', key: 'pe', width: 78, align: 'right', render: row => row.pe == null ? '--' : row.pe.toFixed(2) },
  {
    // 三个按钮：加自选 / 详情（分时·K线 + 五档）/ 分析（评分 + 操作计划 + 价格位）
    title: '操作', key: 'action', width: 186, align: 'center',
    render: row => {
      const symbol = stockSymbol(row);
      const done = addedSymbols.value.has(symbol);
      return h('div', { class: 'action-cell' }, [
        h(
          NButton,
          {
            size: 'tiny',
            tertiary: true,
            type: done ? 'default' : 'primary',
            disabled: done || addingCode.value !== null,
            loading: addingCode.value === symbol,
            onClick: (event: MouseEvent) => {
              event.stopPropagation();
              void handleAdd(row);
            },
          },
          { default: () => (done ? '已添加' : '加自选') },
        ),
        h(
          NButton,
          {
            size: 'tiny',
            tertiary: true,
            onClick: (event: MouseEvent) => {
              event.stopPropagation();
              openMemberDetail(row);
            },
          },
          { default: () => '详情' },
        ),
        h(
          NButton,
          {
            size: 'tiny',
            tertiary: true,
            onClick: (event: MouseEvent) => {
              event.stopPropagation();
              openAnalysis(row);
            },
          },
          { default: () => '分析' },
        ),
      ]);
    },
  },
]);

async function refresh() {
  await sector.fetchSummaries(true);
}

function selectKind(value: string) {
  if (value === 'industry' || value === 'concept') void sector.selectKind(value as SectorKind);
}

async function handleAdd(row: SectorMember) {
  const symbol = stockSymbol(row);
  if (addedSymbols.value.has(symbol) || addingCode.value !== null) return;
  addingCode.value = symbol;
  addError.value = null;
  try {
    await watchlist.addStock(symbol, 'CN', row.name);
  } catch (e) {
    addError.value = `加入自选失败：${e}`;
  } finally {
    addingCode.value = null;
  }
}

/**
 * 「详情」= 在弹窗内切到第三个视图看个股详情（分时 / K 线 + 五档盘口）。
 *
 * ⚠️ 修过一个 bug：以前这里是 `visible.value = false`，把整个市场板块弹窗关掉，
 * 改用主窗口的详情面板显示 —— 代价是**市场板块页面连同已选板块、成分股列表一起消失**，
 * 用户得重新点工具栏图标、重新找到那个板块才能回来（用户反馈）。
 * 而且主窗口的详情面板本来就被这个模态挡着、根本看不见，"关掉再开"没解决任何问题。
 */
const detailMember = ref<SectorMember | null>(null);

function openMemberDetail(row: SectorMember) {
  detailMember.value = row;
}

function closeMemberDetail() {
  detailMember.value = null;
}

/** 内嵌的 StockDetail 要 `WatchItem` 形状；id 用 -1 表示"不在自选股里"（沿用主窗口的约定） */
const detailItem = computed<WatchItem | null>(() => {
  const row = detailMember.value;
  if (!row) return null;
  return {
    id: -1,
    code: stockSymbol(row),
    market: 'CN',
    name: row.name,
    sort_order: 0,
    added_at: '',
  };
});

// ── 个股技术分析 ──
// 与筛选器 / 推荐榜 / 自选股的「分析」走同一条路：不传规则，让后端按这只股票**当前状态**
// 自动匹配（筛选器结果表那种"固定用当前策略配套规则"是列表页横向比较才需要的口径）。
const showAnalysis = ref(false);
const analysisTarget = ref({ symbol: '', name: '' });

function openAnalysis(row: SectorMember) {
  analysisTarget.value = { symbol: stockSymbol(row), name: row.name };
  showAnalysis.value = true;
}
</script>

<template>
  <NModal
    v-model:show="visible"
    preset="card"
    title="市场板块"
    :style="{ width: 'min(980px, calc(100vw - 24px))' }"
    :content-style="{ maxHeight: 'calc(100vh - 150px)', overflow: 'auto' }"
    :bordered="false"
    size="small"
  >
    <div class="sector-dialog">
      <!-- 视图三：个股详情。内嵌在这里，而不是把弹窗关掉去用主窗口的详情面板 ——
           关掉的话市场板块页面（含已选板块与成分股列表）会一起消失，「返回成分股」就没得回了。 -->
      <template v-if="detailItem">
        <div class="detail-toolbar">
          <NButton text size="small" @click="closeMemberDetail">‹ 返回成分股</NButton>
          <span class="muted">{{ detailItem.name }} · {{ detailItem.code }}</span>
        </div>
        <div class="detail-host">
          <StockDetail :item="detailItem" @close="closeMemberDetail" />
        </div>
      </template>

      <template v-else-if="sector.selected">
        <div class="detail-toolbar">
          <NButton text size="small" @click="sector.clearSector">‹ 返回板块排行</NButton>
          <NButton size="small" :loading="sector.memberLoading" @click="sector.fetchMembers(true)">刷新成分股</NButton>
        </div>

        <div class="detail-heading">
          <div>
            <span class="detail-name">{{ sector.selected.name }}</span>
            <span class="muted mono">{{ sector.selected.code }}</span>
          </div>
          <span :class="changeClass(sector.selected.change_pct)">{{ percent(sector.selected.change_pct) }}</span>
        </div>
        <div class="summary-line">
          <span>成交额 {{ amount(sector.selected.amount) }}</span>
          <span>上涨 {{ count(sector.selected.up_count) }}</span>
          <span>下跌 {{ count(sector.selected.down_count) }}</span>
          <span v-if="sector.selected.leader_name">领涨 {{ sector.selected.leader_name }}</span>
        </div>

        <div class="meta-line">
          <span>共 {{ sector.memberTotal }} 只成分股</span>
          <NTag v-if="sector.memberStale" size="small" type="warning" :bordered="false">陈旧数据</NTag>
          <span class="muted">{{ sector.memberSource || '未获取' }} · {{ sector.memberAsOf || '--' }}</span>
        </div>
        <div v-if="sector.memberError" class="error-line" role="alert">
          <span>{{ sector.memberError }}</span>
          <NButton size="tiny" @click="sector.fetchMembers(true)">重试</NButton>
        </div>
        <div v-if="addError" class="error-line" role="alert">
          <span>{{ addError }}</span>
        </div>

        <NSpin :show="sector.memberLoading">
          <NDataTable
            :columns="memberColumns"
            :data="sector.members"
            :row-key="memberKey"
            :max-height="430"
            :scroll-x="900"
            :bordered="false"
            :single-line="true"
            size="small"
          />
          <NEmpty v-if="!sector.memberLoading && !sector.memberError && !sector.members.length" description="没有成分股数据" class="empty" />
        </NSpin>

        <div v-if="sector.memberPageCount > 1" class="pagination">
          <NPagination :page="sector.memberPage" :page-count="sector.memberPageCount" @update:page="sector.selectMemberPage" />
        </div>
      </template>

      <template v-else>
        <div class="toolbar">
          <NTabs type="segment" :value="sector.kind" @update:value="selectKind">
            <NTabPane name="industry" tab="行业板块" />
            <NTabPane name="concept" tab="概念板块" />
          </NTabs>
          <NInput v-model:value="sector.keyword" clearable placeholder="搜索板块名称或代码" class="search" />
          <NButton size="small" :loading="sector.loading" @click="refresh">刷新</NButton>
        </div>

        <div class="meta-line">
          <span>共 {{ sector.total }} 个板块</span>
          <NTag v-if="sector.stale" size="small" type="warning" :bordered="false">陈旧数据</NTag>
          <span class="muted">{{ sector.source || '未获取' }} · {{ sector.asOf || '--' }}</span>
        </div>

        <div v-if="sector.error" class="error-line" role="alert">
          <span>{{ sector.error }}</span>
          <NButton size="tiny" @click="refresh">重试</NButton>
        </div>

        <NSpin :show="sector.loading">
          <NDataTable
            :columns="columns"
            :data="sector.rows"
            :row-key="rowKey"
            :row-props="(row: SectorSummary) => ({ style: 'cursor: pointer', onClick: () => { void sector.selectSector(row); } })"
            :max-height="480"
            :scroll-x="820"
            :bordered="false"
            :single-line="true"
            size="small"
          />
          <NEmpty v-if="!sector.loading && !sector.error && !sector.rows.length" description="没有匹配的板块" class="empty" />
        </NSpin>

        <div v-if="sector.pageCount > 1" class="pagination">
          <NPagination :page="sector.page" :page-count="sector.pageCount" @update:page="sector.selectPage" />
        </div>
      </template>
    </div>
  </NModal>

  <!-- 分析弹窗放在市场板块模态之外：它是独立模态，挂在后面才会盖在板块弹窗之上 -->
  <AnalysisDialog
    v-model:show="showAnalysis"
    :symbol="analysisTarget.symbol"
    :name="analysisTarget.name"
  />
</template>

<style scoped>
.sector-dialog { display: flex; flex-direction: column; gap: 10px; }
.toolbar { display: flex; align-items: center; gap: 8px; }
.toolbar :deep(.n-tabs) { width: 180px; flex-shrink: 0; }
.search { flex: 1; min-width: 160px; }
.detail-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
/* 内嵌个股详情的宿主：StockDetail 自己是 `flex: 1 1 56%` 的纵向容器，
   必须有个**确定高度**的 flex 父级才撑得开 —— 否则它算成 0 高，图表区一片空白。
   高度用 vh 算，图表的 `.chart-container` 本身也是 vh 基准，两者对得上。 */
.detail-host {
  display: flex;
  flex-direction: column;
  height: calc(100vh - 220px);
  min-height: 360px;
  overflow: hidden;
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md);
  background: var(--color-surface-1);
}
/* StockDetail 自带顶部边框（它原本是主窗口里贴在表格下方的分隔线），内嵌时是多余的 */
.detail-host :deep(.stock-detail) { border-top: 0; }
.detail-heading { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
.detail-heading > div { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
.detail-name { font-size: var(--text-md); font-weight: 600; color: var(--color-text-primary); }
.summary-line { display: flex; flex-wrap: wrap; gap: 14px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.meta-line { display: flex; align-items: center; gap: 8px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.muted { color: var(--color-text-tertiary); }
.mono { font-variant-numeric: tabular-nums; }
.action-cell { display: inline-flex; align-items: center; justify-content: center; gap: 4px; }
.error-line { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--color-warning); font-size: var(--text-xs); }
.empty { padding: 36px 0; }
.pagination { display: flex; justify-content: flex-end; padding-top: 2px; }
:deep(.up) { color: var(--color-up); font-weight: 500; }
:deep(.down) { color: var(--color-down); font-weight: 500; }
:deep(.flat) { color: var(--color-text-secondary); }
@media (max-width: 620px) {
  .toolbar { flex-wrap: wrap; }
  .toolbar :deep(.n-tabs) { width: 100%; }
  .search { min-width: 0; }
}
</style>
