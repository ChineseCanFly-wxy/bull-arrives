import type { SectorKind } from './sector';

/** A read-only observation destination. It never selects a trading account. */
export interface MainlineNavigationTarget {
  kind: SectorKind;
  code: string;
  name: string;
  snapshotFingerprint?: string;
}
