// src/stores/universe.ts
// 全市场股票池（漏斗 L0 + L1）状态管理。
//
// 设计要点：
// - 预设与筛选规则由 Rust 侧下发（`get_filter_presets`），前端只做展示与微调，
//   避免前后端各维护一份规则导致不一致。
// - 快照本身在 Rust 侧带 60 秒缓存，所以重复点击「筛选」不会打爆数据源。

import { defineStore } from 'pinia';
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type {
  BoardCount,
  MarketFilter,
  PresetInfo,
  SnapshotRow,
  SnapshotSource,
  UniverseResponse,
} from '@/types/universe';
import { createDefaultFilter } from '@/types/universe';

/** 筛选条件持久化在 settings 表里的 key（复用现有 KV 存储，零后端改动） */
const FILTER_SETTING_KEY = 'universe_filter';
const PRESET_SETTING_KEY = 'universe_preset';
const SOURCE_SETTING_KEY = 'universe_source';
const PAGE_SIZE_SETTING_KEY = 'universe_page_size';

/** 结果表每页条数的取值范围 */
export const PAGE_SIZE_MIN = 1;
export const PAGE_SIZE_MAX = 100;
export const PAGE_SIZE_DEFAULT = 20;

/** 「自定义」不是后端预设，而是「手动改过条件」的标记 */
export const CUSTOM_PRESET_ID = 'custom';

/**
 * 把前端日志转发到 Rust 侧的 bull-arrives.log。
 *
 * 背景：`console.error` 只进 WebView 的开发者控制台，用户看不到、我们也拿不到。
 * 排查「界面没反应但后端日志干净」的问题必须靠这条通道（fire-and-forget，绝不阻塞业务）。
 */
function logToBackend(level: 'info' | 'warn' | 'error', message: string): void {
  void invoke('log_frontend', { level, message }).catch(() => {
    // 连日志都失败时只能静默，避免日志问题本身引发雪崩
  });
}

/**
 * 后端 `get_filter_presets` 失败时的兜底预设。
 *
 * ⚠️ Rust 侧 `eastmoney_universe::FilterPreset` 才是唯一权威来源；这份副本**只在后端
 * 命令失败时**用来保证 UI 不至于一个标签都没有（否则用户会以为功能坏了）。
 * 若改了 Rust 侧的数值，请同步这里。数值必须与 `FilterPreset::build()` 一致。
 */
const FALLBACK_PRESETS: PresetInfo[] = (() => {
  const entry = (id: string, label: string, description: string, patch: Partial<MarketFilter>) => ({
    id,
    label,
    description,
    filter: { ...createDefaultFilter(), ...patch },
  });
  return [
    entry('all', '全部', '仅做基础排除（ST/退市/停牌/一字板/B股），不限数值区间', {}),
    entry('steady_trend', '稳健趋势', '中大盘、温和放量、温和上涨 —— 适合中线持有', {
      market_cap_min_yi: 100, market_cap_max_yi: 3000,
      turnover_min: 1, turnover_max: 8, volume_ratio_min: 1,
      change_pct_min: -2, change_pct_max: 5,
      amount_min_wan: 10000, price_min: 5,
    }),
    entry('strong_breakout', '强势突破', '明显放量且涨幅靠前 —— 适合突破跟进', {
      volume_ratio_min: 2, change_pct_min: 3, change_pct_max: 9,
      turnover_min: 3, turnover_max: 20, amount_min_wan: 20000,
    }),
    entry('short_term_active', '短线活跃', '中小市值、高换手、有资金关注 —— 适合短线', {
      market_cap_min_yi: 20, market_cap_max_yi: 300,
      turnover_min: 5, turnover_max: 25, volume_ratio_min: 1.5,
      change_pct_min: -3, change_pct_max: 9, amount_min_wan: 8000,
    }),
    entry('oversold_rebound', '超跌反弹', '当日走弱但换手不低 —— 博超跌反弹', {
      change_pct_min: -9, change_pct_max: 0, turnover_min: 2,
      volume_ratio_min: 1.2, amount_min_wan: 8000, price_min: 3,
    }),
  ];
})();

/** 数据源模式：auto 表示后端自动（新浪优先、东财兜底） */
export type SourceMode = 'auto' | SnapshotSource;

function cloneFilter(source: MarketFilter): MarketFilter {
  return { ...source, boards: [...source.boards] };
}

/** 把未知值安全地收敛为 number | null，避免脏数据破坏 UI 绑定 */
function numOrNull(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null;
}

/** 宽松解析持久化的筛选条件，字段缺失/类型不对时回退默认值 */
function safeParseFilter(raw: string): MarketFilter | null {
  try {
    const obj = JSON.parse(raw) as Partial<MarketFilter>;
    if (!obj || typeof obj !== 'object') return null;
    const d = createDefaultFilter();
    return {
      boards: Array.isArray(obj.boards) ? obj.boards : d.boards,
      exclude_st: typeof obj.exclude_st === 'boolean' ? obj.exclude_st : d.exclude_st,
      exclude_delisting: typeof obj.exclude_delisting === 'boolean' ? obj.exclude_delisting : d.exclude_delisting,
      exclude_suspended: typeof obj.exclude_suspended === 'boolean' ? obj.exclude_suspended : d.exclude_suspended,
      exclude_limit_locked: typeof obj.exclude_limit_locked === 'boolean' ? obj.exclude_limit_locked : d.exclude_limit_locked,
      price_min: numOrNull(obj.price_min),
      price_max: numOrNull(obj.price_max),
      market_cap_min_yi: numOrNull(obj.market_cap_min_yi),
      market_cap_max_yi: numOrNull(obj.market_cap_max_yi),
      turnover_min: numOrNull(obj.turnover_min),
      turnover_max: numOrNull(obj.turnover_max),
      volume_ratio_min: numOrNull(obj.volume_ratio_min),
      change_pct_min: numOrNull(obj.change_pct_min),
      change_pct_max: numOrNull(obj.change_pct_max),
      amount_min_wan: numOrNull(obj.amount_min_wan),
    };
  } catch {
    return null;
  }
}

export const useUniverseStore = defineStore('universe', () => {
  const presets = ref<PresetInfo[]>([]);
  const activePresetId = ref('all');
  // 保持非空：真正的默认值在 Rust 侧，这里只是预设加载前的占位，UI 可安全绑定
  const filter = ref<MarketFilter>(createDefaultFilter());

  /** 预设加载失败的原因。非空时 UI 要给出重试入口 —— 不能静默吞掉 */
  const presetsError = ref<string | null>(null);
  const presetsLoading = ref(false);

  const rows = ref<SnapshotRow[]>([]);
  const boardCounts = ref<BoardCount[]>([]);
  const totalAll = ref(0);
  const totalMatched = ref(0);
  const stale = ref(false);
  const hasLoaded = ref(false);

  /** 数据来自哪个通道 */
  const source = ref<SnapshotSource>('sina');
  const sourceLabel = ref('');
  /** 当前通道是否提供「量比」。false 时 UI 必须禁用该条件并说明原因 */
  const volumeRatioSupported = ref(true);
  /** 被数据源不支持而自动忽略的条件名 */
  const skippedConditions = ref<string[]>([]);
  /** 用户选择的取数通道（持久化在 settings） */
  const sourceMode = ref<SourceMode>('auto');
  /** 结果表每页条数（1–100），持久化在 settings */
  const pageSize = ref(PAGE_SIZE_DEFAULT);
  /** 当前页码（1 起） */
  const page = ref(1);
  /** 结果表是否已展示：筛选完成后先只显示统计，点「展示数据」才出现表格 */
  const resultsVisible = ref(false);
  /** 翻页请求进行中（与筛选 loading 分开，避免整表闪烁） */
  const pageLoading = ref(false);

  const loading = ref(false);
  const error = ref<string | null>(null);

  // 竞态保护：只接受最后一次请求的结果
  let generation = 0;

  /** 当前生效预设的中文名 */
  const activePresetLabel = computed(
    () => presets.value.find(p => p.id === activePresetId.value)?.label ?? '自定义',
  );

  /** 拉取预设（幂等，只需一次）。
   *
   * ⚠️ 失败会抛出，但**绝不缓存失败状态** —— 由调用方决定是否重试。
   * 之前这里一失败就把整个 hydrate 永久卡死，表现为「预设标签只剩自定义、
   * 点开始筛选也永远没结果」，就是这个原因。
   */
  async function ensurePresets() {
    if (presets.value.length) return;
    presetsLoading.value = true;
    presetsError.value = null;
    try {
      const list = await invoke<PresetInfo[]>('get_filter_presets');
      presets.value = list;
      presetsError.value = null;
      logToBackend('info', `预设加载成功：${list.map(p => p.id).join(',')}`);
    } catch (e) {
      // 兜底：后端失败也给出预设标签，保证筛选器可用；同时明确提示用户这是降级状态
      presetsError.value = String(e);
      presets.value = FALLBACK_PRESETS;
      logToBackend('error', `get_filter_presets 失败，已用内置兜底：${String(e)}`);
      console.error('[universe] 加载预设失败，已使用内置兜底预设:', e);
    } finally {
      presetsLoading.value = false;
    }
    // 预设加载完成后立刻对齐到当前选中项，覆盖占位默认值。
    // 注意：custom 表示「手动改过条件」，此时不能套用任何预设，否则会覆盖掉已恢复的条件。
    if (activePresetId.value !== CUSTOM_PRESET_ID) {
      const initial = presets.value.find(p => p.id === activePresetId.value) ?? presets.value[0];
      if (initial) filter.value = cloneFilter(initial.filter);
    }
  }

  /** 预设加载失败后的手动重试入口 */
  async function retryPresets() {
    presetsError.value = null;
    try {
      await ensurePresets();
    } catch {
      // 错误已写入 presetsError，由 UI 展示
    }
  }

  /** 用户手动改过条件 → 标记为「自定义」 */
  function markCustom() {
    activePresetId.value = CUSTOM_PRESET_ID;
  }

  /**
   * 切换预设：整体替换数值区间，但**保留用户自己设置的板块与排除项**。
   *
   * 板块（沪/深/创/科/北）和排除规则（ST/退市/停牌/一字板）是「我的选股范围」，
   * 而预设描述的是「在这些范围里找什么形态的股票」。之前切预设会把板块一起重置，
   * 用户每切一次就得重新勾一遍，所以这里显式保留。
   */
  function applyPreset(id: string) {
    if (id === CUSTOM_PRESET_ID) {
      // 自定义只是一次「标记」，不动任何条件
      activePresetId.value = CUSTOM_PRESET_ID;
      void persist();
      return;
    }
    const preset = presets.value.find(p => p.id === id);
    if (!preset) return;
    const next = cloneFilter(preset.filter);
    const current = filter.value;
    next.boards = [...current.boards];
    next.exclude_st = current.exclude_st;
    next.exclude_delisting = current.exclude_delisting;
    next.exclude_suspended = current.exclude_suspended;
    next.exclude_limit_locked = current.exclude_limit_locked;
    filter.value = next;
    activePresetId.value = id;
    void persist();
  }

  // 持久化只需加载一次（首次筛选前）
  let persistedLoaded = false;
  /** 恢复失败的原因（非空时 UI 提示，且允许重试） */
  const restoreError = ref<string | null>(null);
  // 多个调用方（对话框打开 / 自动筛选）可能同时触发 hydrate，
  // 必须共用同一个 Promise，否则会出现「条件恢复了但立刻又被预设覆盖」的竞态。
  let hydratePromise: Promise<void> | null = null;

  /**
   * 从 settings 恢复上次的筛选条件与预设选择。
   *
   * ⚠️ `persistedLoaded` 只在**成功后**才置位：之前是在 await 之前就置 true，
   * 一旦 `get_settings` 瞬时失败（重启后偶发），就永久跳过恢复，
   * 表现为「重启后板块/排除回到默认、预设只剩自定义」。
   */
  async function loadPersisted() {
    if (persistedLoaded) return;
    restoreError.value = null;
    try {
      const all = await invoke<Record<string, string>>('get_settings');
      persistedLoaded = true;
      const presetId = all[PRESET_SETTING_KEY];
      // 注意：`custom`（手动微调过）不在预设列表里，但也要原样恢复，
      // 否则 chip 会错误地高亮「全部」而实际条件是自定义的。
      if (presetId) activePresetId.value = presetId;
      const raw = all[FILTER_SETTING_KEY];
      if (raw) {
        const parsed = safeParseFilter(raw);
        if (parsed) {
          filter.value = parsed;
          // 有持久化的自定义条件但没有合法预设时，标记为「自定义」
          if (!all[PRESET_SETTING_KEY]) activePresetId.value = CUSTOM_PRESET_ID;
        }
      }
      const mode = all[SOURCE_SETTING_KEY];
      if (mode === 'sina' || mode === 'eastmoney') sourceMode.value = mode;
      const size = Number(all[PAGE_SIZE_SETTING_KEY]);
      if (Number.isFinite(size) && size >= PAGE_SIZE_MIN && size <= PAGE_SIZE_MAX) {
        pageSize.value = Math.round(size);
      }
    } catch (e) {
      restoreError.value = String(e);
      logToBackend('error', `get_settings 恢复筛选条件失败：${String(e)}`);
      console.warn('[universe] 恢复筛选条件失败:', e);
    }
  }

  /** 修改结果表每页条数（1–100），并持久化；若结果已展示则回到第 1 页重新取数 */
  async function setPageSize(value: number) {
    const v = Math.min(PAGE_SIZE_MAX, Math.max(PAGE_SIZE_MIN, Math.round(value)));
    const changed = pageSize.value !== v;
    pageSize.value = v;
    if (!changed) return;
    try {
      await invoke('set_setting', { key: PAGE_SIZE_SETTING_KEY, value: String(v) });
    } catch (e) {
      console.warn('[universe] 保存每页条数失败:', e);
    }
    if (resultsVisible.value && hasLoaded.value) {
      await fetchPage(1);
    }
  }

  /** 切换取数通道并持久化 */
  async function setSourceMode(mode: SourceMode) {
    sourceMode.value = mode;
    try {
      await invoke('set_setting', { key: SOURCE_SETTING_KEY, value: mode });
    } catch (e) {
      console.warn('[universe] 保存数据源失败:', e);
    }
  }

  /** 把当前生效条件写回 settings（失败不阻断，仅记录） */
  async function persist() {
    try {
      await invoke('set_setting', { key: PRESET_SETTING_KEY, value: activePresetId.value });
      await invoke('set_setting', { key: FILTER_SETTING_KEY, value: JSON.stringify(filter.value) });
    } catch (e) {
      console.warn('[universe] 保存筛选条件失败:', e);
    }
  }

  /**
   * 立即把待写入的条件落盘（不走防抖）。
   *
   * 在关闭筛选器对话框时调用：用户改完板块/排除就关掉对话框、甚至直接退出程序，
   * 防抖的 400ms 很容易还没到就被带走 —— 这正是「重启后选择没保留」的另一个来源。
   */
  function flushPersist() {
    if (persistTimer) {
      clearTimeout(persistTimer);
      persistTimer = undefined;
    }
    if (suppressPersist) return;
    void persist();
  }

  // 条件一变就落盘（防抖）。
  // 之前只在「筛选成功」后才持久化，一旦网络失败（东财限流）就什么都不记，
  // 用户下次打开又回到默认条件 —— 这正是要修的问题。
  let persistTimer: ReturnType<typeof setTimeout> | undefined;
  /** 重置期间抑制自动落盘，避免把「已清空」的持久化又写回来 */
  let suppressPersist = false;
  function schedulePersist() {
    if (suppressPersist) return;
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => void persist(), 400);
  }
  watch(filter, schedulePersist, { deep: true });
  watch(activePresetId, schedulePersist);

  /** 预设 + 持久化条件的一次性恢复。
   *
   * ⚠️ 关键设计：**这个 Promise 永远不会 reject**，且任何一步失败都会把
   * `hydratePromise` 清空以便下次重试。之前版本一旦失败就把单例 Promise
   * 永久缓存成 rejected，导致「打开筛选器 → 预设不显示 → 点开始筛选也没反应」，
   * 而且错误被静默吞掉，完全无从排查。
   */
  function hydrate(): Promise<void> {
    if (!hydratePromise) {
      hydratePromise = (async () => {
        let failed = false;
        try {
          await ensurePresets();
        } catch {
          failed = true; // 预设失败不阻断后续：筛选仍可用当前条件
        }
        try {
          await loadPersisted();
        } catch (e) {
          failed = true;
          console.warn('[universe] 恢复上次条件失败:', e);
        }
        if (failed) hydratePromise = null; // 允许下次重试
      })();
    }
    return hydratePromise;
  }

  /** 恢复默认（清空持久化并回到「全部」预设） */
  async function reset() {
    persistedLoaded = true; // 阻止 reset 后又被 loadPersisted 覆盖
    suppressPersist = true; // 阻止自动落盘把刚清空的条件写回来
    activePresetId.value = 'all';
    const all = presets.value.find(p => p.id === 'all') ?? presets.value[0];
    filter.value = all ? cloneFilter(all.filter) : createDefaultFilter();
    page.value = 1;
    resultsVisible.value = false;
    restoreError.value = null;
    try {
      await invoke('set_setting', { key: PRESET_SETTING_KEY, value: 'all' });
      await invoke('set_setting', { key: FILTER_SETTING_KEY, value: '' });
    } catch (e) {
      console.warn('[universe] 重置筛选条件失败:', e);
    } finally {
      if (persistTimer) clearTimeout(persistTimer);
      persistTimer = undefined;
      suppressPersist = false;
    }
  }

  /** 切换预设：整体替换数值区间，保留板块与排除项（见 applyPreset 上方说明）。
   *  旧版本在这里把整个 filter 替换掉，会把用户选的板块一起重置，已废弃。 */

  /**
   * 执行筛选：只算「命中多少条」并取回**第一页**。
   *
   * 结果表默认**不显示**——等用户点「展示数据」再出现（见 `showResults`）。
   * 这样筛选本身就是一次快速反馈，而不是一次性甩出几百行。
   */
  async function search(forceRefresh = false) {
    const request = ++generation;
    loading.value = true;
    error.value = null;
    try {
      // hydrate 内部已容错，不会 reject；这里再兜一层，确保筛选永不因初始化失败而停摆
      await hydrate().catch(e => console.warn('[universe] 初始化未完成，仍继续筛选:', e));

      const response = await invoke<UniverseResponse>('get_market_universe', {
        filter: filter.value,
        page: 1,
        pageSize: pageSize.value,
        forceRefresh,
        source: sourceMode.value,
      });
      if (request !== generation) return;

      applyResponse(response);
      page.value = 1;
      // 展示状态在同一会话内保持：点过一次「展示数据」后，后续筛选直接更新表格，
      // 不再要求每次都重新点（第一次仍需点，符合"先看命中数再展开"的预期）
      hasLoaded.value = true;
      logToBackend(
        'info',
        `筛选完成：全市场${response.total_all} 命中${response.total_matched} 返回${response.rows.length} 条 ` +
          `(${response.source_label}${response.stale ? '/陈旧' : ''})`,
      );

      // 筛选成功后才把当前条件记下来（这是用户确认过的有效条件）
      void persist();
    } catch (e) {
      if (request === generation) error.value = `获取全市场数据失败：${e}`;
      logToBackend('error', `search 失败：${String(e)}`);
      console.error('[universe] search failed:', e);
    } finally {
      if (request === generation) loading.value = false;
    }
  }

  /**
   * 翻页：**只向后端取指定那一页**，不在前端囤积全部数据。
   *
   * 后端按「成交额降序 + 代码升序」稳定排序，所以同一份快照内翻页不会重复、不会漏行。
   */
  async function fetchPage(target: number) {
    const wanted = Math.max(1, Math.round(target));
    const request = ++generation;
    pageLoading.value = true;
    error.value = null;
    try {
      const response = await invoke<UniverseResponse>('get_market_universe', {
        filter: filter.value,
        page: wanted,
        pageSize: pageSize.value,
        source: sourceMode.value,
      });
      if (request !== generation) return;
      applyResponse(response);
      page.value = wanted;
      hasLoaded.value = true;
      logToBackend('info', `翻页成功：第 ${wanted} 页 返回 ${response.rows.length} 条`);
    } catch (e) {
      if (request === generation) error.value = `获取第 ${wanted} 页失败：${e}`;
      logToBackend('error', `fetchPage(${wanted}) 失败：${String(e)}`);
      console.error('[universe] fetchPage failed:', e);
    } finally {
      if (request === generation) pageLoading.value = false;
    }
  }

  /** 「展示数据 / 收起明细」开关 */
  async function toggleResults() {
    if (resultsVisible.value) {
      resultsVisible.value = false;
      logToBackend('info', '收起明细');
      return;
    }
    logToBackend('info', '点击「展示数据」');
    await showResults();
    logToBackend(
      'info',
      `展示数据结果：resultsVisible=${resultsVisible.value}，rows=${rows.value.length}`,
    );
  }

  /** 「展示数据」按钮：第一页通常已在筛选时取回，直接显示即可 */
  async function showResults() {
    if (rows.value.length) {
      resultsVisible.value = true;
      return;
    }
    await fetchPage(page.value || 1);
    if (rows.value.length) resultsVisible.value = true;
  }

  /** 把一次 `get_market_universe` 的响应落到 store */
  function applyResponse(response: UniverseResponse) {
    rows.value = response.rows;
    boardCounts.value = response.board_counts;
    totalAll.value = response.total_all;
    totalMatched.value = response.total_matched;
    stale.value = response.stale;
    source.value = response.source;
    sourceLabel.value = response.source_label;
    volumeRatioSupported.value = response.volume_ratio_supported;
    skippedConditions.value = response.skipped_conditions;
  }

  return {
    presets,
    activePresetId,
    activePresetLabel,
    filter,
    rows,
    boardCounts,
    totalAll,
    totalMatched,
    stale,
    hasLoaded,
    source,
    sourceLabel,
    volumeRatioSupported,
    skippedConditions,
    sourceMode,
    pageSize,
    presetsError,
    presetsLoading,
    restoreError,
    page,
    pageLoading,
    resultsVisible,
    loading,
    error,
    ensurePresets,
    retryPresets,
    setSourceMode,
    setPageSize,
    fetchPage,
    showResults,
    toggleResults,
    flushPersist,
    hydrate,
    applyPreset,
    markCustom,
    search,
    reset,
  };
});
