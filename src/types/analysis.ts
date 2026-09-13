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
