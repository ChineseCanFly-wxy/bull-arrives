// src/types/universe.ts
// 与 Rust 侧 `datasource::eastmoney_universe` / `commands::universe` 一一对应。
// 注意：字段名保持 snake_case —— 与 serde 的序列化结果一致，不要改成 camelCase。

/** A 股板块（对应 Rust `Board` 枚举的 snake_case 序列化） */
export type Board =
  | 'sh_main'
  | 'sz_main'
  | 'chi_next'
  | 'star'
  | 'bse'
  | 'b_share'
  | 'other';

/** 全市场快照的一行 */
export interface SnapshotRow {
  code: string;
  name: string;
  /** 最新价（元） */
  price: number;
  /** 涨跌幅 % */
  change_pct: number;
  change_amount: number;
  /** 成交量（手） */
  volume: number;
  /** 成交额（元） */
  amount: number;
  /** 振幅 % */
  amplitude_pct: number;
  /** 换手率 % */
  turnover_rate: number;
  /** 市盈率（动态） */
  pe: number;
  /** 量比 */
  volume_ratio: number;
  change_5min: number;
  high: number;
  low: number;
  open: number;
  prev_close: number;
  /** 总市值（元） */
  total_market_cap: number;
  /** 流通市值（元） */
  circulating_market_cap: number;
  speed: number;
  pb: number;
  change_60d: number;
  change_ytd: number;

  board: Board;
  is_st: boolean;
  is_delisting: boolean;
  suspected_suspended: boolean;
  is_limit_locked: boolean;
}

/** 用户可配置的筛选条件（对应 Rust `MarketFilter`）。数值为 null 表示不限制。 */
export interface MarketFilter {
  /** 允许的板块；空数组表示全部允许 */
  boards: Board[];
  exclude_st: boolean;
  exclude_delisting: boolean;
  exclude_suspended: boolean;
  exclude_limit_locked: boolean;

  price_min: number | null;
  price_max: number | null;
  /** 总市值（亿元） */
  market_cap_min_yi: number | null;
  market_cap_max_yi: number | null;
  /** 换手率 % */
  turnover_min: number | null;
  turnover_max: number | null;
  volume_ratio_min: number | null;
  /** 当日涨跌幅 % */
  change_pct_min: number | null;
  change_pct_max: number | null;
  /** 成交额下限（万元） */
  amount_min_wan: number | null;
}

/** 板块分布项 */
export interface BoardCount {
  board: Board;
  label: string;
  count: number;
}

/**
 * 前端占位用的默认筛选条件。
 *
 * 注意：**真正的默认值定义在 Rust 侧**（`MarketFilter::default`），
 * 这里只是为了在预设尚未加载完成时让 UI 有一个可绑定的非空对象。
 * 打开筛选器时会立即被 `get_filter_presets` 下发的预设覆盖。
 */
export function createDefaultFilter(): MarketFilter {
  return {
    boards: [...SELECTABLE_BOARDS],
    exclude_st: true,
    exclude_delisting: true,
    exclude_suspended: true,
    exclude_limit_locked: true,
    price_min: null,
    price_max: null,
    market_cap_min_yi: null,
    market_cap_max_yi: null,
    turnover_min: null,
    turnover_max: null,
    volume_ratio_min: null,
    change_pct_min: null,
    change_pct_max: null,
    amount_min_wan: null,
  };
}

/** 预设方案（由 Rust 侧下发，规则只有一份） */
export interface PresetInfo {
  id: string;
  label: string;
  description: string;
  filter: MarketFilter;
}

/** `get_market_universe` 的返回 */
/** 全市场快照的取数通道 */
export type SnapshotSource = 'sina' | 'eastmoney';

export interface UniverseResponse {
  total_all: number;
  total_matched: number;
  returned: number;
  stale: boolean;
  /** 本次数据来自哪个通道 */
  source: SnapshotSource;
  /** 通道中文名，后端直接下发 */
  source_label: string;
  /** 当前通道是否提供「量比」；false 时前端应禁用并说明 */
  volume_ratio_supported: boolean;
  /** 因数据源不支持而被自动忽略的条件名（如 ["量比"]） */
  skipped_conditions: string[];
  board_counts: BoardCount[];
  rows: SnapshotRow[];
}

/** 数据源选项（筛选器工具条下拉） */
export const SOURCE_OPTIONS: Array<{ label: string; value: 'auto' | SnapshotSource }> = [
  { label: '自动（推荐）', value: 'auto' },
  { label: '新浪财经', value: 'sina' },
  { label: '东方财富', value: 'eastmoney' },
];

/** 板块中文名（兜底映射；后端也会回传 label） */
export const BOARD_LABELS: Record<Board, string> = {
  sh_main: '沪市主板',
  sz_main: '深市主板',
  chi_next: '创业板',
  star: '科创板',
  bse: '北交所',
  b_share: 'B股',
  other: '未识别',
};

/** 可在 UI 中勾选的板块（B股与未识别不提供勾选） */
export const SELECTABLE_BOARDS: Board[] = [
  'sh_main',
  'sz_main',
  'chi_next',
  'star',
  'bse',
];

/** 元 → 亿元 */
export function toYi(yuan: number): number {
  return yuan / 1e8;
}

/**
 * 6 位纯代码 → 项目内部使用的「交易所前缀 + 代码」完整符号。
 *
 * ⚠️ 项目里 `WatchItem.code` / `Quote.code` 用的是 `sh600519`、`sz000001`、`bj920992`
 * 这种带前缀的完整符号（见 `datasource/search.rs`），
 * 而东财快照返回的是 6 位纯代码（`600519`）。不转换会导致自选股存进去后取不到行情。
 */
export function toFullSymbol(code: string, board: Board): string {
  switch (board) {
    case 'sh_main':
    case 'star':
      return `sh${code}`;
    case 'sz_main':
    case 'chi_next':
      return `sz${code}`;
    case 'bse':
      return `bj${code}`;
    default:
      // B股与未识别不值得加入自选，兜底按深市处理以免产生空值
      return `sz${code}`;
  }
}

/** 元 → 万元 */
export function toWan(yuan: number): number {
  return yuan / 1e4;
}

/** 成交额友好显示：< 1 亿显示万元，否则显示亿 */
export function formatAmount(yuan: number): string {
  if (!Number.isFinite(yuan) || yuan <= 0) return '-';
  const yi = yuan / 1e8;
  if (yi >= 1) return `${yi.toFixed(2)}亿`;
  return `${(yuan / 1e4).toFixed(0)}万`;
}

/** 带符号的百分比 */
export function formatPct(value: number, digits = 2): string {
  if (!Number.isFinite(value)) return '-';
  const sign = value > 0 ? '+' : '';
  return `${sign}${value.toFixed(digits)}%`;
}
