export interface FollowSourceRun {
  id: number;
  model_id: string;
  model_name: string;
  holding_days: number;
  comparison: string;
  mode: string;
}

export interface FollowPosition {
  symbol: string;
  name: string;
  quantity: number;
  available_quantity: number;
  cost_cny: number;
  mark_cny: number | null;
  mark_at?: string | null;
  entry_date: string;
  holding_sessions: number;
  exit_reason: string | null;
}

export interface FollowOrder {
  id: number;
  symbol: string;
  name: string;
  side: 'buy' | 'sell';
  quantity: number;
  limit_price_cny: number | null;
  estimated_fee_cny: number | null;
  status: string;
  reason: string;
  signal_date: string;
  created_at: string;
  confirmed_at: string | null;
  automatic_submission?: boolean;
  filled_at: string | null;
  filled_price_cny: number | null;
  fee_cny: number | null;
  quote_at: string | null;
  valid_until: string | null;
  reject_reason: string | null;
}

export interface FollowCandidate {
  symbol: string;
  score: number | null;
  rank: number;
  reference_quantity: number;
  entry_condition_met?: boolean;
  reason: string;
}

export interface FollowExecution {
  signal_basis?: 'completed_daily';
  signal_label?: string;
  entry_schedule?: 'next_session_open';
  execution_basis?: 'live_depth';
  schedule_note?: string;
  position_pct: number;
  total_entry_pct?: number;
  allocation_note?: string;
  holding_days: number;
  entry_policy?: string;
  entry_note?: string;
  exit_policy?: string;
  exit_note?: string;
  max_gap_pct: number;
  buy_window: string;
  sell_window?: string;
  intraday_note?: string;
  fee_note: string;
}

export interface FollowPerformance {
  filled_buys: number;
  filled_sells: number;
  cancelled: number;
  expired: number;
  confirmation_delay_seconds: number | null;
}

export interface FollowAllocationResearch {
  start: string;
  end: string;
  early_period: string;
  later_period: string;
  decision: string;
  selection_note: string;
  ledger_count: number;
  annual_slice_count: number;
  selected: string;
  selected_policy_id?: string;
  recommendation?: { default_max_positions: number; position_pct: number; total_entry_pct: number; characteristics: string; reason: string; default_passed_pressure: boolean };
  slot_comparison?: { max_positions: number; net_return_pct: number; max_drawdown_pct: number }[];
  admitted: boolean;
  reasons: string[];
  exit_decision: string;
  policies: { id: string; name: string; total_entry_pct: number; net_return_pct: number; max_drawdown_pct: number; average_invested_pct: number }[];
}

export interface FollowTimingResearch {
  start: string;
  end: string;
  early_period: string;
  later_period: string;
  studied_max_positions: number;
  /** 数量、仓位和实际时点配置是否与研究配置一致。 */
  current_configuration_matches: boolean;
  selected_policy: string;
  active_policy: string;
  admitted: boolean;
  decision: string;
  reasons: string[];
  characteristics: string;
  ledger_count: number;
  policies: {
    id: string;
    name: string;
    entry_note: string;
    exit_note: string;
    max_holding_sessions: number;
    net_return_pct: number;
    max_drawdown_pct: number;
    mean_holding_sessions: number | null;
    early_exit_cycles: number;
    completed_cycles: number;
  }[];
  stress: {
    name: string;
    baseline_later_net_return_pct: number;
    selected_later_net_return_pct: number;
  }[];
  selection_note: string;
}

export interface FollowView {
  account_id: number;
  setup_reused?: boolean;
  source_run_id: number;
  model_id: string;
  model_name: string;
  as_of: string;
  max_positions: number;
  enabled: boolean;
  effective_enabled?: boolean;
  automatic_execution?: boolean;
  allocation_policy?: string;
  initial_cash_cny: number;
  cash_cny: number;
  reserved_cash_cny?: number;
  available_cash_cny?: number;
  equity_cny: number | null;
  net_return_pct: number | null;
  valuation_note?: string;
  state: string;
  message: string;
  source_sha256: string;
  positions: FollowPosition[];
  orders: FollowOrder[];
  candidates: FollowCandidate[];
  execution: FollowExecution;
  comparison_note: string;
  allocation_research?: FollowAllocationResearch | null;
  timing_research?: FollowTimingResearch | null;
  performance: FollowPerformance;
}

export interface FollowStartInput {
  source_run_id: number;
  initial_cash_cny: number;
  max_positions: number;
}

export interface FollowSetupInput {
  model_id: string;
  initial_cash_cny: number;
  max_positions: number;
}

export type FollowOrderAction = 'cancel';

export interface FollowAutomaticModel {
  model_id: string;
  state: string;
  account_id?: number | null;
  as_of?: string;
  source_run_id?: number;
  source_sha256?: string;
  message?: string;
  error?: string;
}

export interface FollowAutomaticStatus {
  enabled: boolean;
  effective_enabled: boolean;
  state: string;
  message: string;
  next_check_at: string | null;
  as_of: string;
  last_completed_day: string | null;
  last_completed_at: string | null;
  last_error: string | null;
  failures: number;
  models: FollowAutomaticModel[];
}

export interface FollowAutomaticPreset {
  initial_cash_cny: number;
}
