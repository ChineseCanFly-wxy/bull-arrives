// src/types/monitor.ts
// 与 Rust 侧 `db::monitors::Monitor` 一一对应（snake_case）。

/** 一条个股监控规则（量化自动止损/止盈） */
export interface Monitor {
  id: number;
  code: string;
  market: string;
  name: string;
  enabled: boolean;
  /** 参考价（开启监控时的最新收盘价） */
  reference_price: number;
  /** 量化自动计算的止损价 */
  stop_price: number;
  /** 量化自动计算的止盈价 */
  take_price: number;
  /** 已触发的条件（stop_loss / take_profit） */
  last_triggered: string | null;
  /** 止损/止盈位计算时间 */
  updated_at: string;
}
