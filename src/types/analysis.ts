// src/types/analysis.ts
// 与 Rust 侧 `quant::scorer::StockAnalysis` 一一对应（snake_case）。

/** 单个因子的评分结果 */
export interface FactorScore {
  /** 因子名（中文） */
  name: string;
  /** 0–100 分 */
  score: number;
  /** 权重（合计 1.0） */
  weight: number;
  /** 一句话说明 */
  note: string;
}

/** 个股技术分析结果 */
export interface StockAnalysis {
  /** 综合评分 0–100 */
  total_score: number;
  /** 结论：强烈关注 / 关注 / 中性 / 回避 */
  verdict: string;
  /** 各因子明细 */
  factors: FactorScore[];
  /**
   * 关键指标快照（null 表示数据不足未算出）。
   *
   * ⚠️ 只放**彼此不重复**的指标。下面几项是被刻意删掉的，别再加回来：
   * - `ma5` / `ma10`：与 MA20 高度共线，MA5 一周平均噪声极大
   * - `macd_dea`：就是 DIF 的 9 日 EMA，两者永远贴着走
   * - `boll_mid`：**数学上恒等于 MA20**，同一个数字显示两遍
   * - `kdj_k/d/j`：J = 3K−2D 纯派生，且 KDJ 整体判据只有 4 档
   */
  close: number | null;
  ma20: number | null;
  ma60: number | null;
  macd_dif: number | null;
  rsi12: number | null;
  boll_upper: number | null;
  boll_lower: number | null;
  /** 20 日 / 60 日动量（0.1 = +10%） */
  momentum20: number | null;
  momentum60: number | null;
  /** 量比 */
  volume_ratio: number | null;
  /** 操作计划（买点 / 止损 / 止盈 / 仓位）。K 线不足或价格异常时为 null */
  trade_plan: TradePlan | null;
  /** 最终采用的那条规则在这只股票自身历史上的回测结果 */
  backtest: BacktestStats | null;
  /** 支撑位与压力位，**已按最终采用的规则加权排序**（压力由近到远，然后支撑由近到远） */
  levels: PriceLevel[];
  /** 按市场状态自动匹配到的规则，以及三条规则各自的状态 */
  rule_match: RuleMatch | null;
  /** 本次量化计算使用的历史数据口径 */
  history: HistoryMeta | null;
  /** 后端冻结的 Agent 输入指纹，不回传可篡改的量化对象 */
  agent_context_fingerprint: string | null;
}

export interface AgentStatus {
  installed: boolean;
  state: 'ready' | 'detected' | 'failed' | 'unavailable' | 'not_found' | 'misconfigured';
  path: string | null;
  message: string;
  guidance: string;
}

export interface AgentEvidence {
  field: string;
  value: string;
  source: string;
  as_of: string;
}

export interface AgentInvalidation {
  field: string;
  operator: 'lt' | 'lte' | 'gt' | 'gte' | 'cross_below' | 'cross_above';
  reference_field: string;
}

export interface AgentAnalysisResponse {
  status: 'ready' | 'unavailable' | 'failed';
  provider: 'claude_code';
  cached: boolean;
  conclusion: string | null;
  summary: string | null;
  claims: Array<{ kind: 'support' | 'risk' | 'watch'; text: string; evidence: AgentEvidence[] }>;
  evidence: AgentEvidence[];
  confidence: number | null;
  invalidation_conditions: AgentInvalidation[];
  error: string | null;
  guidance: string | null;
  generated_at: string;
  context_fingerprint: string;
}

export interface AgentRoleResult {
  role: 'technical' | 'bull' | 'bear' | 'risk';
  round: 1 | 2;
  status: 'ready' | 'failed';
  cached: boolean;
  conclusion: 'bullish' | 'neutral' | 'bearish' | 'cautious' | null;
  evidence: AgentEvidence[];
  confidence: number | null;
  argument: string | null;
  error: string | null;
}

export interface PredictionCalibration {
  status: 'observing' | 'ready';
  verified: number;
  span_days: number;
  required_verified: number;
  required_span_days: number;
  buckets: Array<{ confidence_min: number; confidence_max: number; count: number; actual_hit_rate: number }>;
}

export interface AgentTeamResponse {
  status: 'ready' | 'failed' | 'cancelled';
  roles: AgentRoleResult[];
  final_conclusion: AgentRoleResult['conclusion'];
  confidence: number | null;
  prediction_saved: boolean;
  calibration: PredictionCalibration;
}

export interface ResearchReport {
  bars: number;
  horizon_days: number;
  causal_audit_passed: boolean;
  factors: Array<{
    id: string;
    label: string;
    samples: number;
    rank_ic: number | null;
    quantiles: Array<{
      quantile: number;
      samples: number;
      avg_return_pct: number;
      positive_probability: number;
    }>;
  }>;
  patterns: Array<{
    id: string;
    label: string;
    samples: number;
    positive_probability: number | null;
    avg_return_pct: number | null;
    max_drawdown_pct: number | null;
  }>;
  stratification_status: 'unavailable' | 'ready';
  stratification_note: string;
  intraday_status: 'unavailable' | 'ready';
  intraday_note: string;
  runtime: string;
}

export interface HistoryMeta {
  source: 'local_stockdb' | 'online';
  source_label: string;
  start_date: string | null;
  end_date: string | null;
  bars: number;
  adjustment: 'qfq';
  stale: boolean;
  warning: string | null;
}

/** 一条规则在当前这只股票上的状态 */
export interface RuleCandidate {
  rule: TradeRuleId;
  rule_label: string;
  /** 当前是否满足该规则的入场条件 */
  ready: boolean;
  /** 未满足时缺什么 */
  waiting_for: string | null;
  /** 状态强度 0–100：成立时是「这种状态有多典型」，未成立时是「离触发有多近」 */
  strength: number;
  /** 一句话说明这条规则现在怎么看 */
  note: string;
}

/**
 * 按**当前市场状态**自动匹配到的规则。
 *
 * ⚠️ 这不是「历史上哪条规则最赚」。实测那样挑的选对率只有 40%（随机挑是 33%），
 * 本质上是在 3 个噪声里挑最大值 —— 把某段行情的特征当成了规律。
 * 这里挑的依据是三条规则各自的入场条件是否成立，也就是**当前处于什么状态**。
 */
export interface RuleMatch {
  recommended: TradeRuleId;
  recommended_label: string;
  /** 推荐依据（说人话） */
  reason: string;
  /** 是否有多条同时成立 */
  multiple_ready: boolean;
  /** 三条都不成立时为 true，此时推荐的是「最接近触发」的一条 */
  none_ready: boolean;
  /** 顺序固定：趋势跟随 / 均值回归 / 放量突破 */
  candidates: RuleCandidate[];
}

/** 位在现价上方还是下方 */
export type LevelKind = 'support' | 'resistance';

/**
 * 这个价位是从哪来的。
 *
 * ⚠️ 曾经还有一个 `chip_peak`（筹码密集区）来源，v1.5.1 随筹码分布一起去掉了：
 * A 股没有公开的筹码原始数据，各家软件的"筹码峰"都是自己的模型算的、互相之间对不上，
 * 拿它当支撑压力位会误导。现在所有来源都是**价格自己走出来的**。
 */
export type LevelSource =
  | 'ma20'
  | 'ma60'
  | 'ma120'
  | 'boll_lower'
  | 'boll_upper'
  | 'swing_low'
  | 'swing_high'
  | 'prior_low20'
  | 'prior_high20';

/** 一条支撑位 / 压力位 */
export interface PriceLevel {
  price: number;
  kind: LevelKind;
  /** 强度最高的那个来源 */
  source: LevelSource;
  /** 中文来源说明，可能由多个来源共振而成（如 "MA20 + 摆动低点"） */
  label: string;
  /** 为什么这个位置值得看 */
  note: string;
  /** 参考强度 0–100（已按当前策略加权，不是"必守/必破"的概率） */
  strength: number;
  /** 距现价的百分比（正数） */
  distance_pct: number;
}

/** 交易规则族。与 Rust `quant::playbook::TradeRule` 一一对应。 */
export type TradeRuleId = 'trend_follow' | 'mean_reversion' | 'breakout';

/**
 * 规则说明。
 *
 * ⚠️ 措辞里刻意把**胜率与盈亏比绑在一起说**：高胜率低盈亏比的策略是亏钱的，
 * 公开资料里就有「胜率 66.7% 但一年只赚 1.06%」的实例。UI 上不许只显示胜率。
 */
export const TRADE_RULE_OPTIONS: Array<{
  label: string;
  value: TradeRuleId;
  hint: string;
  /** 这套规则最该盯哪一类价位 —— 支撑/压力位的权重就是按这个定的（见 Rust `quant/levels.rs`） */
  focus: string;
}> = [
  {
    label: '趋势跟随',
    value: 'trend_follow',
    hint: '胜率通常只有四成上下，但盈亏比能过 2 —— 靠少数大行情赚钱，多数交易小亏出局。回踩 MA20 买入，2×ATR 止损。',
    focus: '看均线：MA20 是回踩买点，跌破收不回就先离场',
  },
  {
    label: '均值回归',
    value: 'mean_reversion',
    hint: '胜率能到六成以上，但盈亏比普遍不足 1.5 —— 赚多次小钱，怕的是单边下跌里一路接飞刀。布林下轨附近买入，短持仓。',
    focus: '看布林轨道：下轨附近买、上轨附近走，均线只作参考',
  },
  {
    label: '放量突破',
    value: 'breakout',
    hint: '胜率中等偏上，盈亏比约 1.5 —— 关键是量能确认，缺了量的突破假信号率很高。突破前 20 日高点买入。',
    focus: '看前高：前 20 日高点是触发位，摆动高点被反复冲高回落说明上方抛压重',
  },
];

/** 规则 id → 该规则最该盯哪类价位 */
export function tradeRuleFocus(rule: string): string {
  return TRADE_RULE_OPTIONS.find(o => o.value === rule)?.focus ?? '';
}

/** 规则 id → 中文名（兜底；后端也会回传 label） */
export function tradeRuleLabel(rule: string): string {
  return TRADE_RULE_OPTIONS.find(o => o.value === rule)?.label ?? '趋势跟随';
}

/**
 * 筛选器结果表用的「轻量状态」—— `batch_stock_status` 命令的返回项。
 *
 * 与 `StockAnalysis` 的区别：这里只带**表格列需要的那几个字段**。
 * 一页最多 100 只，把完整分析（因子明细 / 回测）全传回来纯属浪费。
 */
export interface StockStatusItem {
  /** 完整符号（sh600519），前端用它回填到对应行 */
  symbol: string;
  /** 量化评分；拉 K 线失败或数据不足时为 null */
  score: number | null;
  /** 当前是否已满足入场条件；无计划（K 线不足 / 价格异常）时为 null */
  ready: boolean | null;
  /** 未满足时缺什么条件 */
  waiting_for: string | null;
  buy_low: number | null;
  buy_high: number | null;
  stop_loss: number | null;
  take_profit: number | null;
  risk_reward: number | null;
  /** 失败原因；成功时为 null */
  error: string | null;
  history: HistoryMeta | null;
}

/** 一只股票在某条规则下的操作计划 */
export interface TradePlan {
  rule: TradeRuleId;
  rule_label: string;
  /** 这类规则的「性格」：胜率与盈亏比的方向性说明 */
  rule_profile: string;
  reference_price: number;
  /** 建议建仓区间 */
  buy_low: number;
  buy_high: number;
  stop_loss: number;
  take_profit: number;
  /** 盈亏比：(止盈 − 现价) / (现价 − 止损) */
  risk_reward: number;
  /** 止损幅度 % */
  stop_pct: number;
  /** 建议仓位上限 %（按单笔风险反推） */
  position_pct: number;
  /** 当前是否已满足入场条件 */
  ready: boolean;
  /** 未满足时缺什么 */
  waiting_for: string | null;
  /** 价位是怎么来的 */
  notes: string[];
}

/** 规则回测统计 */
export interface BacktestStats {
  rule: TradeRuleId;
  rule_label: string;
  /** 回测用了多少根日 K */
  bars: number;
  trades: number;
  wins: number;
  /** 胜率 0–1 */
  win_rate: number;
  avg_win_pct: number;
  avg_loss_pct: number;
  /** 盈亏比 */
  payoff_ratio: number;
  /** 每笔期望收益 %（扣成本后）—— 决定赚不赚钱的是这个 */
  expectancy_pct: number;
  total_return_pct: number;
  max_drawdown_pct: number;
  avg_hold_days: number;
  /** 已扣除的双边交易成本 % */
  cost_pct: number;
  causal_audit: {
    passed: boolean;
    static_checks: number;
    dynamic_checks: number;
    message: string;
  };
  trust: {
    eligible: boolean;
    status: string;
    methodology: string[];
    gates: Array<{ key: string; label: string; passed: boolean; detail: string }>;
    raw_execution: boolean;
    oos_trades: number;
    oos_expectancy_pct: number;
    profit_probability: number;
    doubled_cost_expectancy_pct: number;
    cost_flip: boolean;
    parameter_min_expectancy_pct: number;
    benchmark_return_pct: number | null;
    excess_return_pct: number | null;
    period_returns_pct: number[];
    residual_position: boolean;
  };
  note: string;
}

/**
 * 把回测结果翻译成人话结论。
 *
 * **这是本功能里最重要的一个函数**：它强制胜率与盈亏比、期望值一起判断，
 * 避免用户看到「胜率 70%」就以为能赚钱。
 */
export function evaluateBacktest(stats: BacktestStats): {
  tone: 'good' | 'warn' | 'bad';
  text: string;
} {
  if (!stats.trust.eligible) {
    const failed = stats.trust.gates.filter(gate => !gate.passed).map(gate => gate.label);
    return {
      tone: 'bad',
      text: `${stats.trust.status}：未通过 ${failed.join('、')}`,
    };
  }
  if (!stats.causal_audit.passed) {
    return { tone: 'bad', text: `因果审计未通过：${stats.causal_audit.message}` };
  }
  if (stats.trades === 0) {
    return { tone: 'warn', text: `过去 ${stats.bars} 根日 K 里这套规则一次都没触发 —— 它挑的不是当前这种走势` };
  }
  if (stats.trades < 10) {
    return {
      tone: 'warn',
      text: `只触发 ${stats.trades} 次，样本太少，胜率基本是噪声，别当依据`,
    };
  }
  const exp = stats.expectancy_pct;
  if (exp <= 0) {
    return {
      tone: 'bad',
      text: `每笔期望 ${exp.toFixed(2)}%，扣掉交易成本是亏的 —— 胜率 ${
        (stats.win_rate * 100).toFixed(0)
      }% 也救不了，盈亏比只有 ${stats.payoff_ratio.toFixed(2)}`,
    };
  }
  if (exp >= 1.0) {
    return {
      tone: 'good',
      text: `每笔期望 +${exp.toFixed(2)}%，胜率 ${(stats.win_rate * 100).toFixed(0)}% 配合盈亏比 ${stats.payoff_ratio.toFixed(2)}，是正期望`,
    };
  }
  return {
    tone: 'warn',
    text: `每笔期望 +${exp.toFixed(2)}%，勉强为正 —— 扣掉成本后余量很薄，别重仓`,
  };
}

/** 评分结论对应的颜色类型（红涨绿跌语义下用中性色表达风险等级） */
export function verdictTone(verdict: string): 'success' | 'warning' | 'default' | 'error' {
  switch (verdict) {
    case '强烈关注':
      return 'success';
    case '关注':
      return 'warning';
    case '中性':
      return 'default';
    case '回避':
      return 'error';
    default:
      return 'default';
  }
}

/** 把 0–1 的动量转成百分比字符串 */
export function formatMomentum(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '-';
  const sign = v > 0 ? '+' : '';
  return `${sign}${(v * 100).toFixed(1)}%`;
}
