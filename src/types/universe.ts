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
  listing_date: string | null;
  listed_days: number | null;

  board: Board;
  is_st: boolean;
  is_delisting: boolean;
  suspected_suspended: boolean;
  is_limit_locked: boolean;
  is_cdr: boolean;
}

/** 用户可配置的筛选条件（对应 Rust `MarketFilter`）。数值为 null 表示不限制。 */
export interface MarketFilter {
  /** 允许的板块；空数组表示全部允许 */
  boards: Board[];
  exclude_st: boolean;
  exclude_delisting: boolean;
  exclude_suspended: boolean;
  exclude_limit_locked: boolean;
  exclude_cdr: boolean;

  listed_days_min: number | null;
  listed_days_max: number | null;

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
  /**
   * 60 日涨跌幅 % 区间 —— 趋势 / 反转类策略的主要依据。
   * ⚠️ 仅东方财富通道提供；新浪通道下该条件会被自动忽略并在界面提示。
   */
  change_60d_min: number | null;
  change_60d_max: number | null;
  /** 市盈率（动态）。下限设 0.01 即「只要盈利股」 */
  pe_min: number | null;
  pe_max: number | null;
  /** 市净率 */
  pb_min: number | null;
  pb_max: number | null;
  /** 振幅上限 %（当日振幅，作为波动率代理） */
  amplitude_max: number | null;
  above_ma_days: number | null;
  new_high_days: number | null;
  macd_bullish: boolean;
  kdj_bullish: boolean;
  volume_price_rising: boolean;
  rise_from_low_days: number | null;
  rise_from_low_min: number | null;
  rise_from_low_max: number | null;
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
    exclude_cdr: false,
    listed_days_min: null,
    listed_days_max: null,
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
    change_60d_min: null,
    change_60d_max: null,
    pe_min: null,
    pe_max: null,
    pb_min: null,
    pb_max: null,
    amplitude_max: null,
    above_ma_days: null,
    new_high_days: null,
    macd_bullish: false,
    kdj_bullish: false,
    volume_price_rising: false,
    rise_from_low_days: null,
    rise_from_low_min: null,
    rise_from_low_max: null,
  };
}

/** 预设方案（由 Rust 侧下发，规则只有一份） */
export interface PresetInfo {
  id: string;
  label: string;
  description: string;
  filter: MarketFilter;
  /**
   * 配套的交易规则 id（`trend_follow` / `mean_reversion` / `breakout`）。
   *
   * 策略只负责粗筛（单日快照能表达的字段），精确买点/止损/止盈要靠日 K 算，
   * 这个字段是两者的纽带：打开个股分析时用它决定用哪套规则。
   */
  rule: string;
  /** 内置策略随版本维护，不允许改名 / 删除，只能「另存为」自己的副本 */
  builtin: boolean;
  strategy_version_id: number | null;
  strategy_version: number;
  strategy_status: string;
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
  /** 当前通道是否提供「60 日涨跌幅」；false 时依赖它的策略条件会被跳过 */
  change_60d_supported: boolean;
  listing_date_supported: boolean;
  /** 因数据源不支持而被自动忽略的条件名（如 ["量比"]） */
  skipped_conditions: string[];
  history_notice: string | null;
  history_evaluated: number;
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

/** 数值区间的可读描述：只填一边就说「≥ / ≤」，两边都填说「a–b」 */
function rangeText(
  label: string,
  min: number | null,
  max: number | null,
  unit: string,
  signed = false,
): string | null {
  if (min == null && max == null) return null;
  const fmt = (value: number) => {
    const text = Number.isInteger(value) ? String(value) : String(Number(value.toFixed(4)));
    return signed && value > 0 ? `+${text}` : text;
  };
  if (min != null && max != null) return `${label} ${fmt(min)}–${fmt(max)}${unit}`;
  if (min != null) return `${label} ≥ ${fmt(min)}${unit}`;
  return `${label} ≤ ${fmt(max as number)}${unit}`;
}

/** 万元 → 友好文本 */
function wanText(wan: number): string {
  if (wan >= 10000) {
    const yi = wan / 10000;
    return `${Number.isInteger(yi) ? yi : yi.toFixed(2)} 亿`;
  }
  return `${wan} 万`;
}

/**
 * 把筛选条件拆成可读片段。
 *
 * 用途：策略名只有四个字，用户根本记不住它到底筛什么 ——
 * 「稳健趋势」和「低波动稳健」的差别只有点开才看得见。
 * 保存策略时把这份摘要一并展示，用户能一眼确认自己存下来的到底是什么条件。
 */
export function summarizeFilterParts(filter: MarketFilter): string[] {
  const parts: string[] = [];

  const chosen = SELECTABLE_BOARDS.filter(board => filter.boards.includes(board));
  if (chosen.length === SELECTABLE_BOARDS.length) parts.push('全部板块');
  else if (chosen.length === 0) parts.push('未选板块');
  else parts.push(chosen.map(board => BOARD_LABELS[board]).join('/'));

  const excludes: string[] = [];
  if (filter.exclude_st) excludes.push('ST');
  if (filter.exclude_delisting) excludes.push('退市');
  if (filter.exclude_suspended) excludes.push('停牌');
  if (filter.exclude_limit_locked) excludes.push('一字板');
  if (filter.exclude_cdr) excludes.push('存托凭证');
  parts.push(excludes.length ? `排除 ${excludes.join('/')}` : '不做排除');

  const ranges = [
    rangeText('价格', filter.price_min, filter.price_max, ' 元'),
    rangeText('市值', filter.market_cap_min_yi, filter.market_cap_max_yi, ' 亿'),
    rangeText('上市天数', filter.listed_days_min, filter.listed_days_max, ' 天'),
    rangeText('换手率', filter.turnover_min, filter.turnover_max, '%'),
    rangeText('涨跌幅', filter.change_pct_min, filter.change_pct_max, '%', true),
    rangeText('60日涨跌幅', filter.change_60d_min, filter.change_60d_max, '%', true),
    rangeText('市盈率', filter.pe_min, filter.pe_max, ''),
    rangeText('市净率', filter.pb_min, filter.pb_max, ''),
    filter.volume_ratio_min == null ? null : `量比 ≥ ${filter.volume_ratio_min}`,
    filter.amplitude_max == null ? null : `振幅 ≤ ${filter.amplitude_max}%`,
    filter.amount_min_wan == null ? null : `成交额 ≥ ${wanText(filter.amount_min_wan)}`,
    filter.above_ma_days == null ? null : `站上 MA${filter.above_ma_days}`,
    filter.new_high_days == null ? null : `${filter.new_high_days} 日新高`,
    filter.macd_bullish ? 'MACD 多头' : null,
    filter.kdj_bullish ? 'KDJ 多头' : null,
    filter.volume_price_rising ? '量价齐升' : null,
    filter.rise_from_low_days == null ? null : rangeText(
      `距${filter.rise_from_low_days}日低点`,
      filter.rise_from_low_min,
      filter.rise_from_low_max,
      '%',
      true,
    ),
  ].filter((item): item is string => item !== null);
  parts.push(...(ranges.length ? ranges : ['不限数值区间']));

  return parts;
}

/** 单行摘要，用于 tooltip */
export function summarizeFilter(filter: MarketFilter): string {
  return summarizeFilterParts(filter).join(' · ');
}
