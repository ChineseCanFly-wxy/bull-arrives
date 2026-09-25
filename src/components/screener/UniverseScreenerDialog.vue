<script setup lang="ts">
// src/components/screener/UniverseScreenerDialog.vue
// 全市场筛选器（漏斗 L0 + L1）
//
// 设计说明：
// - 筛选规则与预设全部由 Rust 侧下发，前端只负责展示与微调
// - 快照在 Rust 侧带 60 秒缓存，重复点击「筛选」不会打爆数据源
// - 结果表支持本地排序；「加自选」会自动把 6 位代码转成 sh/sz/bj 前缀的完整符号

import { computed, h, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {
  NButton,
  NCheckbox,
  NCheckboxGroup,
  NDataTable,
  NDropdown,
  NInput,
  NInputNumber,
  NModal,
  NSelect,
  NTag,
  type DataTableColumns,
  type DropdownOption,
} from 'naive-ui';
import { useUniverseStore } from '@/stores/universe';
import { useWatchlistStore } from '@/stores/watchlist';
import { useRankStore } from '@/stores/rank';
import AnalysisDialog from '@/components/analysis/AnalysisDialog.vue';
import type { StockStatusItem } from '@/types/analysis';
import { TRADE_RULE_OPTIONS, tradeRuleLabel, type TradeRuleId } from '@/types/analysis';
import type { SectorKind, SectorRotation, SectorSummary } from '@/types/sector';
import {
  BOARD_LABELS,
  SELECTABLE_BOARDS,
  SOURCE_OPTIONS,
  createDefaultFilter,
  formatAmount,
  formatPct,
  summarizeFilter,
  summarizeFilterParts,
  toFullSymbol,
  toYi,
  type MarketFilter,
  type PresetInfo,
  type SnapshotRow,
  type UniverseResponse,
} from '@/types/universe';

const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();

const universe = useUniverseStore();
const watchlist = useWatchlistStore();
const rank = useRankStore();

const addedSymbols = ref<Set<string>>(new Set());
const addError = ref<string | null>(null);

// 行业/概念目录。借用板块轮动的全量接口，打开筛选器时行业、概念各一次请求。
const sectorCatalog = ref<SectorSummary[]>([]);
const sectorCatalogLoading = ref(false);
const sectorCatalogError = ref<string | null>(null);

function optionsFor(kind: SectorKind) {
  return sectorCatalog.value
    .filter(item => item.kind === kind)
    .map(item => ({ label: item.name, value: item.code }))
    .sort((a, b) => a.label.localeCompare(b.label, 'zh-CN'));
}

const industryOptions = computed(() => optionsFor('industry'));
const conceptOptions = computed(() => optionsFor('concept'));

async function loadSectorCatalog(force = false) {
  if (sectorCatalogLoading.value || (!force && sectorCatalog.value.length)) return;
  sectorCatalogLoading.value = true;
  sectorCatalogError.value = null;
  try {
    const response = await invoke<SectorRotation>('get_sector_rotation');
    const succeeded = new Set(response.statuses.filter(status => status.ok).map(status => status.kind));
    // 某一类短暂失败时保留上次目录，只替换本次成功的类别。
    sectorCatalog.value = [
      ...sectorCatalog.value.filter(item => !succeeded.has(item.kind)),
      ...response.items,
    ];
    const failed = response.statuses.filter(status => !status.ok);
    if (failed.length) {
      sectorCatalogError.value = failed
        .map(status => `${status.kind === 'industry' ? '行业' : '概念'}目录加载失败：${status.error ?? '未知错误'}`)
        .join('；');
    }
  } catch (error) {
    sectorCatalogError.value = `行业/概念目录加载失败：${error}`;
  } finally {
    sectorCatalogLoading.value = false;
  }
}

// 分析对话框状态
const showAnalysis = ref(false);
const analysisTarget = ref<{ symbol: string; name: string; rule: string }>({
  symbol: '',
  name: '',
  // 'auto' = 让后端按个股状态自动匹配规则
  rule: 'auto',
});

/**
 * 当前策略配套的交易规则。
 *
 * 现在只剩两个用途：
 * 1. **筛选结果表「入场」列的判据** —— 列表页要用同一把尺子横向比较，
 *    所以固定用当前策略的规则，而不是逐只自动匹配；
 * 2. 界面上提示「买卖点按 X 规则计算」。
 *
 * ⚠️ 打开个股分析**不再**用它：那里会按这只股票自身的状态自动匹配规则
 * （见 Rust `quant::playbook::match_rule`），用户在弹窗里也能手动切换。
 */
const activeRule = computed(() => {
  const preset = universe.presets.find(p => p.id === universe.activePresetId);
  return preset?.rule || 'trend_follow';
});

function openAnalysis(row: SnapshotRow) {
  analysisTarget.value = {
    symbol: toFullSymbol(row.code, row.board),
    name: row.name,
    // 交给后端按这只股票当前的状态自动挑规则，而不是沿用筛选策略配套的那条
    rule: 'auto',
  };
  showAnalysis.value = true;
}

// 推荐榜对话框状态
let rankOpenTimer: ReturnType<typeof setTimeout> | null = null;

function openRank() {
  if (rankOpenTimer) clearTimeout(rankOpenTimer);
  // 等当前按钮的 click 事件传播结束，再挂载模态层，避免它把这次点击当作遮罩点击关闭。
  rankOpenTimer = setTimeout(() => {
    rankOpenTimer = null;
    rank.open(universe.filter);
  }, 0);
}

// ── 策略（预设）管理 ──────────────────────────────────────────────
// 内置策略只读（随版本维护），用户的自建策略支持重命名 / 覆盖条件 / 删除。
// 想让内置策略「按自己口味来」的标准路径是「另存为我的策略」——
// 这样版本升级时新的内置方案还能继续拿到，用户改的那份也不受版本影响。

/** 与后端 `MAX_LABEL_CHARS` / `MAX_DESCRIPTION_CHARS` 保持一致 */
const STRATEGY_LABEL_MAX = 16;
const STRATEGY_DESC_MAX = 80;

/** 弹窗在做哪件事 —— 只影响标题与提示文案 */
type StrategyAction = 'create' | 'duplicate' | 'rename' | 'overwrite';

const strategyDialog = ref(false);
const strategyAction = ref<StrategyAction>('create');
/** 被编辑的自建策略 id；为空表示新建 */
const strategyTargetId = ref<string | null>(null);
const strategyLabel = ref('');
const strategyDescription = ref('');
/** 弹窗里将要保存的条件（新建/覆盖用当前条件，重命名沿用原条件） */
const strategyFilter = ref<MarketFilter>(createDefaultFilter());
/** 策略配套的交易规则 —— 打开个股分析时按它算买点/止损/止盈 */
const strategyRule = ref<TradeRuleId>('trend_follow');
const strategyBusy = ref(false);
const strategyError = ref<string | null>(null);
/** 操作成功后的即时反馈（保存/删除都往这里写，由模板展示） */
const strategyNotice = ref<string | null>(null);

const deleteDialog = ref(false);
const deleteTarget = ref<PresetInfo | null>(null);
const deleteBusy = ref(false);

interface StrategyStage { stage: string; status: string; detail: string; checked_at: string }
interface StrategyCard {
  id: string; name: string; source: string; current_version: number; active_version_id: number | null;
  version_id: number; status: string; rule: string; revision: number; updated_at: string; stages: StrategyStage[];
}
interface StrategyLibrary {
  cards: StrategyCard[];
  counts: { candidate: number; trial: number; rejected: number; active: number };
}
const strategyLibraryOpen = ref(false);
const strategyLibrary = ref<StrategyLibrary | null>(null);
const strategyLibraryLoading = ref(false);
const strategyLibraryError = ref<string | null>(null);

interface DynamicFilterProposal {
  mode: 'advice'; state: 'observing'; auto_eligible: false; observation_days: number;
  source: string; as_of: string; date_basis: string; filter: MarketFilter;
  candidate_limit: number; preview_count: number; preview_codes: string[];
  clamped_fields: string[]; rationale: string; guidance: string;
}
const dynamicProposal = ref<DynamicFilterProposal | null>(null);
const dynamicLoading = ref(false);
const dynamicError = ref<string | null>(null);

async function generateDynamicProposal() {
  if (dynamicLoading.value) return;
  dynamicLoading.value = true;
  dynamicError.value = null;
  try {
    dynamicProposal.value = await invoke<DynamicFilterProposal>('generate_dynamic_filter_proposal');
  } catch (error) {
    dynamicError.value = String(error);
  } finally {
    dynamicLoading.value = false;
  }
}

function applyDynamicProposal() {
  if (!dynamicProposal.value) return;
  Object.assign(universe.filter, cloneFilterLocal(dynamicProposal.value.filter));
  universe.markCustom();
  strategyNotice.value = '已由你手动采用动态建议；生成建议本身不会修改真实设置。';
}

const strategyStatusLabel: Record<string, string> = {
  candidate: '候选', trial: '试用', probation: '考核中', awaiting_confirmation: '待确认',
  active: '在用', degraded: '已降级', paused: '已暂停', rejected: '淘汰', retired: '已退役', superseded: '已替换',
};
const stageLabel: Record<string, string> = {
  static: '静态校验', causal: '因果审计', historical: '历史验证', admission: '前向前门禁',
  forward: '实时前向', probation: '考核期',
};

async function loadStrategyLibrary() {
  strategyLibraryLoading.value = true;
  strategyLibraryError.value = null;
  try {
    strategyLibrary.value = await invoke<StrategyLibrary>('strategy_library');
  } catch (error) {
    strategyLibraryError.value = String(error);
  } finally {
    strategyLibraryLoading.value = false;
  }
}

function openStrategyLibrary() {
  strategyLibraryOpen.value = true;
  void loadStrategyLibrary();
}

/** 请求删除：先记住目标，再开确认框（删错了没法恢复，必须问一句） */
function askDeleteStrategy(preset: PresetInfo) {
  deleteTarget.value = preset;
  deleteDialog.value = true;
}

function cancelDeleteStrategy() {
  deleteDialog.value = false;
  deleteTarget.value = null;
}

function cloneFilterLocal(source: MarketFilter): MarketFilter {
  return {
    ...source,
    boards: [...source.boards],
    industry_codes: [...source.industry_codes],
    concept_codes: [...source.concept_codes],
  };
}

const strategyTitle = computed(() => {
  switch (strategyAction.value) {
    case 'duplicate': return '另存为我的策略';
    case 'rename': return '重命名策略';
    case 'overwrite': return '用当前条件覆盖策略';
    default: return '新建策略';
  }
});

/** 若按当前条件保存，筛出来会是什么 —— 直接摊开给用户看，避免存下一个自己都忘了内容的策略 */
const strategySummary = computed(() => summarizeFilterParts(strategyFilter.value));

/** 只有「重命名」不改筛选条件，其余动作都会把弹窗里的条件写进去 */
const strategyChangesFilter = computed(() => strategyAction.value !== 'rename');

/** 当前选中规则的说明，让用户在保存前就知道这条规则的性格（胜率 / 盈亏比） */
const strategyRuleHint = computed(
  () => TRADE_RULE_OPTIONS.find(o => o.value === strategyRule.value)?.hint ?? '',
);

function openCreateStrategy() {
  strategyAction.value = 'create';
  strategyTargetId.value = null;
  strategyLabel.value = '';
  strategyDescription.value = '';
  strategyFilter.value = cloneFilterLocal(universe.filter);
  // 新建时继承当前策略的规则 —— 用户通常是在延续同一种打法
  strategyRule.value = activeRule.value as TradeRuleId;
  strategyError.value = null;
  strategyDialog.value = true;
}

function openRenameStrategy(preset: PresetInfo) {
  strategyAction.value = 'rename';
  strategyTargetId.value = preset.id;
  strategyLabel.value = preset.label;
  strategyDescription.value = preset.description;
  strategyFilter.value = cloneFilterLocal(preset.filter);
  strategyRule.value = (preset.rule || 'trend_follow') as TradeRuleId;
  strategyError.value = null;
  strategyDialog.value = true;
}

function openOverwriteStrategy(preset: PresetInfo) {
  strategyAction.value = 'overwrite';
  strategyTargetId.value = preset.id;
  strategyLabel.value = preset.label;
  strategyDescription.value = preset.description;
  // 覆盖：条件取「当前界面上的条件」，名称、说明与规则沿用原策略
  strategyFilter.value = cloneFilterLocal(universe.filter);
  strategyRule.value = (preset.rule || 'trend_follow') as TradeRuleId;
  strategyError.value = null;
  strategyDialog.value = true;
}

function openDuplicateStrategy(preset: PresetInfo) {
  strategyAction.value = 'duplicate';
  strategyTargetId.value = null;
  strategyLabel.value = `${preset.label} 副本`;
  strategyDescription.value = preset.description;
  strategyFilter.value = cloneFilterLocal(preset.filter);
  strategyRule.value = (preset.rule || 'trend_follow') as TradeRuleId;
  strategyError.value = null;
  strategyDialog.value = true;
}

/** 每个策略的「⋯」菜单。内置策略只给「另存为」，自建策略才有改名与删除 */
function strategyMenu(preset: PresetInfo): DropdownOption[] {
  if (preset.builtin) {
    return [{ label: '另存为我的策略', key: 'duplicate' }];
  }
  return [
    { label: '重命名 / 改说明', key: 'rename' },
    { label: '用当前条件覆盖', key: 'overwrite' },
    { label: '另存为副本', key: 'duplicate' },
    { type: 'divider', key: 'divider' },
    { label: '删除', key: 'delete' },
  ];
}

function onStrategyMenu(key: string, preset: PresetInfo) {
  switch (key) {
    case 'rename': openRenameStrategy(preset); break;
    case 'overwrite': openOverwriteStrategy(preset); break;
    case 'duplicate': openDuplicateStrategy(preset); break;
    case 'delete': askDeleteStrategy(preset); break;
    default: break;
  }
}

async function submitStrategy() {
  const label = strategyLabel.value.trim();
  if (!label) {
    strategyError.value = '策略名称不能为空';
    return;
  }
  if (label.length > STRATEGY_LABEL_MAX) {
    strategyError.value = `策略名称最多 ${STRATEGY_LABEL_MAX} 个字，当前 ${label.length} 个`;
    return;
  }
  const description = strategyDescription.value.trim();
  if (description.length > STRATEGY_DESC_MAX) {
    strategyError.value = `策略说明最多 ${STRATEGY_DESC_MAX} 个字，当前 ${description.length} 个`;
    return;
  }
  // 本地先拦一道，用户不用等一次往返才知道重名
  const clash = universe.presetNameTaken(label, strategyTargetId.value ?? undefined);
  if (clash) {
    strategyError.value = `已有同名策略「${clash}」，换个名字吧`;
    return;
  }

  strategyBusy.value = true;
  strategyError.value = null;
  try {
    const saved = await universe.savePreset({
      id: strategyTargetId.value ?? undefined,
      label,
      description,
      rule: strategyRule.value,
      filter: strategyFilter.value,
    });
    strategyDialog.value = false;
    strategyNotice.value = `已保存策略「${saved.label}」`;
  } catch (e) {
    // 后端的校验信息（重名、超长、内置不可改）直接透给用户，不要吞掉
    strategyError.value = `保存失败：${e}`;
  } finally {
    strategyBusy.value = false;
  }
}

async function confirmDeleteStrategy() {
  const target = deleteTarget.value;
  if (!target) {
    deleteDialog.value = false;
    return;
  }
  deleteBusy.value = true;
  try {
    await universe.deletePreset(target.id);
    strategyNotice.value = `已删除策略「${target.label}」`;
    deleteDialog.value = false;
  } catch (e) {
    // 删除失败也把确认框收掉，错误走统一提示区，避免弹窗卡住
    strategyError.value = `删除失败：${e}`;
    deleteDialog.value = false;
  } finally {
    deleteBusy.value = false;
    deleteTarget.value = null;
  }
}

const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

/**
 * 全量评分每次交给后端的股票数。
 *
 * 与后端 `batch_stock_status` 的单次上限（120）对齐并留点余量。
 * 分批只为了让进度条能走、单次请求不至于过大 —— 并发本身由 Rust 侧控制。
 */
const SCORE_BATCH = 100;
const scoreByRow = ref<Map<string, number | null>>(new Map());
const scoreSortActive = ref(false);
const scoring = ref(false);
const scoreProgress = ref({ done: 0, total: 0 });
const scoreError = ref<string | null>(null);
const rankedRows = ref<SnapshotRow[] | null>(null);
const rankedPage = ref(1);
const rankedPageSize = ref(20);
let scoreRun = 0;

/**
 * 表格行的唯一键。
 *
 * 用**完整符号**（sh600519）而不是 `board:code`：后端 `batch_stock_status` 回传的
 * 就是这个形式的 symbol，两边对齐就不用再做一次映射。
 */
function scoreKey(row: SnapshotRow): string {
  return toFullSymbol(row.code, row.board);
}

function scoreOf(row: SnapshotRow): number {
  return scoreByRow.value.get(scoreKey(row)) ?? -1;
}

// ── 当前页「评分 + 入场」批量状态 ────────────────────────────────
// 「入场」列要直接展示「当前是否满足入场条件」，不能让用户逐只点开分析才知道。
// 一页最多 100 只，若前端逐只调 `analyze_stock` 就要发 100 次 IPC；
// 交给 Rust 侧并发拉日 K（见 commands/analysis.rs 的 batch_stock_status），一次调用搞定。
const statusByRow = ref<Map<string, StockStatusItem>>(new Map());
const statusLoading = ref(false);
const statusError = ref<string | null>(null);
let statusRun = 0;
let statusTimer: ReturnType<typeof setTimeout> | undefined;

/** 本页还没算过状态的股票 */
function pendingStatusRows(): SnapshotRow[] {
  return displayedRows.value.filter(row => !statusByRow.value.has(scoreKey(row)));
}

/**
 * 算当前页的评分与入场状态。
 *
 * 规则（`activeRule`）必须与当前策略配套，否则会出现「用超跌反弹策略选出来、
 * 却按趋势规则判断入场」的错配 —— 与打开个股分析时的处理一致。
 */
async function scanPageStatus() {
  const rows = pendingStatusRows();
  if (!rows.length) return;

  const run = ++statusRun;
  const rule = activeRule.value;
  statusLoading.value = true;
  statusError.value = null;

  try {
    const items = await invoke<StockStatusItem[]>('batch_stock_status', {
      symbols: rows.map(row => scoreKey(row)),
      rule,
    });
    // 换页 / 换策略 / 重新筛选都会让这一批作废，直接丢弃即可
    if (run !== statusRun || rule !== activeRule.value) return;

    const bySymbol = new Map(items.map(item => [item.symbol, item]));
    const nextScore = new Map(scoreByRow.value);
    const nextStatus = new Map(statusByRow.value);
    let failed = 0;
    for (const row of rows) {
      const key = scoreKey(row);
      const item = bySymbol.get(key);
      if (!item) continue;
      nextStatus.set(key, item);
      if (item.score != null) nextScore.set(key, item.score);
      if (item.error) failed += 1;
    }
    scoreByRow.value = nextScore;
    statusByRow.value = nextStatus;
    // 部分失败不静默：K 线不足 / 限流都可能只影响几只，说清楚比让人以为「全都没机会」好
    if (failed) {
      statusError.value = `${rows.length} 只里有 ${failed} 只没算出入场状态（多为上市时间短、K 线不足或数据源限流）`;
    }
  } catch (e) {
    if (run === statusRun) {
      statusError.value = `计算入场状态失败：${e}`;
      console.error('[universe] batch_stock_status failed:', e);
    }
  } finally {
    if (run === statusRun) statusLoading.value = false;
  }
}

/** 防抖：快速翻页时只算最后停下来的那一页 */
function scheduleStatusScan(delay = 250) {
  if (statusTimer) clearTimeout(statusTimer);
  statusTimer = setTimeout(() => {
    statusTimer = undefined;
    void scanPageStatus();
  }, delay);
}

// ── 弹窗尺寸：可拖拽缩放 + 自动记忆 + 跟随窗口 ───────────────────
// 固定尺寸看不下更多行，所以宽高放开给用户拖；拖完存进 settings（与筛选条件同一套 KV）。
// 窗口变小时弹窗必须跟着收敛，否则会有一部分跑到屏幕外、连关闭按钮都点不到。
const SIZE_SETTING_KEY = 'universe_dialog_size';
/** 缩放下限。低于这个尺寸筛选面板会挤成一团，不如不许拖 */
const MIN_PANEL_W = 640;
const MIN_PANEL_H = 400;
/** 视口留白：四周留一点，保证遮罩可见、卡片不至于贴边 */
const VIEWPORT_GUTTER_W = 24;
const VIEWPORT_GUTTER_H = 56;

/** 用户设定的尺寸（px）。null = 还没定过，用视口默认值 */
const prefWidth = ref<number | null>(null);
const prefHeight = ref<number | null>(null);
/** 视口尺寸，随 window.resize 更新 */
const viewport = ref({ w: window.innerWidth, h: window.innerHeight });
const resizing = ref(false);

/**
 * 筛选条件面板是否收起。
 *
 * 窗口小时，条件面板（板块 + 排除 + 11 组数值区间）会占掉大半高度，
 * 结果表被挤到只剩几十像素；表体一矮，它自己的**横向**滚动条就被顶到
 * 看不见的地方 —— 用户想左右挪一下看后面的指标，得先把整块滚到底、
 * 挪完再滚回来。收起条件是最直接的解法。
 *
 * 收起不会让人"不知道自己筛的是什么"：策略条上的「当前条件」摘要一直在。
 */
const filterCollapsed = ref(false);

const sizeLimits = computed(() => {
  const maxW = Math.max(320, viewport.value.w - VIEWPORT_GUTTER_W);
  const maxH = Math.max(280, viewport.value.h - VIEWPORT_GUTTER_H);
  return {
    // 下限也要收敛进视口：窗口比下限还小时，否则会算出「min > max」的死锁
    minW: Math.min(MIN_PANEL_W, maxW),
    minH: Math.min(MIN_PANEL_H, maxH),
    maxW,
    maxH,
  };
});

const panelWidth = computed(() => {
  const l = sizeLimits.value;
  const want = prefWidth.value ?? Math.min(1120, l.maxW);
  return Math.round(Math.min(Math.max(want, l.minW), l.maxW));
});

const panelHeight = computed(() => {
  const l = sizeLimits.value;
  // 默认高度取视口的 86% —— 够大，又不会顶到屏幕边缘
  const want = prefHeight.value ?? Math.min(Math.round(viewport.value.h * 0.86), l.maxH);
  return Math.round(Math.min(Math.max(want, l.minH), l.maxH));
});

const modalStyle = computed(() => ({
  width: `${panelWidth.value}px`,
  height: `${panelHeight.value}px`,
}));

/**
 * 右下角缩放柄：pointerdown 时记下起点，拖动期间实时改 `prefWidth/prefHeight`。
 *
 * 注意这里拖的是**用户意愿值**，显示时再按视口 clamp（见 panelWidth/panelHeight）。
 * 这样窗口临时变小不会把用户的设定改小，窗口恢复后尺寸也跟着回来。
 */
function startResize(event: PointerEvent) {
  if (event.button !== 0) return;
  const startX = event.clientX;
  const startY = event.clientY;
  const startW = panelWidth.value;
  const startH = panelHeight.value;
  resizing.value = true;

  const onMove = (e: PointerEvent) => {
    const l = sizeLimits.value;
    prefWidth.value = Math.min(Math.max(startW + (e.clientX - startX), l.minW), l.maxW);
    prefHeight.value = Math.min(Math.max(startH + (e.clientY - startY), l.minH), l.maxH);
  };
  const onUp = () => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    resizing.value = false;
    void persistSize();
  };
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
}

async function persistSize() {
  if (prefWidth.value === null && prefHeight.value === null) return;
  try {
    await invoke('set_setting', {
      key: SIZE_SETTING_KEY,
      value: JSON.stringify({ width: prefWidth.value, height: prefHeight.value }),
    });
  } catch (e) {
    // 存不下只是下次打开回到默认尺寸，不影响本次使用，记一笔就够了
    console.warn('[universe] 保存筛选器尺寸失败:', e);
  }
}

async function loadSize() {
  try {
    const all = await invoke<Record<string, string>>('get_settings');
    const raw = all[SIZE_SETTING_KEY];
    if (!raw) return;
    const parsed = JSON.parse(raw) as { width?: unknown; height?: unknown };
    // 宽松解析：脏数据不能把弹窗变成 0 尺寸
    if (typeof parsed.width === 'number' && Number.isFinite(parsed.width)) {
      prefWidth.value = parsed.width;
    }
    if (typeof parsed.height === 'number' && Number.isFinite(parsed.height)) {
      prefHeight.value = parsed.height;
    }
  } catch (e) {
    console.warn('[universe] 恢复筛选器尺寸失败:', e);
  }
}

function handleViewportChange() {
  viewport.value = { w: window.innerWidth, h: window.innerHeight };
}

// ── 结果表高度：跟着弹窗长高，而不是写死 420px ───────────────────
// ⚠️ 这里用 ResizeObserver 量出实际可用高度，再喂给 data-table 的 max-height。
// 不用 virtual-scroll / flex-height：那两个都要求外层高度被正确算出来，
// 一旦算出 0 表格就是一片空白（这个坑踩过，见表格处的注释）。
//
// ⚠️ 量出来的高度要**扣掉分页条**：分页在 n-data-table 内部，但它不占 max-height
// 的额度 —— 不扣的话表体 + 分页会比容器高出一截，`.table-wrap` 又是 overflow:hidden，
// 于是表体底部那条**横向滚动条**（横向滚动的滚动条就渲染在表体最下方）
// 会被直接裁掉，表现就是「窗口小时下面没有左右滚轮」。
//
// 这个高度**优先量真实的 DOM**：早先写死 52（按小号分页约 28px + 留白估的），
// 但分页控件在窄弹窗下会换行变高，估出来的值就偏小 → 又裁到滚动条。
// 量不到（分页还没渲染）才退回常量。
const TABLE_PAGINATION_H = 52;
/** 表体最小高度兜底（`.table-wrap` 的 min-height 112 − 分页 52 = 60） */
const TABLE_MIN_H = 60;
const tableWrapRef = ref<HTMLElement | null>(null);
const tableMaxHeight = ref(420);
let tableObserver: ResizeObserver | null = null;

/** 分页条真实占位高度（含上下外边距）；量不到就用常量兜底 */
function paginationHeight(el: HTMLElement): number {
  const pager = el.querySelector<HTMLElement>('.n-data-table__pagination');
  if (!pager) return TABLE_PAGINATION_H;
  const style = getComputedStyle(pager);
  const margins = (parseFloat(style.marginTop) || 0) + (parseFloat(style.marginBottom) || 0);
  const total = Math.round(pager.getBoundingClientRect().height + margins);
  return total > 0 ? total : TABLE_PAGINATION_H;
}

/**
 * 按容器实际高度重算表体 max-height。
 *
 * 只在变化超过 1px 时写入，避免 ResizeObserver ↔ 响应式更新互相触发形成抖动。
 * 容器的 flex:1 高度不受 max-height 影响，所以这里不会自激。
 */
function syncTableHeight() {
  const el = tableWrapRef.value;
  if (!el) return;
  const height = el.clientHeight;
  if (height <= 0) return;
  const next = Math.max(TABLE_MIN_H, Math.round(height) - paginationHeight(el));
  if (Math.abs(next - tableMaxHeight.value) > 1) tableMaxHeight.value = next;
}

function observeTableWrap() {
  tableObserver?.disconnect();
  tableObserver = null;
  const el = tableWrapRef.value;
  if (!el || typeof ResizeObserver === 'undefined') return;
  tableObserver = new ResizeObserver(() => syncTableHeight());
  tableObserver.observe(el);
  syncTableHeight();
}

watch(
  () => universe.resultsVisible,
  async visible => {
    if (!visible) {
      tableObserver?.disconnect();
      tableObserver = null;
      return;
    }
    await nextTick();
    observeTableWrap();
  },
);

// 分页条要等数据和分页状态渲染出来才存在：等它出现后再量一次，
// 否则会一直沿用「量不到 → 常量 52」的估值。
watch(
  () => [universe.rows.length, universe.totalMatched, universe.page] as const,
  async () => {
    if (!universe.resultsVisible) return;
    await nextTick();
    syncTableHeight();
  },
);

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

// 表格可见、页面内容变化、策略规则变化 —— 这三种情况都需要重新判断入场。
// ⚠️ 必须放在 `displayedRows` 之后：watch 建立时会先跑一次 getter 收集依赖，
// 提前定义会撞上 const 的暂存死区。
watch(
  () => [
    universe.resultsVisible,
    displayedRows.value.map(row => scoreKey(row)).join('|'),
    activeRule.value,
  ],
  () => {
    if (!universe.resultsVisible) return;
    scheduleStatusScan();
  },
);

// 换策略：入场是按规则算的，评分与入场状态全部作废
watch(activeRule, () => {
  statusRun += 1;
  statusByRow.value = new Map();
  statusLoading.value = false;
  statusError.value = null;
});

watch(() => universe.rows, () => {
  scoreRun += 1;
  scoreByRow.value = new Map();
  scoreSortActive.value = false;
  scoring.value = false;
  scoreError.value = null;
  rankedRows.value = null;
  rankedPage.value = 1;
  // 换了一批筛选结果，上一批的评分与入场状态一律作废
  statusRun += 1;
  statusByRow.value = new Map();
  statusLoading.value = false;
  statusError.value = null;
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

    // 分批交给后端：`batch_stock_status` 在 Rust 侧用信号量控制并发（5），
    // 一次 IPC 出整批结果 —— 比前端逐只调 `analyze_stock` 少发几十倍的调用，
    // 也顺带避开了「逐只完整分析会连股本信息一起拉」的开销。
    const nextStatus = new Map(statusByRow.value);
    try {
      for (let start = 0; start < rows.length; start += SCORE_BATCH) {
        if (run !== scoreRun) return;
        const batch = rows.slice(start, start + SCORE_BATCH);
        const items = await invoke<StockStatusItem[]>('batch_stock_status', {
          symbols: batch.map(row => toFullSymbol(row.code, row.board)),
          rule: activeRule.value,
        });
        if (run !== scoreRun) return;

        const bySymbol = new Map(items.map(item => [item.symbol, item]));
        for (const row of batch) {
          const key = scoreKey(row);
          const item = bySymbol.get(key);
          next.set(key, item?.score ?? null);
          // 顺手把入场状态也存上 —— 表格翻到这一页时就不必再算一遍
          if (item) nextStatus.set(key, item);
        }
        scoreByRow.value = new Map(next);
        statusByRow.value = new Map(nextStatus);
        scoreProgress.value = {
          done: Math.min(start + SCORE_BATCH, rows.length),
          total: rows.length,
        };
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
  void loadSectorCatalog();
  // 恢复上次拖出来的弹窗尺寸。与筛选条件分开读：尺寸坏了不该拖累条件恢复
  void loadSize();
  window.addEventListener('resize', handleViewportChange);
});

onBeforeUnmount(() => {
  // 关闭对话框时立即落盘：防抖的 400ms 可能在用户直接退出程序时被带走
  universe.flushPersist();
  window.removeEventListener('resize', handleViewportChange);
  if (statusTimer) {
    clearTimeout(statusTimer);
    statusTimer = undefined;
  }
  // 推荐榜的延迟挂载定时器也要清 —— 否则卸载后才触发，会去开一个已经没人管的模态
  if (rankOpenTimer) clearTimeout(rankOpenTimer);
  tableObserver?.disconnect();
  tableObserver = null;
});

const activeDesc = computed(
  () =>
    universe.presets.find(p => p.id === universe.activePresetId)?.description ??
    '自定义条件：不套用预设，完全按下方勾选与数值区间筛选',
);

/**
 * 当前**实际生效**的条件摘要。
 *
 * 策略名只有四个字，光看名字根本不知道它筛什么；手动微调之后更是没处对照。
 * 所以这里始终展示实时条件，用户随时能确认「我现在到底在筛什么」。
 */
const conditionSummary = computed(() => summarizeFilter(universe.filter));

/** 涨跌配色：A 股习惯 —— 红涨绿跌 */
function changeClass(value: number): string {
  if (value > 0) return 'up';
  if (value < 0) return 'down';
  return 'flat';
}

function num(value: number, digits = 2): string {
  return Number.isFinite(value) ? value.toFixed(digits) : '-';
}

function setListedDays(max: number | null) {
  universe.filter.listed_days_min = null;
  universe.filter.listed_days_max = max;
  universe.markCustom();
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
    // 「当前是否满足入场条件」直接摊在列表里 —— 不用逐只点开分析才知道。
    // 判断用的是当前策略配套的规则（与打开个股分析时同一条规则）。
    title: '入场',
    key: 'entry',
    width: 96,
    align: 'center',
    render: row => {
      const item = statusByRow.value.get(scoreKey(row));
      if (!item) {
        // 还没轮到它算：给个「计算中」而不是装作没有这回事
        return h('span', { class: 'muted' }, statusLoading.value ? '计算中' : '--');
      }
      if (item.ready === true) {
        const history = item.history ? `；${item.history.source_label} ${item.history.start_date ?? '—'}→${item.history.end_date ?? '—'}，${item.history.bars}根` : '';
        const range =
          item.buy_low != null && item.buy_high != null
            ? `，买入区间 ${num(item.buy_low)}–${num(item.buy_high)}`
            : '';
        return h(
          'span',
          {
            class: 'entry-ok',
            title: `当前满足「${tradeRuleLabel(activeRule.value)}」的入场条件${range}${history}`,
          },
          '满足',
        );
      }
      if (item.ready === false) {
        const history = item.history ? `；${item.history.source_label} ${item.history.bars}根` : '';
        return h(
          'span',
          {
            class: 'entry-no',
            title: (item.waiting_for ? `当前未触发：${item.waiting_for}` : '当前未触发') + history,
          },
          '未触发',
        );
      }
      // ready === null：K 线不足 / 数据源失败，算不出就是算不出
      return h('span', { class: 'muted', title: item.error ?? '数据不足，无法判断' }, '--');
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
    // 「这只票是走什么板块涨/跌的」第一层答案：东财 f100 行业。
    // 替代原来的交易所板块列（沪/深/创/科/北）—— 那个信息由代码前缀即可判断，
    // 占一列不值；行业才是选股时真正在看的维度。
    title: '行业',
    key: 'industry',
    width: 108,
    ellipsis: { tooltip: true },
    render: row => h('span', { class: 'muted' }, row.industry || '--'),
  },
  {
    // 第二层答案：东财 f103 概念。一只票通常同时属于多个概念，
    // 只展示前 2 个、其余折叠成「+N」并整串放进 tooltip ——
    // 全量平铺会把行撑高、也会把后面的列挤出可视区。
    title: '概念',
    key: 'concepts',
    width: 232,
    render: row => {
      const all = row.concepts ?? [];
      if (!all.length) return h('span', { class: 'muted' }, '--');
      const head = all.slice(0, 2).join(' · ');
      const more = all.length > 2 ? ` +${all.length - 2}` : '';
      return h(
        'span',
        { class: 'concept-cell', title: `${all.length} 个概念：${all.join(' · ')}` },
        `${head}${more}`,
      );
    },
  },
  {
    // 「加自选 / 分析」固定在右侧：这张表 13 列、横向要滚，
    // 不固定的话窄弹窗下这两个按钮要滚动才看得见（操作类按钮不该藏起来）
    title: '操作',
    key: 'action',
    width: 132,
    align: 'center',
    fixed: 'right',
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

/**
 * 表格横向滚动宽度 —— **由列宽求和得出**，不再手写常量。
 *
 * 之前写死 1224：列宽一改就和真实宽度脱节 —— 写小了最后一列被挤掉、
 * 写大了右侧留一截空白，两种情况都会表现成「滚动条拉到底也看不全后面的东西」。
 * 加 20px 余量吸收单元格边框与内边距。
 */
const scrollX = computed(() =>
  columns.value.reduce(
    (sum, column) => sum + (typeof column.width === 'number' ? column.width : 100),
    0,
  ) + 20,
);
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="全市场筛选器"
    :class="['universe-screener-modal', { 'is-resizing': resizing }]"
    :style="modalStyle"
    :content-style="{ position: 'relative', display: 'flex', flexDirection: 'column', overflow: 'hidden' }"
    :bordered="false"
    size="small"
  >
    <!--
      两层 div 是刻意的：
      - `.screener-scroll` 是唯一的滚动区（筛选面板高时整体可滚）
      - `.screener` 负责内部 flex 布局，让表格吃掉剩余高度
      缩放柄要相对弹窗卡片定位（见下面 .size-grip），所以它必须是滚动区的**兄弟**，
      否则内容一滚就跟着滚走了。
    -->
    <div class="screener-scroll"><div class="screener">
      <!-- 策略条：内置策略只读，自建策略可改名 / 覆盖 / 删除 -->
      <div class="preset-bar">
        <div class="chips">
          <div
            v-for="preset in universe.presets"
            :key="preset.id"
            class="preset-chip"
            :class="{ active: universe.activePresetId === preset.id, mine: !preset.builtin }"
            :title="`${preset.description}\n\n条件：${summarizeFilter(preset.filter)}\n交易规则：${tradeRuleLabel(preset.rule)}`"
          >
            <button class="preset-chip-label" @click="universe.applyPreset(preset.id)">
              {{ preset.label }}<sup v-if="preset.strategy_version">v{{ preset.strategy_version }}</sup>
            </button>
            <n-dropdown
              trigger="click"
              size="small"
              :options="strategyMenu(preset)"
              @select="key => onStrategyMenu(String(key), preset)"
            >
              <button class="preset-chip-more" :title="`管理「${preset.label}」`">⋯</button>
            </n-dropdown>
          </div>

          <button
            class="chip chip-add"
            title="把当前条件存成一个自己的策略，下次一键切回来"
            @click="openCreateStrategy"
          >
            ＋ 新建策略
          </button>
          <button class="chip" title="查看策略版本、验证门禁和生命周期" @click="openStrategyLibrary">
            策略闭环
          </button>
          <button class="chip" title="仅生成隔离建议，不自动修改当前筛选条件" :disabled="dynamicLoading" @click="generateDynamicProposal">
            {{ dynamicLoading ? 'Agent 生成中…' : '动态参数建议' }}
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
        <div class="preset-meta">
          <span class="preset-desc">{{ activeDesc }}</span>
          <span class="preset-cond" :title="`${conditionSummary}`">
            当前条件：{{ conditionSummary }}
            <em class="preset-rule">· 「入场」列按「{{ tradeRuleLabel(activeRule) }}」判断；点「分析」会按个股状态自动匹配规则</em>
          </span>
        </div>
      </div>

      <!-- 策略操作反馈：成败都要说出来，不能静默 -->
      <div v-if="strategyNotice" class="notice-line notice-ok">
        {{ strategyNotice }}
        <button class="link-btn" @click="strategyNotice = null">知道了</button>
      </div>
      <div v-if="dynamicError" class="notice-line chip-error">动态建议失败：{{ dynamicError }}</div>
      <div v-if="dynamicProposal" class="dynamic-advice">
        <div><b>动态参数 · 仅建议</b><span>{{ dynamicProposal.as_of }} · 候选 {{ dynamicProposal.preview_count }}/{{ dynamicProposal.candidate_limit }}</span></div>
        <p>{{ dynamicProposal.rationale }}</p>
        <small>{{ dynamicProposal.guidance }}<template v-if="dynamicProposal.clamped_fields.length"> 已夹限：{{ dynamicProposal.clamped_fields.join('、') }}</template></small>
        <n-button size="tiny" type="primary" secondary @click="applyDynamicProposal">由我手动采用</n-button>
      </div>

      <!-- 筛选条件 -->
      <div v-show="!filterCollapsed" class="filter-panel">
        <div class="field">
          <span class="field-label">板块</span>
          <n-checkbox-group v-model:value="universe.filter.boards" @update:value="universe.markCustom">
            <n-checkbox v-for="b in SELECTABLE_BOARDS" :key="b" :value="b" :label="BOARD_LABELS[b]" />
          </n-checkbox-group>
        </div>

        <div class="field sector-filter-row">
          <span class="field-label">行业</span>
          <n-select
            v-model:value="universe.filter.industry_codes"
            class="sector-filter-select"
            :options="industryOptions"
            :loading="sectorCatalogLoading"
            multiple
            filterable
            clearable
            max-tag-count="responsive"
            :max="10"
            placeholder="不限行业（可多选）"
            @update:value="universe.markCustom"
          />
          <span class="field-label concept-label">概念</span>
          <n-select
            v-model:value="universe.filter.concept_codes"
            class="sector-filter-select"
            :options="conceptOptions"
            :loading="sectorCatalogLoading"
            multiple
            filterable
            clearable
            max-tag-count="responsive"
            :max="10"
            placeholder="不限概念（可多选）"
            @update:value="universe.markCustom"
          />
          <n-button
            v-if="sectorCatalogError"
            size="tiny"
            quaternary
            type="warning"
            :title="sectorCatalogError"
            :loading="sectorCatalogLoading"
            @click="loadSectorCatalog(true)"
          >目录加载失败，重试</n-button>
        </div>

        <div class="field">
          <span class="field-label">排除</span>
          <div class="checks">
            <n-checkbox v-model:checked="universe.filter.exclude_st" @update:checked="universe.markCustom">ST</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.exclude_delisting" @update:checked="universe.markCustom">退市</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.exclude_suspended" @update:checked="universe.markCustom">停牌</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.exclude_limit_locked" @update:checked="universe.markCustom">一字板</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.exclude_cdr" @update:checked="universe.markCustom">存托凭证</n-checkbox>
          </div>
        </div>

        <div class="ranges">
          <div class="range">
            <span class="range-label">上市天数</span>
            <n-input-number v-model:value="universe.filter.listed_days_min" size="small" :show-button="false" :precision="0" :min="0" :disabled="!universe.listingDateSupported" placeholder="不限" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.listed_days_max" size="small" :show-button="false" :precision="0" :min="0" :disabled="!universe.listingDateSupported" placeholder="不限" clearable @update:value="universe.markCustom" />
            <n-button size="tiny" quaternary :disabled="!universe.listingDateSupported" @click="setListedDays(30)">新股</n-button>
            <n-button size="tiny" quaternary :disabled="!universe.listingDateSupported" @click="setListedDays(365)">次新</n-button>
            <n-button size="tiny" quaternary @click="setListedDays(null)">不限</n-button>
          </div>

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

          <!-- 60 日涨跌幅：趋势 / 反转类策略的核心维度，只有东财通道提供 -->
          <div class="range">
            <span
              class="range-label"
              title="近 60 个交易日的涨跌幅。趋势与反转类策略的依据，仅东方财富通道提供；新浪通道下该条件会被自动忽略"
            >60日涨跌幅(%)</span>
            <n-input-number
              v-model:value="universe.filter.change_60d_min"
              size="small"
              :show-button="false"
              :disabled="!universe.change60dSupported"
              :placeholder="universe.change60dSupported ? '不限' : '数据源不支持'"
              clearable
              @update:value="universe.markCustom"
            />
            <span class="tilde">~</span>
            <n-input-number
              v-model:value="universe.filter.change_60d_max"
              size="small"
              :show-button="false"
              :disabled="!universe.change60dSupported"
              :placeholder="universe.change60dSupported ? '不限' : '数据源不支持'"
              clearable
              @update:value="universe.markCustom"
            />
          </div>

          <div class="range">
            <span class="range-label" title="动态市盈率。下限填 0.01 即「只要盈利股」——亏损股该值为 0 或负">市盈率(动)</span>
            <n-input-number v-model:value="universe.filter.pe_min" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.pe_max" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label" title="市净率。A 股实证里 PB 是最稳健的估值因子 —— 低 PB 长期胜率高于低 PE">市净率</span>
            <n-input-number v-model:value="universe.filter.pb_min" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.pb_max" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
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
            <span class="range-label" title="当日振幅，作为波动率的粗略代理（不是 60 日波动率）">振幅 ≤(%)</span>
            <n-input-number v-model:value="universe.filter.amplitude_max" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label">成交额 ≥(万)</span>
            <n-input-number v-model:value="universe.filter.amount_min_wan" size="small" :show-button="false" placeholder="不限" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label" title="快照粗筛后，仅对成交额前 120 只候选读取本地历史">站上均线 MA</span>
            <n-input-number v-model:value="universe.filter.above_ma_days" size="small" :show-button="false" :precision="0" :min="2" :max="250" placeholder="日数" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label">N 日新高</span>
            <n-input-number v-model:value="universe.filter.new_high_days" size="small" :show-button="false" :precision="0" :min="2" :max="250" placeholder="日数" clearable @update:value="universe.markCustom" />
          </div>

          <div class="range">
            <span class="range-label">距低点涨幅</span>
            <n-input-number v-model:value="universe.filter.rise_from_low_days" size="small" :show-button="false" :precision="0" :min="2" :max="250" placeholder="日数" clearable @update:value="universe.markCustom" />
            <n-input-number v-model:value="universe.filter.rise_from_low_min" size="small" :show-button="false" placeholder="最小%" clearable @update:value="universe.markCustom" />
            <span class="tilde">~</span>
            <n-input-number v-model:value="universe.filter.rise_from_low_max" size="small" :show-button="false" placeholder="最大%" clearable @update:value="universe.markCustom" />
          </div>
        </div>

        <div class="field">
          <span class="field-label" title="仅使用本地历史；本地不可用时自动跳过并提示">历史技术</span>
          <div class="checks">
            <n-checkbox v-model:checked="universe.filter.macd_bullish" @update:checked="universe.markCustom">MACD 多头</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.kdj_bullish" @update:checked="universe.markCustom">KDJ 多头</n-checkbox>
            <n-checkbox v-model:checked="universe.filter.volume_price_rising" @update:checked="universe.markCustom">量价齐升</n-checkbox>
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
        <!-- 收起条件：窗口小时给结果表腾高度，横向滚动条才不会被挤出视野 -->
        <n-button
          size="small"
          quaternary
          :title="filterCollapsed ? '展开筛选条件面板' : '收起筛选条件面板，给结果表让出高度'"
          @click="filterCollapsed = !filterCollapsed"
        >
          {{ filterCollapsed ? '展开条件' : '收起条件' }}
        </n-button>
        <!-- 推荐榜走独立的数据获取与加载状态：不受筛选器快照加载（universe.loading）影响，
             只在榜单自身生成期间禁用并显示进度。 -->
        <n-button
          size="small"
          type="warning"
          secondary
          :loading="rank.loading"
          :disabled="rank.loading"
          @click="openRank"
        >
          {{ rank.loading ? '正在生成推荐榜…' : '生成推荐榜' }}
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

        <!-- 入场状态是逐只拉日 K 算的，说一声「还在算」，别让用户以为列坏了 -->
        <n-tag v-if="universe.resultsVisible && statusLoading" size="small" :bordered="false">
          正在判断本页入场条件…
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
      <div v-if="universe.hasLoaded && universe.historyNotice" class="notice-line">
        {{ universe.historyNotice }}
      </div>

      <div v-if="statusError" class="notice-line">{{ statusError }}</div>

      <div v-if="universe.error" class="error-line">{{ universe.error }}</div>
      <div v-if="scoreError" class="error-line">{{ scoreError }}</div>
      <!-- 行业 / 概念只有东财 clist 提供：新浪通道下两列会全是 ——，
           必须明说「是通道的问题」，否则会被当成这两列坏了 -->
      <div v-if="universe.resultsVisible && !universe.sectorSupported" class="channel-hint">
        当前通道（{{ universe.sourceLabel }}）不提供「行业 / 概念」，这两列显示 ——。
        把取数通道切到「东方财富」即可看到每只票的行业与所属概念。
      </div>

      <!-- 结果表：筛完先只显示统计，点「展示数据」才出现 -->
      <div v-if="universe.resultsVisible" ref="tableWrapRef" class="table-wrap">
        <!--
          这里刻意**不用 virtual-scroll**：虚拟滚动要求外层容器有确定高度，
          一旦布局计算出 0 高度，表格会渲染成一片空白 —— 表现就是「点了筛选什么都没有」。
          也不用 flex-height：同属"依赖外层高度"的方案（曾导致行不渲染）。
          服务端分页每页最多 100 行，用固定 max-height + 内部滚动最稳。

          max-height 现在是个**具体数值**（tableMaxHeight），由 ResizeObserver 量出
          这块区域的实际高度得到 —— 弹窗拖大，表格跟着变大；仍然不是 CSS 百分比，
          所以不会退化成上面那两种「依赖外层高度」的写法。
          该数值已扣掉分页条高度，因此表体 + 分页恰好等于这块区域，
          不会溢出到外面去（溢出会把横向滚动条挤到看不见的地方）。
        -->
        <n-data-table
          :columns="columns"
          :data="displayedRows"
          :loading="universe.loading || universe.pageLoading"
          :row-key="rowKey"
          size="small"
          :remote="true"
          :max-height="tableMaxHeight"
          :scroll-x="scrollX"
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
    </div></div>

    <!-- 右下角缩放柄：拖动改宽高，松手自动记住 -->
    <div
      class="size-grip"
      :class="{ active: resizing }"
      title="拖动调整窗口大小（会自动记住）"
      @pointerdown.prevent="startResize"
    />
  </n-modal>

  <n-modal
    v-model:show="strategyLibraryOpen"
    preset="card"
    title="策略闭环"
    :style="{ width: 'min(900px, calc(100vw - 24px))' }"
    :bordered="false"
  >
    <div class="strategy-library-summary">
      <n-tag>候选 {{ strategyLibrary?.counts.candidate ?? 0 }}</n-tag>
      <n-tag type="warning">试用 {{ strategyLibrary?.counts.trial ?? 0 }}</n-tag>
      <n-tag type="error">淘汰 {{ strategyLibrary?.counts.rejected ?? 0 }}</n-tag>
      <n-tag type="success">在用 {{ strategyLibrary?.counts.active ?? 0 }}</n-tag>
      <n-button size="tiny" tertiary :loading="strategyLibraryLoading" @click="loadStrategyLibrary">刷新</n-button>
    </div>
    <div v-if="strategyLibraryError" class="error-line">{{ strategyLibraryError }}</div>
    <p class="notice-line">策略修改会生成新版本，旧版本、验证和状态历史不删除。未通过历史硬门禁、前向和考核期时不能确认采用。</p>
    <div class="strategy-library-list">
      <details v-for="card in strategyLibrary?.cards ?? []" :key="card.version_id" class="strategy-card-row">
        <summary>
          <b>{{ card.name }}</b>
          <span>v{{ card.current_version }} · {{ tradeRuleLabel(card.rule) }}</span>
          <n-tag size="small" :type="card.status === 'active' ? 'success' : card.status === 'rejected' || card.status === 'retired' ? 'error' : 'warning'">
            {{ strategyStatusLabel[card.status] ?? card.status }}
          </n-tag>
        </summary>
        <div v-for="stage in card.stages" :key="stage.stage" class="strategy-stage" :class="stage.status">
          {{ stage.status === 'passed' ? '✓' : stage.status === 'failed' ? '✕' : '•' }}
          {{ stageLabel[stage.stage] ?? stage.stage }}：{{ stage.detail }}
        </div>
      </details>
      <div v-if="!strategyLibraryLoading && !strategyLibrary?.cards.length" class="empty-line">暂无策略版本</div>
    </div>
  </n-modal>

  <AnalysisDialog
    v-model:show="showAnalysis"
    :symbol="analysisTarget.symbol"
    :name="analysisTarget.name"
    :rule="analysisTarget.rule"
  />

  <!-- 策略命名 / 覆盖：新建、另存为、重命名、覆盖条件共用这一个弹窗 -->
  <n-modal
    v-model:show="strategyDialog"
    preset="card"
    :title="strategyTitle"
    :style="{ width: 'min(520px, calc(100vw - 24px))' }"
    :bordered="false"
    size="small"
  >
    <div class="strategy-form">
      <label class="form-row">
        <span class="form-label">名称</span>
        <n-input
          v-model:value="strategyLabel"
          size="small"
          :maxlength="STRATEGY_LABEL_MAX"
          show-count
          clearable
          placeholder="例如：我的低估值池"
          @keyup.enter="submitStrategy"
        />
      </label>

      <label class="form-row">
        <span class="form-label">说明</span>
        <n-input
          v-model:value="strategyDescription"
          size="small"
          :maxlength="STRATEGY_DESC_MAX"
          show-count
          clearable
          placeholder="选填。写一句这条策略想选什么，以后一眼就认得出来"
        />
      </label>

      <div class="form-row form-block">
        <span class="form-label">交易规则</span>
        <n-select
          v-model:value="strategyRule"
          size="small"
          :options="TRADE_RULE_OPTIONS.map(o => ({ label: o.label, value: o.value }))"
        />
        <span class="form-hint">{{ strategyRuleHint }}</span>
      </div>

      <div class="form-row form-block">
        <span class="form-label">{{ strategyChangesFilter ? '将保存的条件' : '条件（重命名不改动）' }}</span>
        <div class="summary-box">
          <span v-for="(part, index) in strategySummary" :key="index" class="summary-item">{{ part }}</span>
        </div>
        <span v-if="strategyChangesFilter && strategyAction === 'create'" class="form-hint">
          取自当前界面上的条件；若最小值填得比最大值大，保存时会自动交换过来。
        </span>
        <span v-else-if="strategyAction === 'overwrite'" class="form-hint">
          会用当前界面上的条件替换这条策略原来的条件，名称、说明与交易规则保持不变。
        </span>
        <span v-else class="form-hint">
          只改名称、说明与交易规则，筛选条件保持原样。
        </span>
      </div>

      <div v-if="strategyError" class="error-line">{{ strategyError }}</div>
    </div>

    <template #footer>
      <div class="modal-footer">
        <n-button size="small" quaternary :disabled="strategyBusy" @click="strategyDialog = false">取消</n-button>
        <n-button size="small" type="primary" :loading="strategyBusy" @click="submitStrategy">保存</n-button>
      </div>
    </template>
  </n-modal>

  <!-- 删除确认：自建策略删掉就没了，必须问一句 -->
  <n-modal
    v-model:show="deleteDialog"
    preset="dialog"
    type="warning"
    title="删除策略"
    :content="deleteTarget ? `确定删除策略「${deleteTarget.label}」吗？删除后无法恢复。内置策略不受影响。` : ''"
    positive-text="删除"
    negative-text="取消"
    :loading="deleteBusy"
    @positive-click="confirmDeleteStrategy"
    @negative-click="cancelDeleteStrategy"
  />
</template>

<style scoped>
/* ⚠️ 弹窗内**不再有整体滚动区**（v1.5.1）：
   以前这里是 overflow: auto，窗口小时整块内容（含筛选面板）会一起上下滚，
   结果把结果表自己的横向滚动条推出了可视区 —— 用户要左右挪一下，得先把整块
   拉到最底、挪完再拉回来看指标。现在改成：面板内部各自滚（条件面板自己滚、
   表体自己滚），整块永不滚动，横向滚动条永远在弹窗内看得见。 */
.screener-scroll {
  flex: 1 1 auto;
  min-height: 0;
  min-width: 0;
  /* ⚠️ 这里**不是**主滚动区：正常情况下子项会各自收缩到恰好填满，永远不溢出，
     也就不出现滚动条。`auto` 只是**兜底** —— 万一连最小高度都放不下
     （比如同时冒出好几条提示行 + 弹窗被拖到最小），宁可给一条纵向滚动条，
     也不能让内容被 overflow:hidden 直接裁掉、连分页都点不到。 */
  overflow-x: hidden;
  overflow-y: auto;
  /* 自己也是 flex 列容器 —— 让 .screener 用 flex:1 撑满，
     而不是靠 height:100%（百分比高度依赖祖先有确定高度，容易在某些布局下变成 auto）。 */
  display: flex;
  flex-direction: column;
}

.screener {
  display: flex;
  flex-direction: column;
  /* 撑满滚动区高度：子项按自身规则收缩，总高恰好等于面板高度，不会溢出 */
  flex: 1 1 auto;
  min-height: 0;
  min-width: 0;
  gap: var(--space-2);
}

/* ── 右下角缩放柄 ── */
.size-grip {
  position: absolute;
  right: 2px;
  bottom: 2px;
  width: 18px;
  height: 18px;
  z-index: 2;
  cursor: nwse-resize;
  /* 触摸设备上别把拖动手势让给页面滚动 */
  touch-action: none;
}
.size-grip::before {
  content: '';
  position: absolute;
  right: 4px;
  bottom: 4px;
  width: 8px;
  height: 8px;
  border-right: 2px solid var(--color-border-0);
  border-bottom: 2px solid var(--color-border-0);
  border-radius: 0 0 2px 0;
  opacity: 0.7;
  transition: border-color var(--transition-fast), opacity var(--transition-fast);
}
.size-grip:hover::before,
.size-grip.active::before {
  border-color: var(--color-accent);
  opacity: 1;
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
  display: flex;
  gap: var(--space-1);
  /* 策略 chip 单行排，超出就横向滚 —— 不换行是为了**锁住这一条的高度**：
     窄窗口下 7 个内置策略 + 新建 + 自定义会折成三四行，把结果表挤没。 */
  flex-wrap: nowrap;
  min-width: 0;
  overflow-x: auto;
  padding-bottom: 2px;
}
.chip {
  /* 单行横向滚动的前提下，chip 不许被压扁（见 .chips 的注释） */
  flex-shrink: 0;
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
.preset-meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
/* 实时条件摘要：光看策略名不知道筛什么，这一行把条件摊开 */
.preset-cond {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.preset-rule {
  color: var(--color-accent);
  font-style: normal;
}

/* ── 策略 chip：主体点击切换，右侧「⋯」开管理菜单 ── */
.preset-chip {
  flex-shrink: 0;
  display: inline-flex;
  align-items: stretch;
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-full);
  background: transparent;
  overflow: hidden;
  transition: background var(--transition-fast), color var(--transition-fast),
    border-color var(--transition-fast);
}
.preset-chip:hover {
  border-color: var(--color-accent-dim);
}
.preset-chip.active {
  background: var(--color-accent-dim);
  border-color: var(--color-accent);
}
.preset-chip-label,
.preset-chip-more {
  border: 0;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
  font-family: var(--font-sans);
  cursor: pointer;
  padding: 3px 2px 3px 10px;
  transition: color var(--transition-fast);
}
.preset-chip-more {
  /* 常驻但弱化：悬停才点亮，保证「有菜单」这件事是可发现的，
     同时键盘 / 触摸也能直接点到（不用先 hover） */
  padding: 3px 8px 3px 4px;
  opacity: 0.35;
  line-height: 1;
}
.preset-chip:hover .preset-chip-more,
.preset-chip-more:focus-visible {
  opacity: 1;
}
.preset-chip-label:hover,
.preset-chip-more:hover {
  color: var(--color-accent);
}
.preset-chip.active .preset-chip-label,
.preset-chip.active .preset-chip-more {
  color: var(--color-accent);
}
.preset-chip-label sup { margin-left: 3px; color: var(--color-text-tertiary); font-size: 9px; }
.strategy-library-summary { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin-bottom: 10px; }
.strategy-library-list { display: flex; flex-direction: column; gap: 8px; max-height: 55vh; overflow: auto; }
.strategy-card-row { border: 1px solid var(--color-border-0); border-radius: var(--radius-sm); padding: 8px 10px; }
.strategy-card-row summary { cursor: pointer; display: flex; gap: 10px; align-items: center; }
.strategy-card-row summary span { color: var(--color-text-secondary); }
.strategy-stage { margin: 7px 0 0 14px; color: var(--color-text-secondary); font-size: var(--text-xs); line-height: 1.5; }
.strategy-stage.passed { color: var(--color-down); }
.strategy-stage.failed, .strategy-stage.blocked { color: var(--color-error); }
/* 自建策略加一个小圆点，和内置策略区分开 */
.preset-chip.mine .preset-chip-label::after {
  content: '';
  display: inline-block;
  width: 4px;
  height: 4px;
  margin-left: 5px;
  border-radius: 50%;
  background: var(--color-accent);
  vertical-align: middle;
  opacity: 0.7;
}
.chip-add {
  border-style: dashed;
}

/* ── 策略弹窗 ── */
.strategy-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.form-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.form-block {
  flex-direction: column;
  align-items: stretch;
}
.form-label {
  flex-shrink: 0;
  width: 42px;
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}
.summary-box {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
  padding: var(--space-2);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md);
  background: var(--color-bg-card);
}
.summary-item {
  padding: 2px 8px;
  border-radius: var(--radius-full);
  background: var(--color-accent-dim);
  color: var(--color-accent);
  font-size: var(--text-xs);
}
.form-hint {
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}
.notice-ok {
  border-left: 3px solid var(--color-accent);
}
.link-btn {
  margin-left: var(--space-2);
  border: 0;
  background: transparent;
  color: var(--color-accent);
  font-size: var(--text-xs);
  cursor: pointer;
  padding: 0;
}

/* ── 筛选面板 ── */
/* 允许被压缩（flex-shrink: 1）并自带纵向滚动：窗口小时它让高度给结果表，
   自己内部滚 —— 这样整块内容不会溢出，表格的横向滚动条也就在视野内。
   刻意不设 max-height：高度够时按内容自然高（本来就不到 200px），
   高度不够时由 flex 收缩兜住，加个百分比上限只是多余且依赖父级高度。 */
.filter-panel {
  flex: 0 1 auto;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md);
  background: var(--color-bg-card);
}
:global([data-style="trading"]) .filter-panel,
:global([data-style="modern"]) .filter-panel { background: var(--color-surface-1); border-color: var(--color-border-1); }
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
.sector-filter-row { flex-wrap: nowrap; }
.sector-filter-select { flex: 1 1 260px; min-width: 180px; }
.concept-label { margin-left: var(--space-2); }
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
/* 同样单行横向滚（理由见 .chips）：按钮组一换行就是两三条，窗口小时
   光工具条就能吃掉结果表一半的高度。 */
.toolbar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: nowrap;
  min-width: 0;
  overflow-x: auto;
  padding-bottom: 2px;
}
.toolbar > * {
  flex-shrink: 0;
}
.stats {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  white-space: nowrap;
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
/* 通道能力提示：不是错误，只是「换个通道就有了」，所以用中性色 */
.channel-hint {
  flex-shrink: 0;
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-warning-bg);
  border: 1px solid var(--color-warning-border);
  color: var(--color-warning);
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
.dynamic-advice { display:grid;grid-template-columns:1fr auto;gap:6px 12px;padding:10px 12px;border:1px solid var(--color-border);border-radius:var(--radius-sm);background:var(--color-surface-1); }
.dynamic-advice div,.dynamic-advice p,.dynamic-advice small{grid-column:1}.dynamic-advice div{display:flex;justify-content:space-between;gap:12px}.dynamic-advice p{margin:0}.dynamic-advice small{color:var(--color-text-secondary)}.dynamic-advice .n-button{grid-column:2;grid-row:1/4;align-self:center}

/* ── 结果表 ── */
.table-wrap {
  flex: 1 1 auto;
  /* 下限 = 表体最小高度(TABLE_MIN_H) + 分页条(TABLE_PAGINATION_H)，
     保证表体 + 分页刚好装得下、不会被裁掉分页或横向滚动条。 */
  min-height: 112px;
  min-width: 0;
  overflow: hidden;
}
.table-wrap :deep(.mono) {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.table-wrap :deep(.muted) {
  color: var(--color-text-tertiary);
}
/* 概念可能有很多个，只显示前两个 + 「+N」，其余靠原生 tooltip 展开：
   单行高度恒定，后面的列也不会被内容推出去。 */
.table-wrap :deep(.concept-cell) {
  display: inline-block;
  width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--color-text-secondary);
  cursor: help;
}
/* 「可入场」用品牌蓝而不是红/绿 —— 红绿在本项目里表示涨跌，借用会误导 */
.table-wrap :deep(.entry-ok) {
  color: var(--color-accent);
  font-weight: var(--font-weight-semibold);
}
.table-wrap :deep(.entry-no) {
  color: var(--color-text-tertiary);
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
  .sector-filter-row { flex-wrap: wrap; }
  .sector-filter-row .field-label { width: 40px; }
  .sector-filter-select { flex-basis: calc(100% - 56px); min-width: 0; }
  .concept-label { margin-left: 0; }
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

<style>
/*
 * 可缩放弹窗的骨架 —— 不能写成 scoped：
 * `class` 会经由 NModal 的 $attrs 落到 NCard 根元素（即 .n-modal 本身），
 * 那个元素不在本组件的模板里，拿不到 scoped 的 data 属性。
 *
 * 为什么必须让卡片成为纵向 flex：内容区要 flex:1 撑满卡片高度，
 * 内部的「表格区 flex:1」才有剩余空间可分 —— 否则拖大弹窗只有外框变大。
 */
.universe-screener-modal.n-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.universe-screener-modal.n-card > .n-card-content {
  flex: 1 1 auto;
  min-height: 0;
  overflow: hidden;
}
/* 拖动缩放时别顺手选中面板里的文字 */
.universe-screener-modal.is-resizing,
.universe-screener-modal.is-resizing * {
  user-select: none;
}
</style>
