<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NModal, NSelect } from 'naive-ui';
import { useUniverseStore } from '@/stores/universe';
import { useWatchlistStore } from '@/stores/watchlist';
import ResearchModelScreener from './ResearchModelScreener.vue';
import StockDetail from '@/components/detail/StockDetail.vue';
import AnalysisDialog from '@/components/analysis/AnalysisDialog.vue';
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import { BOARD_LABELS, SELECTABLE_BOARDS, SOURCE_OPTIONS, formatAmount, formatPct, summarizeFilter, toFullSymbol } from '@/types/universe';
import type { Board, MarketFilter, PresetInfo, SnapshotRow } from '@/types/universe';
import type { WatchItem } from '@/types';
import type { SectorKind } from '@/types/sector';
const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();
const universe = useUniverseStore(); const watchlist = useWatchlistStore();
const actionError = ref(''); const adding = ref(''); const detail = ref<WatchItem | null>(null);
const analysisRow = ref<SnapshotRow | null>(null); const modelAnalysis = ref<{symbol:string;name:string}|null>(null); const added = ref(new Set<string>());
const filtersExpanded = ref(false); const ready = ref(false); const scrollEl = ref<HTMLElement | null>(null);
const busy = computed(() => universe.loading || universe.scoring);
const builtinPresets = computed(() => universe.presets.filter(p => p.builtin));
const savedOptions = computed(() => universe.savedConditions.map(p => ({ label: p.label, value: p.id })));
type SortKey = 'price' | 'change_pct' | 'amount' | 'turnover_rate' | 'total_market_cap' | 'score';
const sort = ref<{ key: SortKey; ascending: boolean } | null>(null);
function sortPage(key: SortKey) {
  if (key === 'score' && universe.isRanking) { universe.sortScores(); sort.value = null; return; }
  sort.value = { key, ascending: sort.value?.key === key ? !sort.value.ascending : false };
}
const displayedRows = computed(() => {
  const order = sort.value;
  if (!order) return universe.rows;
  return [...universe.rows].sort((a, b) => {
    const left = order.key === 'score' ? universe.rankableScore(a) : a[order.key];
    const right = order.key === 'score' ? universe.rankableScore(b) : b[order.key];
    const validLeft = typeof left === 'number' && Number.isFinite(left);
    const validRight = typeof right === 'number' && Number.isFinite(right);
    if (!validLeft || !validRight) return validLeft ? -1 : validRight ? 1 : a.code.localeCompare(b.code);
    return (order.ascending ? left - right : right - left) || a.code.localeCompare(b.code);
  });
});
type NumericKey = { [K in keyof MarketFilter]: MarketFilter[K] extends number | null ? K : never }[keyof MarketFilter];
const numericFields: Array<{ key: NumericKey; label: string }> = [
  { key: 'price_min', label: '价格下限（元）' }, { key: 'price_max', label: '价格上限（元）' },
  { key: 'amount_min_wan', label: '成交额下限（万元）' },
  { key: 'market_cap_min_yi', label: '总市值下限（亿元）' }, { key: 'market_cap_max_yi', label: '总市值上限（亿元）' },
  { key: 'turnover_min', label: '换手率下限（%）' }, { key: 'turnover_max', label: '换手率上限（%）' },
  { key: 'change_pct_min', label: '当日涨跌幅下限（%）' }, { key: 'change_pct_max', label: '当日涨跌幅上限（%）' },
];
const advancedFields: typeof numericFields = [
  { key: 'pe_min', label: '动态PE下限' }, { key: 'pe_max', label: '动态PE上限' },
  { key: 'pb_min', label: 'PB下限' }, { key: 'pb_max', label: 'PB上限' },
  { key: 'volume_ratio_min', label: '量比下限' },
  { key: 'change_60d_min', label: '60日涨跌幅下限（%）' }, { key: 'change_60d_max', label: '60日涨跌幅上限（%）' },
  { key: 'listed_days_min', label: '上市天数下限' }, { key: 'listed_days_max', label: '上市天数上限' },
  { key: 'amplitude_max', label: '当日振幅上限（%）' },
];
const sectorCatalog = ref<Array<{ kind: SectorKind; code: string; name: string }>>([]);
const catalogBusy = ref(false);
const catalogError = ref('');
const catalogOptions = (kind: SectorKind) => sectorCatalog.value.filter(row => row.kind === kind && /^BK\d+$/.test(row.code))
  .map(row => ({ label: row.name, value: row.code })).sort((a, b) => a.label.localeCompare(b.label, 'zh-CN'));
const industryOptions = computed(() => catalogOptions('industry'));
const conceptOptions = computed(() => catalogOptions('concept'));
async function loadCatalog() {
  if (catalogBusy.value || sectorCatalog.value.length) return;
  catalogBusy.value = true;
  catalogError.value = '';
  try { sectorCatalog.value = await invoke<typeof sectorCatalog.value>('get_sector_catalog'); }
  catch (e) { catalogError.value = `行业概念目录加载失败：${e}`; }
  finally { catalogBusy.value = false; }
}
function unsupported(key: NumericKey) {
  return key === 'volume_ratio_min' ? !universe.volumeRatioSupported :
    key.startsWith('change_60d') ? !universe.change60dSupported :
    key.startsWith('listed_days') ? !universe.listingDateSupported : false;
}
async function run(action: () => Promise<unknown>) {
  actionError.value = '';
  try { await action(); } catch (e) { actionError.value = String(e); }
}
function changeNumber(key: typeof numericFields[number]['key'], event: Event) {
  const text = (event.target as HTMLInputElement).value;
  universe.filter[key] = text === '' ? null : Number.isFinite(Number(text)) ? Number(text) : null;
}
function toggleBoard(board: Board, event: Event) {
  const checked = (event.target as HTMLInputElement).checked;
  universe.filter.boards = checked ? [...universe.filter.boards, board] : universe.filter.boards.filter(value => value !== board);
}
const number = (value: unknown, digits = 2) => typeof value === 'number' && Number.isFinite(value) ? value.toFixed(digits) : '--';
const symbol = (row: SnapshotRow) => toFullSymbol(row.code, row.board);
function isAdded(row: SnapshotRow) {
  return added.value.has(symbol(row)) || watchlist.items.some(item => item.market === 'CN' && item.code === symbol(row));
}
async function addStock(row: SnapshotRow) {
  if (adding.value || isAdded(row)) return;
  adding.value = symbol(row);
  await run(async () => {
    await watchlist.addStock(symbol(row), 'CN', row.name);
    added.value.add(symbol(row));
  });
  adding.value = '';
}
function openDetail(row: SnapshotRow) {
  detail.value = { id: -1, code: symbol(row), market: 'CN', name: row.name, sort_order: 0, added_at: '' };
}

function scoreState(row: SnapshotRow) {
  const item = universe.statuses[symbol(row)];
  if (!item) return 'not-scored';
  if (item.error || item.score === null) return 'failed';
  if (item.history?.stale) return 'stale';
  return universe.rankableScore(row) === null ? 'outdated' : 'scored';
}
function scoreNote(row: SnapshotRow) {
  const item = universe.statuses[symbol(row)];
  if (!item) return '尚未计算，点击“本页评分”或“全部量化评分排名”。';
  if (item.error) return item.error;
  return (item.history?.source_label || '来源未知') + (item.history?.warning ? ' · ' + item.history.warning : '') + ' · 截止 ' + (item.history?.end_date || '未知') +
    (scoreState(row) === 'scored' ? ' · 技术状态分，非胜率' : ' · 日期不同或数据陈旧，不参与同日排名');
}
const conditionEditor = ref(false); const conditionName = ref('');
const editingMode = ref<'save' | 'update' | 'rename'>('save'); const editingId = ref<string | undefined>();
const editorError = ref(''); const saving = ref(false); const deleteTarget = ref<PresetInfo | null>(null);
function editCondition(mode: 'save' | 'update' | 'rename') {
  editingMode.value = mode; editingId.value = mode === 'save' ? undefined : universe.selectedCondition?.id;
  conditionName.value = mode === 'save' ? '' : universe.selectedCondition?.label || '';
  editorError.value = ''; conditionEditor.value = true;
}
async function saveCondition() {
  if (saving.value) return;
  saving.value = true; editorError.value = '';
  try { await universe.saveCondition(conditionName.value, editingId.value, editingMode.value === 'rename'); conditionEditor.value = false; }
  catch (e) { editorError.value = String(e); } finally { saving.value = false; }
}
async function deleteCondition() {
  if (!deleteTarget.value || saving.value) return;
  saving.value = true; editorError.value = '';
  try { await universe.deleteCondition(deleteTarget.value.id); deleteTarget.value = null; }
  catch (e) { editorError.value = String(e); } finally { saving.value = false; }
}
async function query(force = false) { sort.value = null; await run(() => universe.search(force)); }
async function score(kind: 'page' | 'all') { sort.value = null; await run(() => universe.scoreResults(kind)); }
async function changePage(target: number) {
  await run(() => universe.fetchPage(target));
  scrollEl.value?.querySelector('.pool-results')?.scrollIntoView({ block: 'start' });
}
watch(filtersExpanded, value => { if (value) void loadCatalog(); });
let openGeneration = 0;
watch(() => props.show, async show => {
  const request = ++openGeneration;
  actionError.value = ''; detail.value = null; analysisRow.value = null; modelAnalysis.value=null;
  if (show) {
    ready.value = false; universe.clearScores(); sort.value = null;
    await run(() => universe.hydrate());
    if (request === openGeneration) ready.value = true;
  } else { universe.cancelScoring(); universe.flushPersist(); conditionEditor.value = false; deleteTarget.value = null; }
}, { immediate: true });
onBeforeUnmount(() => { ++openGeneration; universe.cancelScoring(); universe.flushPersist(); });
</script>

<template>
  <NModal :show="show" preset="card" :bordered="false" class="universe-screener-modal"
    :style="{ width: 'min(1300px, calc(100vw - 24px))', height: 'min(900px, calc(100dvh - 28px))', display: 'flex', flexDirection: 'column', overflow: 'hidden' }"
    :content-style="{ display: 'flex', flex: 1, flexDirection: 'column', minHeight: 0, padding: 0, overflow: 'hidden' }"
    @update:show="emit('update:show', $event)">
    <template #header><div class="screener-title"><span class="screener-mark"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M4 4h16v3l-6 6v5l-4 2v-7L4 7z" /></svg></span><div><h2>市场筛选与模型研究</h2><p>筛选股票池，再按需评分与解读</p></div></div></template>
    <div v-if="show" class="market-screener">
      <nav class="market-screener-nav" role="tablist" aria-label="市场筛选页面">
        <button id="pool-tab" type="button" role="tab" aria-controls="pool-panel" :aria-selected="universe.lastView === 'pool'"
          :class="{ active: universe.lastView === 'pool' }" :disabled="!ready" @click="run(() => universe.setView('pool'))">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M4 4h16v3l-6 6v5l-4 2v-7L4 7z" /></svg><span>市场筛选<small>条件 · 评分 · 个股分析</small></span><b v-if="universe.lastView === 'pool'">当前</b>
        </button>
        <button id="models-tab" type="button" role="tab" aria-controls="models-panel" :aria-selected="universe.lastView === 'models'"
          :class="{ active: universe.lastView === 'models' }" :disabled="!ready" @click="run(() => universe.setView('models'))">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M5 20V10m7 10V4m7 16v-7M3 20h18" /></svg><span>研究模型<small>模型选股 · 条件观察</small></span><b v-if="universe.lastView === 'models'">当前</b>
        </button>
      </nav>
      <div v-if="!ready" class="screener-loading" role="status">正在恢复上次页面与条件…</div>
      <template v-else>
        <div v-if="universe.lastView === 'pool'" class="pool-toolbar">
          <div class="pool-toolbar-summary"><strong>{{ universe.activePreset?.label || '已自定义' }}</strong><span :title="summarizeFilter(universe.filter)">{{ summarizeFilter(universe.filter) }}</span><small :class="{ changed: universe.conditionsChanged }">{{ universe.conditionsChanged ? '条件已调整 · 待查询' : universe.hasLoaded ? '已查询 · 可按需评分' : '待查询 · 选择条件后点击查询' }}</small></div>
          <div class="pool-toolbar-actions"><button class="primary pool-query" :disabled="busy" @click="query()">{{ universe.loading ? '查询中…' : '查询' }}</button><button :disabled="busy" @click="query(true)">刷新查询</button><button class="pool-filter-toggle" :aria-expanded="filtersExpanded" @click="filtersExpanded = !filtersExpanded">{{ filtersExpanded ? '收起条件' : '自定义条件' }}</button><HelpTooltip label="市场筛选使用说明">选择预设或修改条件后，点击查询才读取行情。页面、条件和保存的名称会记住；打开页面不会自动查询或调用AI。评分是技术状态分，模型验证请查看研究模型。</HelpTooltip></div>
        </div>
        <div ref="scrollEl" class="screener-scroll" tabindex="0" aria-label="市场筛选内容">
          <section v-if="universe.lastView === 'models'" id="models-panel" role="tabpanel" aria-labelledby="models-tab"><ResearchModelScreener @analyze="modelAnalysis=$event" @detail="detail={id:-1,code:$event.symbol,market:'CN',name:$event.name,sort_order:0,added_at:''}" /></section>
          <section v-else id="pool-panel" class="basic-pool" role="tabpanel" aria-labelledby="pool-tab">
            <div class="pool-section-label">常用条件 <small>只应用条件，点击查询后生效</small></div>
            <div class="pool-presets" aria-label="常用筛选预设">
              <button v-for="preset in builtinPresets" :key="preset.id" :class="{ active: universe.activePreset?.id === preset.id }" :disabled="busy" @click="run(() => universe.applyPreset(preset.id))"><strong>{{ preset.label }}</strong><span>{{ preset.description }}</span></button>
            </div>
            <div class="pool-saved"><label>我的条件</label><NSelect :value="universe.selectedCondition?.id || null" :options="savedOptions" :disabled="busy" placeholder="选择已保存条件" :style="{ width: 'min(260px, 100%)' }" @update:value="value => { if (value) run(() => universe.applyPreset(value)); }" />
              <button :disabled="busy" @click="editCondition('save')">另存为</button><template v-if="universe.selectedCondition"><button :disabled="busy" @click="editCondition('update')">覆盖条件</button><button :disabled="busy" @click="editCondition('rename')">改名</button><button class="subtle" :disabled="busy" @click="deleteTarget = universe.selectedCondition; editorError = ''">删除</button></template><button class="subtle" :disabled="busy" @click="run(() => universe.reset())">清空条件</button>
            </div>
            <p class="pool-current">当前条件：{{ universe.activePreset?.label || '已自定义' }} · {{ summarizeFilter(universe.filter) }}</p>
            <p v-if="universe.migrationNotice" class="pool-notice" role="status">{{ universe.migrationNotice }}</p><p v-if="universe.restoreError" class="pool-error" role="alert">{{ universe.restoreError }}</p>
            <fieldset v-show="filtersExpanded" class="pool-condition-panel" :disabled="busy">
              <legend>自定义条件 <small>留空表示不限</small></legend>
              <div class="pool-boards"><label v-for="board in SELECTABLE_BOARDS" :key="board"><input type="checkbox" :value="board" :checked="universe.filter.boards.includes(board)" :disabled="universe.filter.boards.length === 1 && universe.filter.boards.includes(board)" @change="toggleBoard(board, $event)">{{ BOARD_LABELS[board] }}</label><span class="hard-exclusions">ST / 退市 / 北交所 / B股固定排除</span></div>
              <div class="pool-sector-filters"><label>行业<NSelect v-model:value="universe.filter.industry_codes" multiple filterable clearable :options="industryOptions" :loading="catalogBusy" :disabled="busy || !universe.sectorSupported" placeholder="不限行业（多选为或）" /></label><label>概念<NSelect v-model:value="universe.filter.concept_codes" multiple filterable clearable :options="conceptOptions" :loading="catalogBusy" :disabled="busy || !universe.sectorSupported" placeholder="不限概念（与行业为且）" /></label></div>
              <p v-if="catalogError" class="pool-error">{{ catalogError }} <button @click="loadCatalog">重试目录</button></p>
              <div class="pool-filter-grid"><label v-for="field in numericFields" :key="field.key">{{ field.label }}<input type="number" step="any" :value="universe.filter[field.key] ?? ''" :disabled="unsupported(field.key)" :data-filter="field.key" @input="changeNumber(field.key, $event)" placeholder="不限"></label></div>
              <details class="pool-advanced"><summary>估值、量比等更多条件</summary><div class="pool-filter-grid"><label v-for="field in advancedFields" :key="field.key">{{ field.label }}<input type="number" step="any" :value="universe.filter[field.key] ?? ''" :disabled="unsupported(field.key)" :data-filter="field.key" @input="changeNumber(field.key, $event)" placeholder="不限"></label></div><label><input type="checkbox" v-model="universe.filter.exclude_cdr">排除CDR</label></details>
              <div class="pool-data-options"><label>行情通道<NSelect :value="universe.sourceMode" :options="SOURCE_OPTIONS" :disabled="busy" :style="{ width: '180px' }" @update:value="value => run(() => universe.setSourceMode(value))" /></label><label>每页<input type="number" min="1" max="100" :value="universe.pageSize" @change="event => run(() => universe.setPageSize(Number((event.target as HTMLInputElement).value)))"></label><span>调整后点击查询生效；停牌与一字板固定排除</span></div>
            </fieldset>
            <p v-if="actionError || universe.error" class="pool-error" role="alert">{{ actionError || universe.error }}</p><p v-if="universe.loading" role="status">正在读取基础行情…</p>
            <div v-if="!universe.hasLoaded && !universe.loading" class="pool-empty"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M4 4h16v3l-6 6v5l-4 2v-7L4 7z" /></svg><strong>先设置条件，再点击查询</strong><span>首次使用默认不限条件。可用预设快速缩小股票池，也可保存自己的条件。</span></div>
            <section v-if="universe.hasLoaded" class="pool-results">
              <div class="pool-result-head"><div><h3>{{ universe.isRanking ? '量化评分排名' : '筛选结果' }}<HelpTooltip label="评分排名说明">点击评分后才计算。量化分为现有技术因子的加权状态分，范围零到一百，不是上涨概率，也没有自动继承多年模型的盈利验证。全量排名覆盖当前命中股票；失败、陈旧或截止日不同的数据不参与同日分数比较。</HelpTooltip></h3><p>{{ universe.sourceLabel }} · 命中 {{ universe.resultCount }} 股 · {{ universe.stale ? '快照陈旧' : '行情快照' }}</p></div><div class="pool-score-actions"><button :disabled="busy || universe.conditionsChanged || universe.stale || !universe.rows.length" @click="score('page')">本页评分</button><button class="accent" :disabled="busy || universe.conditionsChanged || universe.stale || !universe.resultCount" @click="score('all')">全部量化评分排名</button></div></div>
              <p v-if="universe.conditionsChanged" class="pool-notice">条件已调整，下方仍为上次查询结果。点击查询后再评分。</p>
              <p v-if="universe.skippedConditions.length" class="pool-notice">本通道未执行：{{ universe.skippedConditions.join('、') }}。这些条件不构成有效筛选依据。</p><p v-if="!universe.sectorSupported" class="pool-notice">本行情通道未提供板块概念字段，表格显示“暂无”。已选行业概念条件的实际执行情况以上方提示为准。</p><p v-if="universe.historyNotice" class="pool-notice">{{ universe.historyNotice }} · 实际历史检查 {{ universe.historyEvaluated }} 股</p>
              <div v-if="universe.scoring" class="pool-score-progress" role="status"><span>{{ universe.scoreCancelled ? '正在停止，当前批次结束后不再继续' : '正在计算' }} · {{ universe.scoreDone }} / {{ universe.scoreTotal }} 股</span><progress :value="universe.scoreDone" :max="universe.scoreTotal || 1"></progress><button :disabled="universe.scoreCancelled" @click="universe.cancelScoring()">停止评分</button></div>
              <p v-if="universe.scoreError" class="pool-error" role="alert">{{ universe.scoreError }}</p><p v-if="universe.scoreDone || universe.scoreCancelled" class="pool-ranking-report">{{ universe.scoreCancelled ? '已停止，当前仅为部分评分' : universe.scoring ? '已完成部分评分' : universe.isRanking && universe.rankingSorted ? '全部评分完成 · 按同日技术分排名' : '手动评分完成' }} · 成功 {{ universe.scoreSucceeded }} · 失败 {{ universe.scoreFailed }} · {{ universe.isRanking ? '全结果' : '本页' }}可同日比较 {{ universe.scoreComparable }} · 截止 {{ universe.scoreAsOf || '未知' }}</p>
              <p class="pool-sort-note">{{ universe.isRanking && universe.rankingSorted ? '量化分全结果排序；其他列只排序当前页。' : '默认按成交额展示；点击列标题排序当前页。评分为空时不会计算。' }}</p>
              <div class="pool-table-scroll"><table><thead><tr><th>股票</th><th><button @click="sortPage('price')">最新价 ↕</button></th><th><button @click="sortPage('change_pct')">涨跌幅 ↕</button></th><th><button @click="sortPage('amount')">成交额 ↕</button></th><th><button @click="sortPage('turnover_rate')">换手率 ↕</button></th><th><button @click="sortPage('total_market_cap')">总市值 ↕</button></th><th>板块 / 概念</th><th><button :disabled="!universe.scoreDone || universe.scoring" @click="sortPage('score')">量化评分 ↕</button></th><th>操作</th></tr></thead>
                <tbody><tr v-for="row in displayedRows" :key="row.code"><td class="stock-cell"><b>{{ row.name }}</b><small>{{ row.code }} · {{ BOARD_LABELS[row.board] }}</small></td><td>{{ row.price > 0 ? number(row.price) : '--' }}</td><td :class="{ rising: row.change_pct > 0, falling: row.change_pct < 0 }">{{ formatPct(row.change_pct) }}</td><td>{{ formatAmount(row.amount) }}</td><td>{{ number(row.turnover_rate) }}%</td><td>{{ row.total_market_cap > 0 ? number(row.total_market_cap / 1e8) + '亿' : '--' }}</td>
                  <td class="sector-cell"><b>{{ row.industry || '暂无行业' }}</b><span :title="row.concepts?.join('、')">{{ row.concepts?.length ? row.concepts.slice(0, 2).join(' · ') + (row.concepts.length > 2 ? ' +' + (row.concepts.length - 2) : '') : '暂无概念' }}</span></td>
                  <td class="score-cell" :data-score-status="scoreState(row)" :title="scoreNote(row)"><b>{{ scoreState(row) === 'not-scored' ? '--' : scoreState(row) === 'failed' ? '失败' : number(universe.statuses[symbol(row)]?.score, 1) }}</b><small>{{ scoreState(row) === 'not-scored' ? '未评分' : scoreState(row) === 'stale' ? '数据陈旧' : scoreState(row) === 'outdated' ? '日期不同/未知' : scoreState(row) === 'failed' ? '悬停查看原因' : universe.statuses[symbol(row)]?.ready ? '规则已触发' : '规则待触发' }}</small></td>
                  <td class="row-actions"><button class="accent" @click="analysisRow = row">分析</button><button @click="openDetail(row)">走势</button><button :disabled="!!adding || isAdded(row)" @click="addStock(row)">{{ isAdded(row) ? '已自选' : '+自选' }}</button></td></tr></tbody>
              </table></div><p v-if="!universe.rows.length" class="pool-empty-result">没有符合本次条件的股票，调整后点击查询。</p>
            </section>
          </section>
        </div>
        <footer v-if="universe.lastView === 'pool'" class="pool-pagination"><span>{{ universe.hasLoaded ? '共 ' + universe.resultCount + ' 股 · 每页 ' + universe.appliedPageSize : '等待查询' }}</span><div><button :disabled="busy || !universe.hasLoaded || universe.page <= 1" @click="changePage(1)">首页</button><button :disabled="busy || !universe.hasLoaded || universe.page <= 1" @click="changePage(universe.page - 1)">上一页</button><b>{{ universe.hasLoaded ? universe.page : '--' }} / {{ universe.hasLoaded ? universe.pageCount : '--' }}</b><button :disabled="busy || !universe.hasLoaded || universe.page >= universe.pageCount" @click="changePage(universe.page + 1)">下一页</button><button :disabled="busy || !universe.hasLoaded || universe.page >= universe.pageCount" @click="changePage(universe.pageCount)">末页</button></div></footer>
      </template>
    </div>
  </NModal>
  <NModal :show="!!detail" preset="card" :title="detail?.name || '行情详情'" :style="{ width: 'min(1120px, 94vw)' }" @update:show="value => { if (!value) detail = null; }"><StockDetail v-if="detail" :item="detail" @close="detail = null" /></NModal>
  <AnalysisDialog v-if="analysisRow || modelAnalysis" :show="!!(analysisRow || modelAnalysis)" :symbol="analysisRow ? symbol(analysisRow) : modelAnalysis!.symbol" :name="analysisRow ? analysisRow.name : modelAnalysis!.name" rule="auto" @update:show="value => { if (!value) {analysisRow = null; modelAnalysis=null;} }" />
  <NModal v-model:show="conditionEditor" preset="card" class="condition-editor" :title="editingMode === 'rename' ? '修改条件名称' : editingMode === 'update' ? '覆盖已保存条件' : '保存为我的条件'" :style="{ width: 'min(460px, calc(100vw - 32px))' }"><form @submit.prevent="saveCondition"><label>条件名称<input v-model="conditionName" maxlength="32" autofocus placeholder="例如：放量温和上涨" :disabled="saving"></label><p>{{ editingMode === 'rename' ? '只修改名称，原来保存的条件保持不变。' : summarizeFilter(universe.filter) }}</p><p v-if="editorError" class="pool-error" role="alert">{{ editorError }}</p><div class="editor-actions"><button type="button" :disabled="saving" @click="conditionEditor = false">取消</button><button type="submit" class="primary" :disabled="saving || !conditionName.trim()">{{ saving ? '保存中…' : '保存' }}</button></div></form></NModal>
  <NModal :show="!!deleteTarget" preset="card" title="删除保存的条件" :style="{ width: 'min(420px, calc(100vw - 32px))' }" @update:show="value => { if (!value) deleteTarget = null; }"><p>删除“{{ deleteTarget?.label }}”？当前输入的条件仍可继续查询。</p><p v-if="editorError" class="pool-error" role="alert">{{ editorError }}</p><div class="editor-actions"><button :disabled="saving" @click="deleteTarget = null">取消</button><button :disabled="saving" class="danger" @click="deleteCondition">删除条件</button></div></NModal>
</template>

<style scoped>
:global(.universe-screener-modal.n-card) { border-radius: 16px; border: 1px solid var(--color-border-0); box-shadow: var(--shadow-md); }
:global(.universe-screener-modal.n-card > .n-card-header) { padding: 16px 22px 12px; flex-shrink: 0; }
:global(.universe-screener-modal.n-card > .n-card__content) { flex: 1; }
.screener-title { display: flex; align-items: center; gap: 12px; }.screener-title h2 { font-size: 18px; line-height: 1.4; margin: 0; }.screener-title p { font-size: 12px; font-weight: 400; color: var(--color-text-secondary); margin: 3px 0 0; }
.screener-mark { display: grid; place-items: center; width: 38px; height: 38px; border-radius: 11px; color: var(--color-accent); background: var(--color-accent-dim); }.screener-mark svg { width: 21px; height: 21px; }
.market-screener { display: flex; flex: 1; flex-direction: column; min-height: 0; min-width: 0; color: var(--color-text-primary); line-height: 1.6; }
.market-screener-nav { display: flex; gap: 8px; flex-shrink: 0; padding: 8px 22px 14px; border-bottom: 1px solid var(--color-border-0); }
.market-screener-nav button { display: flex; align-items: center; gap: 10px; min-width: 210px; padding: 10px 14px; text-align: left; border-radius: 10px; background: var(--color-surface-1); }
.market-screener-nav svg { width: 22px; height: 22px; flex: none; }.market-screener-nav span { font-weight: 650; }.market-screener-nav small { display: block; color: var(--color-text-secondary); font-size: var(--text-xs); font-weight: 400; }.market-screener-nav b { margin-left: auto; background: var(--color-accent); color: var(--color-accent-contrast); font-size: var(--text-xs); padding: 1px 6px; border-radius: 4px; }
.market-screener-nav button.active { color: var(--color-accent); border: 2px solid var(--color-accent); padding: 9px 13px; background: var(--color-accent-dim); box-shadow: inset 0 -3px 0 var(--color-accent); }
.screener-scroll { flex: 1; min-height: 0; min-width: 0; padding: 16px 22px; overflow: auto; overscroll-behavior: contain; scrollbar-gutter: stable; }.screener-scroll:focus { outline: none; }.screener-scroll:focus-visible { box-shadow: inset 0 0 0 2px var(--color-accent); }.screener-loading { padding: 30px; }
button { padding: 6px 11px; border: 1px solid var(--color-border-1); border-radius: 7px; color: inherit; background: var(--color-surface-1); font-size: 12px; line-height: 1.5; cursor: pointer; transition: background 120ms, border-color 120ms; }button:hover:not(:disabled) { background: var(--color-surface-hover); border-color: var(--color-accent); }button:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }button:disabled { opacity: .45; cursor: default; }.primary { color: var(--color-accent-contrast); background: var(--color-accent); border-color: var(--color-accent); font-weight: 650; }.primary:hover:not(:disabled) { background: var(--color-accent); filter: brightness(1.1); }.accent { border-color: var(--color-accent); color: var(--color-accent); background: var(--color-accent-dim); }.danger { color: var(--color-error); }.subtle { color: var(--color-text-secondary); }
.pool-toolbar { display: flex; gap: 12px; align-items: center; justify-content: space-between; padding: 12px 22px; background: var(--color-surface-1); border-bottom: 1px solid var(--color-border-0); flex-shrink: 0; }.pool-toolbar-summary { min-width: 0; flex: 1; display: grid; grid-template-columns: auto 1fr; gap: 3px 10px; }.pool-toolbar-summary strong { font-size: 13px; }.pool-toolbar-summary span { font-size: 12px; color: var(--color-text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.pool-toolbar-summary small { grid-column: 1 / -1; font-size: var(--text-xs); color: var(--color-text-secondary); }.pool-toolbar-summary .changed { color: var(--color-warning); }.pool-toolbar-actions { display: flex; align-items: center; gap: 8px; flex: none; }.pool-query { min-width: 76px; }
.pool-section-label { font-size: 13px; font-weight: 600; }.pool-section-label small { font-size: var(--text-xs); font-weight: 400; color: var(--color-text-secondary); margin-left: 8px; }.pool-presets { display: grid; grid-template-columns: repeat(4,minmax(0,1fr)); gap: 10px; margin: 10px 0 14px; }.pool-presets button { padding: 12px; text-align: left; border-radius: 9px; }.pool-presets strong,.pool-presets span { display: block; }.pool-presets span { font-size: var(--text-xs); color: var(--color-text-secondary); margin-top: 6px; }.pool-presets button.active { border-color: var(--color-accent); background: var(--color-accent-dim); }.pool-saved { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 12px; }.pool-current { font-size: var(--text-xs); color: var(--color-text-secondary); overflow-wrap: anywhere; margin: 8px 0 14px; }
.pool-condition-panel { border: 1px solid var(--color-border-0); border-radius: 10px; padding: 14px; margin: 14px 0; background: var(--color-surface-1); }.pool-condition-panel legend { padding: 0 6px; font-size: 13px; }.pool-condition-panel small { font-size: var(--text-xs); color: var(--color-text-secondary); }.pool-boards { display: flex; align-items: center; flex-wrap: wrap; gap: 14px; margin-bottom: 14px; font-size: 12px; }.hard-exclusions { color: var(--color-text-tertiary); font-size: var(--text-xs); }.pool-sector-filters { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-bottom: 14px; }.pool-filter-grid { display: grid; grid-template-columns: repeat(4,minmax(0,1fr)); gap: 12px; }.pool-filter-grid label,.pool-sector-filters label { font-size: 12px; }.pool-filter-grid input { display: block; width: 100%; margin-top: 5px; }input[type=number],.condition-editor input { padding: 7px 9px; border: 1px solid var(--color-border-1); border-radius: 6px; color: inherit; background: var(--color-surface-0); font: inherit; box-sizing: border-box; }input:focus-visible { outline: 2px solid var(--color-accent-dim); border-color: var(--color-accent); }.pool-advanced { margin: 14px 0; font-size: 12px; }.pool-advanced summary { cursor: pointer; color: var(--color-text-secondary); margin-bottom: 12px; }.pool-advanced .pool-filter-grid { margin-bottom: 12px; }.pool-data-options { display: flex; align-items: flex-end; flex-wrap: wrap; gap: 14px; border-top: 1px solid var(--color-border-0); padding-top: 12px; font-size: 12px; }.pool-data-options label { display: grid; gap: 4px; }.pool-data-options input { width: 75px; }.pool-data-options span { color: var(--color-text-secondary); }
.pool-error { color: var(--color-error); font-size: 12px; }.pool-notice { border-left: 3px solid var(--color-warning); padding: 8px 12px; background: var(--color-warning-bg); color: var(--color-text-secondary); font-size: 12px; }.pool-empty { min-height: 160px; padding: 30px 14px; display: flex; align-items: center; justify-content: center; gap: 9px; flex-direction: column; border: 1px dashed var(--color-border-1); border-radius: 12px; color: var(--color-text-secondary); font-size: 12px; }.pool-empty svg { width: 28px; height: 28px; color: var(--color-accent); }.pool-empty strong { font-size: 14px; color: var(--color-text-primary); }.pool-empty span { text-align: center; }
.pool-results { scroll-margin-top: 12px; }.pool-result-head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 10px; margin-top: 20px; }.pool-result-head h3 { margin: 0; font-size: 15px; display: flex; align-items: center; }.pool-result-head p { margin: 4px 0; font-size: var(--text-xs); color: var(--color-text-secondary); }.pool-score-actions { display: flex; gap: 8px; }.pool-sort-note,.pool-ranking-report { color: var(--color-text-secondary); font-size: var(--text-xs); }.pool-ranking-report { background: var(--color-accent-dim); border-radius: 6px; padding: 8px 12px; }.pool-score-progress { display: flex; align-items: center; gap: 12px; padding: 10px 0; font-size: 12px; }.pool-score-progress progress { height: 6px; accent-color: var(--color-accent); flex: 1; max-width: 300px; }
.pool-table-scroll { overflow: auto; border: 1px solid var(--color-border-0); border-radius: 9px; margin-top: 10px; }table { width: 100%; border-collapse: collapse; white-space: nowrap; font-size: 12px; }th,td { padding: 10px 9px; border-bottom: 1px solid var(--color-border-0); text-align: right; }th { background: var(--color-surface-1); color: var(--color-text-secondary); font-weight: 500; }th:first-child,th:nth-child(7) { text-align: left; }th button { border: 0; background: transparent; padding: 0; font-size: var(--text-xs); }tbody tr:hover { background: var(--color-accent-dim); }tbody tr:last-child td { border-bottom: 0; }.stock-cell { text-align: left; min-width: 135px; }.stock-cell b { display: block; font-weight: 550; }.stock-cell small,.score-cell small { display: block; color: var(--color-text-secondary); font-size: var(--text-xs); }.sector-cell { text-align: left; max-width: 180px; min-width: 150px; }.sector-cell b,.sector-cell span { display: block; overflow: hidden; text-overflow: ellipsis; max-width: 180px; font-weight: 400; }.sector-cell span { font-size: var(--text-xs); color: var(--color-text-secondary); margin-top: 3px; }.score-cell { min-width: 90px; }.score-cell b { color: var(--color-accent); font-size: 14px; }.score-cell[data-score-status=not-scored] b { color: var(--color-text-tertiary); }.score-cell[data-score-status=failed] b { color: var(--color-error); font-size: 12px; }.score-cell[data-score-status=stale] b,.score-cell[data-score-status=outdated] b { color: var(--color-text-secondary); }.row-actions { min-width: 185px; }.row-actions button { padding: 4px 8px; margin-left: 5px; font-size: var(--text-xs); }.rising { color: var(--color-up); }.falling { color: var(--color-down); }.pool-empty-result { text-align: center; color: var(--color-text-secondary); font-size: 12px; padding: 16px; }
.pool-pagination { display: flex; flex: none; align-items: center; justify-content: space-between; gap: 12px; padding: 12px 22px; border-top: 1px solid var(--color-border-0); background: var(--color-surface-1); font-size: var(--text-xs); color: var(--color-text-secondary); }.pool-pagination > div { display: flex; align-items: center; gap: 8px; }.pool-pagination b { color: var(--color-text-primary); padding: 0 6px; font-weight: 500; }.pool-pagination button { font-size: var(--text-xs); padding: 5px 10px; }
.condition-editor label { display: grid; gap: 7px; font-size: 13px; }.condition-editor input { width: 100%; }.condition-editor .pool-error { color: var(--color-error); }.condition-editor p { font-size: 12px; overflow-wrap: anywhere; color: var(--color-text-secondary); }.editor-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 18px; }
@media(max-width:1000px) { .pool-toolbar { align-items: flex-start; }.pool-toolbar-summary { grid-template-columns: 1fr; }.pool-toolbar-summary span { grid-column: 1; }.pool-presets { grid-template-columns: repeat(2,minmax(0,1fr)); }.pool-filter-grid { grid-template-columns: repeat(3,minmax(0,1fr)); } }
@media(max-width:680px) { :global(.universe-screener-modal.n-card > .n-card-header) { padding: 12px 14px 8px; }.screener-title h2 { font-size: 16px; }.market-screener-nav { padding: 6px 14px 10px; gap: 6px; }.market-screener-nav button { flex: 1; min-width: 0; gap: 6px; padding: 9px; }.market-screener-nav button.active { padding: 8px; }.market-screener-nav small { display: none; }.market-screener-nav svg { width: 18px; height: 18px; }.market-screener-nav b { font-size: var(--text-xs); padding: 0 4px; }.pool-toolbar { padding: 10px 14px; flex-wrap: wrap; }.pool-toolbar-summary { flex-basis: 100%; }.pool-toolbar-actions { width: 100%; }.pool-toolbar-summary span { max-width: 100%; }.screener-scroll { padding: 12px 14px; }.pool-filter-grid,.pool-sector-filters { grid-template-columns: repeat(2,minmax(0,1fr)); }.pool-pagination { padding: 10px 14px; flex-direction: column; gap: 6px; }.pool-pagination > div { gap: 4px; }.pool-pagination button { padding: 4px 7px; }.pool-score-progress { flex-wrap: wrap; }.pool-score-progress progress { min-width: 90px; }.pool-data-options { gap: 8px; } }
</style>
