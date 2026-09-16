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
