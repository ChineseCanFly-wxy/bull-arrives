export interface VersionedHistoryEntry {
  id: string;
  history_version: number;
  received_at: number;
}
export interface HistoryState<T extends VersionedHistoryEntry> {
  version: number;
  entries: T[];
}
export function applyHistoryWatermark<T extends VersionedHistoryEntry>(state: HistoryState<T>, watermark: number): HistoryState<T> {
  const version = Math.max(state.version, watermark);
  return { version, entries: state.entries.filter(entry => entry.history_version >= version) };
}
export function mergeHistoryEntries<T extends VersionedHistoryEntry>(state: HistoryState<T>, incoming: T[]): HistoryState<T> {
  const version = incoming.reduce((value, entry) => Math.max(value, entry.history_version), state.version);
  const clean = applyHistoryWatermark(state, version);
  const byId = new Map(clean.entries.map(entry => [entry.id, entry]));
  for (const entry of incoming) {
    if (entry.history_version >= version && !byId.has(entry.id)) byId.set(entry.id, entry);
  }
  return { version, entries: [...byId.values()].sort((a, b) => b.received_at - a.received_at).slice(0, 50) };
}
