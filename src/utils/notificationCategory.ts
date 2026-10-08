export type NoticeCategory = 'news' | 'mainline' | 'trades' | 'conditions' | 'research' | 'price' | 'risk' | 'data';
export function noticeSection(category: string): string {
  if (['trades', 'conditions'].includes(category)) return 'research';
  if (category === 'risk') return 'price';
  if (category === 'briefs') return 'news';
  return category === 'data' ? 'operations' : category;
}
export function noticeCategory(row: { signal_kind?: string; sector_code?: unknown; model_snapshot?: { follow_account_id?: unknown } | null; condition_event?: unknown; condition_events?: unknown[]; stockdb_update_alert?: { schema?: string } }): NoticeCategory {
  if (row.signal_kind === 'news') return 'news';
  if (row.stockdb_update_alert?.schema === 'stockdb-update-failed-v1') return 'data';
  if (row.signal_kind === 'research') {
    if (typeof row.sector_code === 'string' && row.sector_code) return 'mainline';
    if (typeof row.model_snapshot?.follow_account_id === 'number') return 'trades';
    if (row.condition_event || row.condition_events?.length) return 'conditions';
    return 'research';
  }
  return row.signal_kind === 'risk' ? 'risk' : 'price';
}
