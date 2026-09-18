export type SectorKind = 'industry' | 'concept';

export interface SectorSummary {
  kind: SectorKind;
  code: string;
  name: string;
  rank: number;
  latest: number | null;
  change_amount: number | null;
  change_pct: number | null;
  amount: number | null;
  market_cap: number | null;
  turnover_rate: number | null;
  up_count: number | null;
  down_count: number | null;
  leader_name: string | null;
  leader_change_pct: number | null;
  change_pct_3d: number | null;
  change_pct_5d: number | null;
  change_pct_10d: number | null;
  main_net_inflow: number | null;
  main_net_ratio: number | null;
  super_large_net_inflow: number | null;
  large_net_inflow: number | null;
  medium_net_inflow: number | null;
  small_net_inflow: number | null;
}

export interface SectorSummaryPage {
  kind: SectorKind;
  page: number;
  page_size: number;
  total: number;
  items: SectorSummary[];
  as_of: string;
  source: string;
  stale: boolean;
}

export interface SectorMember {
  code: string;
  market: string;
  name: string;
  price: number | null;
  change_amount: number | null;
  change_pct: number | null;
  amount: number | null;
  turnover_rate: number | null;
  market_cap: number | null;
  pe: number | null;
  pb: number | null;
}

export interface SectorMemberPage {
  kind: SectorKind;
  sector_code: string;
  page: number;
  page_size: number;
  total: number;
  items: SectorMember[];
  as_of: string;
  source: string;
  stale: boolean;
}

export interface SectorLimitUpStats {
  sector_code: string;
  limit_up_count: number;
  member_count: number;
  as_of: string;
  source: string;
  methodology: string;
}

export type SectorPeriod = 'daily' | 'weekly' | 'monthly';
export type RotationDays = 3 | 5 | 10;

export interface SectorKline {
  date: string;
  open: number;
  close: number;
  high: number;
  low: number;
  volume: number;
  amount: number;
  change_pct: number | null;
  turnover_rate: number | null;
}

export interface SectorHistory {
  sector_code: string;
  period: SectorPeriod;
  items: SectorKline[];
  as_of: string;
  source: string;
}

export interface RotationStatus {
  kind: SectorKind;
  ok: boolean;
  error: string | null;
  as_of: string;
}

export interface SectorRotation {
  items: SectorSummary[];
  statuses: RotationStatus[];
  source: string;
  request_count: number;
}
