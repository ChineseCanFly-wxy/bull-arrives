import type {
  PriceAlert,
  PriceAlertRepeatMode,
  PriceAlertType,
} from '../types/index.ts';

export const PRICE_ALERT_TYPES: readonly PriceAlertType[] = [
  'change_pct',
  'fixed_price',
];

export interface PriceAlertDraft {
  alert_type: PriceAlertType;
  configured: boolean;
  enabled: boolean;
  threshold: number | null;
  repeat_mode: PriceAlertRepeatMode;
  cooldown_minutes: number;
  existing: PriceAlert | null;
}

export function defaultPriceAlertDraft(type: PriceAlertType): PriceAlertDraft {
  return {
    alert_type: type,
    configured: false,
    enabled: false,
    threshold: null,
    repeat_mode: type === 'change_pct' ? 'daily' : 'crossing',
    cooldown_minutes: 30,
    existing: null,
  };
}

export function toPriceAlertDraft(
  type: PriceAlertType,
  alert?: PriceAlert,
): PriceAlertDraft {
  if (!alert) return defaultPriceAlertDraft(type);

  return {
    alert_type: type,
    configured: true,
    enabled: alert.enabled,
    threshold: alert.threshold,
    repeat_mode: alert.repeat_mode,
    cooldown_minutes: clampCooldown(alert.cooldown_minutes || 30),
    existing: alert,
  };
}

export function validatePriceAlertDraft(draft: PriceAlertDraft): string | null {
  if (!draft.configured) return null;
  if (draft.threshold == null || !Number.isFinite(draft.threshold)) {
    return '请输入有效阈值';
  }
  if (draft.alert_type === 'change_pct' && draft.threshold === 0) {
    return '涨跌幅阈值不能为 0';
  }
  if (draft.alert_type === 'fixed_price' && draft.threshold <= 0) {
    return '固定价格必须大于 0';
  }
  if (
    draft.repeat_mode === 'cooldown'
    && (!Number.isInteger(draft.cooldown_minutes)
      || draft.cooldown_minutes < 1
      || draft.cooldown_minutes > 1440)
  ) {
    return '冷却时间须为 1–1440 分钟的整数';
  }
  return null;
}

export function buildPriceAlertPayload(
  draft: PriceAlertDraft,
  code: string,
  market: string,
): PriceAlert | null {
  if (!draft.configured || draft.threshold == null) return null;

  return {
    id: draft.existing?.id ?? 0,
    code,
    market,
    alert_type: draft.alert_type,
    threshold: draft.threshold,
    enabled: draft.enabled,
    repeat_mode: draft.repeat_mode,
    cooldown_minutes: draft.repeat_mode === 'cooldown'
      ? clampCooldown(draft.cooldown_minutes)
      : 0,
    last_triggered_at: draft.existing?.last_triggered_at ?? null,
    last_triggered_day: draft.existing?.last_triggered_day ?? null,
  };
}

export function clampCooldown(value: number): number {
  if (!Number.isFinite(value)) return 30;
  return Math.min(1440, Math.max(1, Math.round(value)));
}
