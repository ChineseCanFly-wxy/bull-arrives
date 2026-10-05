// Manual quote conditions and explicit scoring. Frozen models use their own view.
import { defineStore } from 'pinia';
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { createDefaultFilter, SELECTABLE_BOARDS, toFullSymbol } from '@/types/universe';
import type { MarketFilter, PresetInfo, SnapshotRow, SnapshotSource, UniverseResponse } from '@/types/universe';
import type { StockStatusItem } from '@/types/analysis';

export const PAGE_SIZE_MIN = 1;
export const PAGE_SIZE_MAX = 100;
export const PAGE_SIZE_DEFAULT = 20;
export type SourceMode = 'auto' | SnapshotSource;
export type ScreenerView = 'models' | 'pool';
const MANUAL_SCHEMA = '3';
const CONDITIONS_KEY = 'universe_quote_conditions';
const SCORE_BATCH_SIZE = 20;
const BASIC_PRESET: PresetInfo = {
  id: 'all', label: '不限条件', description: '沪深普通股，不限制价格、市值或涨跌幅',
  filter: createDefaultFilter(), rule: '', builtin: true,
  strategy_version_id: null, strategy_version: 0, strategy_status: 'quote_tool',
};
const NUMERIC_FIELDS = [
  'price_min', 'price_max', 'market_cap_min_yi', 'market_cap_max_yi',
  'turnover_min', 'turnover_max', 'change_pct_min', 'change_pct_max',
  'amount_min_wan', 'pe_min', 'pe_max', 'pb_min', 'pb_max',
  'listed_days_min', 'listed_days_max', 'volume_ratio_min', 'change_60d_min', 'change_60d_max',
  'amplitude_max',
] as const;
function manualFilter(value: unknown): MarketFilter {
  const result = createDefaultFilter();
  if (!value || typeof value !== 'object') return result;
  const input = value as Record<string, unknown>;
  const boards = Array.isArray(input.boards)
    ? SELECTABLE_BOARDS.filter(board => (input.boards as unknown[]).includes(board)) : [];
  result.boards = boards.length ? boards : [...SELECTABLE_BOARDS];
  for (const key of NUMERIC_FIELDS) {
    const number = input[key];
    result[key] = typeof number === 'number' && Number.isFinite(number) ? number : null;
  }
  if (typeof input.exclude_cdr === 'boolean') result.exclude_cdr = input.exclude_cdr;
  for (const key of ['industry_codes', 'concept_codes'] as const) {
    if (Array.isArray(input[key])) result[key] = [...new Set(input[key].filter((code): code is string =>
      typeof code === 'string' && /^BK\d+$/.test(code)))].sort();
  }
  result.exclude_st = true; result.exclude_delisting = true;
  result.exclude_suspended = true; result.exclude_limit_locked = true;
  return result;
}
function validateRanges(filter: MarketFilter) {
  const pairs: Array<[typeof NUMERIC_FIELDS[number], typeof NUMERIC_FIELDS[number], string]> = [
    ['price_min', 'price_max', '价格'], ['market_cap_min_yi', 'market_cap_max_yi', '市值'],
    ['turnover_min', 'turnover_max', '换手率'], ['change_pct_min', 'change_pct_max', '涨跌幅'],
    ['pe_min', 'pe_max', 'PE'], ['pb_min', 'pb_max', 'PB'],
    ['listed_days_min', 'listed_days_max', '上市天数'], ['change_60d_min', 'change_60d_max', '60日涨跌幅'],
  ];
  for (const [minimum, maximum, label] of pairs) {
    const low = filter[minimum], high = filter[maximum];
    if (low !== null && high !== null && low > high) throw new Error(label + '下限不能大于上限，请调整后查询。');
  }
}
const signature = (value: MarketFilter) => JSON.stringify(manualFilter(value));
const eligibleRow = (row: SnapshotRow) => SELECTABLE_BOARDS.includes(row.board) &&
  !row.is_st && !row.is_delisting && !/ST|退/i.test(row.name);
const dateKey = (date: string | null | undefined) => typeof date === 'string' && /^\d{4}-?\d{2}-?\d{2}$/.test(date)
  ? date.replaceAll('-', '') : '';
function failedStatus(symbol: string, error: string): StockStatusItem {
  return { symbol, score: null, ready: null, waiting_for: null, buy_low: null, buy_high: null,
    stop_loss: null, take_profit: null, risk_reward: null, error, history: null };
}
export const useUniverseStore = defineStore('universe', () => {
  const builtins = ref<PresetInfo[]>([BASIC_PRESET]);
  const savedConditions = ref<PresetInfo[]>([]);
  const presets = computed(() => [...builtins.value, ...savedConditions.value]);
  const filter = ref<MarketFilter>(createDefaultFilter());
  const selectedConditionId = ref('all');
  const lastView = ref<ScreenerView>('pool');
  const migrationNotice = ref(''); const restoreError = ref('');
  const quoteRows = ref<SnapshotRow[]>([]);
  const totalAll = ref(0); const totalMatched = ref(0); const stale = ref(false);
  const hasLoaded = ref(false); const sourceLabel = ref('');
  const skippedConditions = ref<string[]>([]);
  const volumeRatioSupported = ref(true); const change60dSupported = ref(true);
  const listingDateSupported = ref(true); const sectorSupported = ref(true);
  const historyNotice = ref<string | null>(null); const historyEvaluated = ref(0);
  const sourceMode = ref<SourceMode>('auto'); const pageSize = ref(PAGE_SIZE_DEFAULT);
  const page = ref(1); const quotePage = ref(1);
  const appliedFilter = ref<MarketFilter | null>(null);
  const appliedSource = ref<SourceMode>('auto'); const appliedPageSize = ref(PAGE_SIZE_DEFAULT);
  const loading = ref(false); const error = ref('');
  const statuses = ref<Record<string, StockStatusItem>>({});
  const rankingRows = ref<SnapshotRow[] | null>(null);
  const scoring = ref(false); const scoringKind = ref<'page' | 'all' | null>(null);
  const scoreTotal = ref(0); const scoreDone = ref(0); const scoreCancelled = ref(false);
  const scoreError = ref(''); const rankingSorted = ref(false); const scoreDescending = ref(true);
  let hydrated = false; let hydrating: Promise<void> | null = null;
  let generation = 0; let scoreGeneration = 0;
  let persistTimer: ReturnType<typeof setTimeout> | undefined;
  let persistQueue: Promise<void> = Promise.resolve();
  const activePreset = computed(() => {
    const selected = presets.value.find(p => p.id === selectedConditionId.value);
    if (selected && signature(selected.filter) === signature(filter.value)) return selected;
    return presets.value.find(p => signature(p.filter) === signature(filter.value)) ?? null;
  });
  const selectedCondition = computed(() => savedConditions.value.find(p => p.id === selectedConditionId.value) ?? null);
  const conditionsChanged = computed(() => hasLoaded.value && !!appliedFilter.value &&
    (signature(filter.value) !== signature(appliedFilter.value) || sourceMode.value !== appliedSource.value || pageSize.value !== appliedPageSize.value));
  const isRanking = computed(() => rankingRows.value !== null);
  const scoreAsOf = computed(() => Object.values(statuses.value)
    .filter(s => !s.error && s.history && !s.history.stale && s.score !== null && Number.isFinite(s.score))
    .map(s => dateKey(s.history?.end_date)).filter(Boolean).sort().at(-1) ?? '');
  function rankableScore(row: SnapshotRow): number | null {
    const item = statuses.value[toFullSymbol(row.code, row.board)];
    return item && !item.error && item.history && !item.history.stale && scoreAsOf.value &&
      dateKey(item.history.end_date) === scoreAsOf.value && typeof item.score === 'number' &&
      Number.isFinite(item.score) && item.score >= 0 && item.score <= 100 ? item.score : null;
  }
  const sortedRanking = computed(() => {
    const source = rankingRows.value ?? [];
    if (!rankingSorted.value) return source;
    return [...source].sort((a, b) => {
      const left = rankableScore(a), right = rankableScore(b);
      if (left === null || right === null) return left !== null ? -1 : right !== null ? 1 : a.code.localeCompare(b.code);
      return (scoreDescending.value ? right - left : left - right) || a.code.localeCompare(b.code);
    });
  });
  const rows = computed(() => rankingRows.value === null ? quoteRows.value :
    sortedRanking.value.slice((page.value - 1) * appliedPageSize.value, page.value * appliedPageSize.value));
  const resultCount = computed(() => rankingRows.value?.length ?? totalMatched.value);
  const pageCount = computed(() => Math.max(1, Math.ceil(resultCount.value / appliedPageSize.value)));
  const scoreSucceeded = computed(() => Object.values(statuses.value).filter(s => !s.error && s.score !== null).length);
  const scoreFailed = computed(() => Object.values(statuses.value).filter(s => !!s.error || s.score === null).length);
  const scoreComparable = computed(() => (rankingRows.value ?? quoteRows.value).filter(row => rankableScore(row) !== null).length);
  async function persist() {
    if (!hydrated) return;
    const values = { universe_preset: 'all', universe_filter: signature(filter.value),
      universe_manual_schema: MANUAL_SCHEMA, universe_selected_condition: selectedConditionId.value,
      universe_last_view: lastView.value };
    persistQueue = persistQueue.then(async () => {
      try { for (const [key, value] of Object.entries(values)) await invoke('set_setting', { key, value }); }
      catch (e) { restoreError.value = '条件保存失败：' + e; }
    });
    await persistQueue;
  }
  function hydrate(): Promise<void> {
    if (hydrated) return Promise.resolve();
    if (hydrating) return hydrating;
    hydrating = (async () => {
      restoreError.value = '';
      try {
        const settings = await invoke<Record<string, string>>('get_settings');
        const savedId = settings.universe_preset;
        if (settings.universe_filter && settings.universe_manual_schema !== MANUAL_SCHEMA && !settings.universe_archived_manual_filter) {
          await invoke('set_setting', { key: 'universe_archived_manual_filter', value: settings.universe_filter });
          migrationNotice.value = '旧历史技术条件已留档；当前只恢复手动行情条件，模型研究请切换上方页面。';
        }
        const legacy = savedId != null && savedId !== 'all' && savedId !== 'custom';
        if (legacy) {
          filter.value = createDefaultFilter();
          migrationNotice.value = '旧策略条件已停用，已回到不限条件；旧记录保留在本地，不会自动运行。';
          if (settings.universe_filter && !settings.universe_legacy_filter_archive) await invoke('set_setting', {
            key: 'universe_legacy_filter_archive', value: JSON.stringify({ preset: savedId, filter: settings.universe_filter }),
          });
        } else {
          try { filter.value = manualFilter(JSON.parse(settings.universe_filter || '{}')); }
          catch { filter.value = createDefaultFilter(); restoreError.value = '上次条件格式错误，已回到不限条件。'; }
        }
        try {
          const saved: unknown = JSON.parse(settings[CONDITIONS_KEY] || '[]');
          if (!Array.isArray(saved)) throw new Error('条件列表格式错误');
          const seen = new Set<string>();
          savedConditions.value = saved.filter((p): p is PresetInfo => !!p && typeof p === 'object' &&
            typeof p.id === 'string' && p.id.startsWith('condition_') && typeof p.label === 'string' && !!p.label.trim())
            .filter(p => { if (seen.has(p.id)) return false; seen.add(p.id); return true; }).slice(0, 40)
            .map(p => ({ ...BASIC_PRESET, ...p, label: p.label.trim(), builtin: false, rule: '', filter: manualFilter(p.filter), strategy_status: 'quote_tool' }));
        } catch {
          if (settings[CONDITIONS_KEY]) await invoke('set_setting', {
            key: CONDITIONS_KEY + '_backup', value: settings[CONDITIONS_KEY],
          });
          restoreError.value = '已保存条件格式错误，原内容已备份；当前不执行旧条件。';
        }
        selectedConditionId.value = settings.universe_selected_condition || 'all';
        if (settings.universe_last_view === 'models') lastView.value = 'models';
        if (settings.universe_source === 'sina' || settings.universe_source === 'eastmoney') sourceMode.value = settings.universe_source;
        const size = Number(settings.universe_page_size);
        if (Number.isFinite(size) && size >= PAGE_SIZE_MIN && size <= PAGE_SIZE_MAX) pageSize.value = Math.round(size);
        hydrated = true;
      } catch (e) { restoreError.value = '条件恢复失败，可重试：' + e; }
      try {
        const list = await invoke<PresetInfo[]>('get_filter_presets');
        const ids = ['all', 'quote_liquidity', 'quote_active', 'quote_gentle_rise'];
        builtins.value = ids.map(id => list.find(p => p.id === id)).filter((p): p is PresetInfo => !!p)
          .map(p => ({ ...p, label: p.id === 'all' ? '不限条件' : p.label, rule: '', filter: manualFilter(p.filter) }));
        if (!builtins.value.some(p => p.id === 'all')) builtins.value.unshift(BASIC_PRESET);
      } catch { builtins.value = [BASIC_PRESET]; }
      await persist();
    })().finally(() => { hydrating = null; });
    return hydrating;
  }
  function flushPersist() {
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = undefined; void persist();
  }
  watch([filter, lastView, selectedConditionId], () => {
    if (!hydrated) return;
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => void persist(), 300);
  }, { deep: true });
  async function setView(view: ScreenerView) { lastView.value = view; await persist(); }
  async function setPageSize(value: number) {
    if (!Number.isFinite(value)) return;
    pageSize.value = Math.min(PAGE_SIZE_MAX, Math.max(PAGE_SIZE_MIN, Math.round(value)));
    await invoke('set_setting', { key: 'universe_page_size', value: String(pageSize.value) });
  }
  async function setSourceMode(mode: SourceMode) {
    sourceMode.value = mode;
    try { await invoke('set_setting', { key: 'universe_source', value: mode }); }
    catch (e) { restoreError.value = '数据源设置保存失败：' + e; }
  }
  async function applyPreset(id: string) {
    await hydrate();
    const preset = presets.value.find(p => p.id === id);
    if (!preset || loading.value || scoring.value) return;
    selectedConditionId.value = id; filter.value = manualFilter(preset.filter); await persist();
  }
  async function saveCondition(label: string, id?: string, renameOnly = false) {
    await hydrate();
    if (!hydrated) throw new Error('设置未恢复，暂不能覆盖已保存条件。');
    const name = label.trim();
    if (!name || Array.from(name).length > 32) throw new Error('条件名称需为 1—32 个字。');
    const existing = id ? savedConditions.value.find(p => p.id === id) : undefined;
    if (id && !existing) throw new Error('要修改的条件不存在。');
    if (!id && savedConditions.value.length >= 40) throw new Error('最多保存 40 组条件，请先删除不用的。');
    if (presets.value.some(p => p.id !== id && p.label.toLocaleLowerCase() === name.toLocaleLowerCase())) throw new Error('已有同名条件，请换个名称。');
    validateRanges(manualFilter(renameOnly && existing ? existing.filter : filter.value));
    const saved: PresetInfo = { ...BASIC_PRESET, id: id || 'condition_' + crypto.randomUUID(), label: name,
      description: '自定义行情条件', builtin: false, filter: manualFilter(renameOnly && existing ? existing.filter : filter.value) };
    const next = existing ? savedConditions.value.map(p => p.id === id ? saved : p) : [...savedConditions.value, saved];
    await invoke('set_setting', { key: CONDITIONS_KEY, value: JSON.stringify(next) });
    savedConditions.value = next; selectedConditionId.value = saved.id; await persist(); return saved;
  }
  async function deleteCondition(id: string) {
    const next = savedConditions.value.filter(p => p.id !== id);
    if (next.length === savedConditions.value.length) throw new Error('条件不存在。');
    await invoke('set_setting', { key: CONDITIONS_KEY, value: JSON.stringify(next) });
    savedConditions.value = next;
    if (selectedConditionId.value === id) selectedConditionId.value = 'custom';
    await persist();
  }
  function cancelScoring() { if (scoring.value) { scoreCancelled.value = true; ++scoreGeneration; } }
  function clearScores() {
    cancelScoring(); statuses.value = {}; rankingRows.value = null; rankingSorted.value = false;
    page.value = quotePage.value; scoreTotal.value = 0; scoreDone.value = 0; scoreError.value = '';
  }
  async function reset() { filter.value = createDefaultFilter(); selectedConditionId.value = 'all'; await persist(); }
  function acceptResponse(response: UniverseResponse) {
    totalAll.value = response.total_all; totalMatched.value = response.total_matched;
    stale.value = response.stale; sourceLabel.value = response.source_label;
    skippedConditions.value = response.skipped_conditions ?? [];
    volumeRatioSupported.value = response.volume_ratio_supported; change60dSupported.value = response.change_60d_supported;
    listingDateSupported.value = response.listing_date_supported; sectorSupported.value = response.sector_supported;
    historyNotice.value = response.history_notice; historyEvaluated.value = response.history_evaluated;
  }
  async function requestPage(wanted: number, forceRefresh: boolean, reuseSnapshot: boolean) {
    if (loading.value || scoring.value) return;
    const request = ++generation; loading.value = true; error.value = '';
    try {
      await hydrate();
      const queryFilter = reuseSnapshot && appliedFilter.value ? manualFilter(appliedFilter.value) : manualFilter(filter.value);
      validateRanges(queryFilter);
      const querySource = reuseSnapshot ? appliedSource.value : sourceMode.value;
      const querySize = reuseSnapshot ? appliedPageSize.value : pageSize.value;
      if (!reuseSnapshot) { clearScores(); quoteRows.value = []; hasLoaded.value = false; }
      const response = await invoke<UniverseResponse>('get_market_universe', {
        filter: queryFilter, page: wanted, pageSize: querySize, forceRefresh, reuseSnapshot, source: querySource,
      });
      if (request !== generation) return;
      quoteRows.value = response.rows.filter(eligibleRow);
      const rejected = response.rows.length - quoteRows.value.length;
      if (rejected) error.value = '已隐藏 ' + rejected + ' 条范围外股票；命中统计为数据源原始数量。';
      acceptResponse(response); appliedFilter.value = queryFilter; appliedSource.value = querySource; appliedPageSize.value = querySize;
      page.value = wanted; quotePage.value = wanted; hasLoaded.value = true; await persist();
    } catch (e) { if (request === generation) error.value = '获取行情失败：' + e; }
    finally { if (request === generation) loading.value = false; }
  }
  const search = (forceRefresh = false) => requestPage(1, forceRefresh, false);
  async function fetchPage(wanted: number) {
    if (loading.value) return;
    const target = Math.min(pageCount.value, Math.max(1, Math.round(wanted)));
    if (rankingRows.value !== null) { page.value = target; return; }
    await requestPage(target, false, true);
  }
  async function scoreResults(kind: 'page' | 'all') {
    if (scoring.value || loading.value || !hasLoaded.value || !appliedFilter.value) return;
    if (conditionsChanged.value) { scoreError.value = '条件已调整，请先点击查询再评分。'; return; }
    if (stale.value) { scoreError.value = '当前行情快照已陈旧，请先刷新查询。'; return; }
    const request = ++scoreGeneration;
    scoring.value = true; scoringKind.value = kind; scoreCancelled.value = false;
    scoreError.value = ''; scoreDone.value = 0; scoreTotal.value = 0;
    try {
      let targets = [...rows.value];
      if (kind === 'all') {
        statuses.value = {}; rankingRows.value = null; rankingSorted.value = false; page.value = quotePage.value;
        // No page/pageSize: request all matches, rather than the top-N recommendation list.
        const response = await invoke<UniverseResponse>('get_market_universe', {
          filter: manualFilter(appliedFilter.value), limit: 100000, forceRefresh: false,
          reuseSnapshot: true, source: appliedSource.value,
        });
        if (request !== scoreGeneration) return;
        if (response.stale) throw new Error('完整行情快照已陈旧，请重新查询。');
        if (response.rows.length !== response.total_matched) throw new Error('数据源未返回全部命中股票，本次不生成全量排名。');
        targets = response.rows.filter(eligibleRow);
        acceptResponse(response); rankingRows.value = targets; rankingSorted.value = false;
        scoreDescending.value = true; page.value = 1; statuses.value = {};
      }
      scoreTotal.value = targets.length;
      for (let offset = 0; offset < targets.length; offset += SCORE_BATCH_SIZE) {
        if (request !== scoreGeneration) break;
        const symbols = targets.slice(offset, offset + SCORE_BATCH_SIZE).map(row => toFullSymbol(row.code, row.board));
        let batch: StockStatusItem[];
        try { batch = await invoke<StockStatusItem[]>('batch_stock_status', { symbols, rule: 'auto' }); }
        catch (e) { batch = symbols.map(symbol => failedStatus(symbol, String(e))); }
        if (request !== scoreGeneration) break;
        const received = new Map(batch.map(item => [item.symbol, item]));
        statuses.value = { ...statuses.value, ...Object.fromEntries(symbols.map(symbol => [symbol,
          received.get(symbol) ?? failedStatus(symbol, '本批次没有返回该股票的评分')])) };
        scoreDone.value += symbols.length;
      }
      if (request === scoreGeneration && kind === 'all') rankingSorted.value = true;
    } catch (e) { if (request === scoreGeneration) scoreError.value = '评分失败：' + e; }
    finally { scoring.value = false; scoringKind.value = null; }
  }
  function sortScores() {
    if (rankingRows.value !== null) { scoreDescending.value = rankingSorted.value ? !scoreDescending.value : true; rankingSorted.value = true; page.value = 1; }
  }
  return { presets, savedConditions, filter, selectedCondition, lastView, migrationNotice, restoreError,
    rows, totalAll, totalMatched, resultCount, stale, hasLoaded, sourceLabel, skippedConditions,
    volumeRatioSupported, change60dSupported, listingDateSupported, sectorSupported, historyNotice, historyEvaluated,
    sourceMode, pageSize, page, pageCount, appliedPageSize, appliedFilter, conditionsChanged, loading, error,
    activePreset, applyPreset, hydrate, flushPersist, setView, setPageSize, setSourceMode, reset, search, fetchPage,
    saveCondition, deleteCondition, statuses, isRanking, scoring, scoringKind, scoreTotal, scoreDone,
    scoreCancelled, scoreError, scoreAsOf, scoreSucceeded, scoreFailed, scoreComparable, rankableScore,
    scoreResults, cancelScoring, clearScores, sortScores, rankingSorted };
});
