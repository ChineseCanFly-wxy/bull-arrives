// src/types/rank.ts
// 与 Rust 侧 `commands::rank::RankItem / RankResponse` 一一对应（snake_case）。

import type { Board, SnapshotSource } from './universe';
import type { StockAnalysis } from './analysis';

/** 推荐榜的一行 */
export interface RankItem {
  /** 6 位代码 */
  code: string;
  name: string;
  board: Board;
  /** 当日涨跌幅 % */
  change_pct: number;
  /** 成交额（元） */
  amount: number;
  /** 量化评分（拉日 K 失败或数据不足时为 null） */
  analysis: StockAnalysis | null;
  /** 评分失败原因（成功时为 null） */
  error: string | null;
}

/** `scan_and_rank` 的返回 */
export interface RankResponse {
  /** 实际扫描的股票池大小（轻量扫描时是「最活跃的一批」而非全市场） */
  scanned: number;
  /** 命中筛选的总数 */
  total_matched: number;
  /** 参与评分的数量 */
  candidates: number;
  /** 成功评分数量 */
  scored: number;
  /** 失败数量 */
  failed: number;
  /** 快照是否陈旧 */
  stale: boolean;
  /** 本次数据来自哪个通道 */
  source: SnapshotSource;
  /** 因数据源不支持而被自动忽略的条件名（如 ["量比"]） */
  skipped_conditions: string[];
  /** 按总分降序的榜单 */
  items: RankItem[];
}
