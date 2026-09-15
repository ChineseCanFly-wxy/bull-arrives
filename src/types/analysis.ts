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
  /** 关键指标快照（null 表示数据不足未算出） */
  close: number | null;
  ma5: number | null;
  ma10: number | null;
  ma20: number | null;
  ma60: number | null;
  macd_dif: number | null;
  macd_dea: number | null;
  rsi12: number | null;
  kdj_k: number | null;
  kdj_d: number | null;
  kdj_j: number | null;
  boll_upper: number | null;
  boll_mid: number | null;
  boll_lower: number | null;
  /** 20 日 / 60 日动量（0.1 = +10%） */
  momentum20: number | null;
  momentum60: number | null;
  /** 量比 */
  volume_ratio: number | null;
  /** 操作计划（买点 / 止损 / 止盈 / 仓位）。K 线不足或价格异常时为 null */
  trade_plan: TradePlan | null;
  /** 该规则在这只股票自身历史上的回测结果 */
  backtest: BacktestStats | null;
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
}> = [
  {
    label: '趋势跟随',
    value: 'trend_follow',
    hint: '胜率通常只有四成上下，但盈亏比能过 2 —— 靠少数大行情赚钱，多数交易小亏出局。回踩 MA20 买入，2×ATR 止损。',
  },
  {
    label: '均值回归',
    value: 'mean_reversion',
    hint: '胜率能到六成以上，但盈亏比普遍不足 1.5 —— 赚多次小钱，怕的是单边下跌里一路接飞刀。布林下轨附近买入，短持仓。',
  },
  {
    label: '放量突破',
    value: 'breakout',
    hint: '胜率中等偏上，盈亏比约 1.5 —— 关键是量能确认，缺了量的突破假信号率很高。突破前 20 日高点买入。',
  },
];

/** 规则 id → 中文名（兜底；后端也会回传 label） */
export function tradeRuleLabel(rule: string): string {
  return TRADE_RULE_OPTIONS.find(o => o.value === rule)?.label ?? '趋势跟随';
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
