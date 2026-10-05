import type {ConditionTree} from './conditions';
export const MODEL_CATALOG = [
  { id: 'breadth22_h20', name: '技术＋市场广度', note: '20日收益标签代理', threshold: 0 },
  { id: 'index26_h20', name: '技术＋广度＋大盘', note: '加入真实沪深300状态', threshold: 0 },
  { id: 'breadth22_excess_csi20', name: '相对大盘超额', note: '相对指数，正分也可能绝对亏损', threshold: 0 },
  { id: 'breadth22_rank20', name: '全市场强弱排序', note: '截面排序标签，固定阈值0.6', threshold: 0.6 },
  { id: 'breadth22_open_downside20', name: '下行惩罚收益', note: '收益代理扣除开盘途中负偏离', threshold: 0 },
] as const;
export interface ResearchJob {
  id: number; kind: string; state: string; phase: string; message: string;
  completed: number; total: number; cancel_requested: boolean; fingerprint: string | null;
  created_at: string; updated_at: string;
  request: { models: string[]; comparisons: string[]; continuation?: number | null };
  results: Array<{ key: string; model_id: string; comparison: string; holding_days: number; run_id?: number;
    year?: number; accounts?:number; bank_size?:number; evaluations?: Array<{policy:string;selected_id:string|null;status:string;metrics:{net_return_pct:number;max_drawdown_pct:number;completed_holding_cycles:number};double_cost?:{net_return_pct:number};delayed_entry?:{net_return_pct:number};matched_pool_neutral?:{net_return_pct:number};state_breakdown?:Array<{state:string;completed_cycles:number;mean_cycle_net_pct:number|null}>}>;
    metrics?: { net_return_pct: number; max_drawdown_pct: number; completed_holding_cycles: number; win_rate_pct: number | null }; group?: ModelCandidateGroup }>;
}
export interface ModelCandidate {
  symbol: string; as_of: string; score: number; close: number; signal_eligible: boolean;
  reason?: string; reasons?: string[]; technical_state?: string; cash_reference_stage?: string; cash_reference_quantity?: number;
}
export interface ModelCandidateGroup {
  model_id: string; name: string; as_of: string; holding_days: number; score_semantic: string;
  scored_stocks: number; hit_count: number; threshold: number; candidates: ModelCandidate[];
  model_sha256: string; runner_sha256: string; source_fingerprint: string; job_id: number; production_admission: false;
}
export interface CandidateView {
  schema: string; current_as_of: string; as_of: string | null; fresh: boolean; groups: ModelCandidateGroup[];
  data_status?: {mode:string;message:string}; job_id: number | null; historical_hit_count?: number; source_fingerprint?: string; message: string; production_admission: false;
}
export interface ConditionWatch {
  id: number; enabled: boolean;
  config: { symbol: string; model_id: string; model_name: string; preset: string; condition_tree?:ConditionTree|null; initial_hit: ModelCandidate };
  observation: { state?: string; message?: string; checked_at?: string; last_scan_day?: string; last_known_model_hit?: boolean };
}
export interface ConditionWatchView { presets: Array<{ id: string; name: string; description: string }>; watches: ConditionWatch[]; limit_per_tick: number }
export function modelScore(value: unknown, modelId: string): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '--';
  return modelId === 'breadth22_rank20' ? value.toFixed(4) : (value * 100).toFixed(4) + '%';
}
export const jobActive = (job: ResearchJob) => job.state === 'running' || job.state === 'queued';
export const jobState = (state: string) => ({ queued: '等待', running: '运行中', complete: '已完成', failed: '失败', cancelled: '已取消', interrupted: '中断待续' }[state] ?? state);
