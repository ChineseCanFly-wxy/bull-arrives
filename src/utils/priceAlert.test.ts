import assert from 'node:assert/strict';
import test from 'node:test';
import type { PriceAlert } from '../types/index.ts';
import {
  buildPriceAlertPayload,
  defaultPriceAlertDraft,
  toPriceAlertDraft,
  validatePriceAlertDraft,
} from './priceAlert.ts';

test('unconfigured rules remain absent instead of creating placeholders', () => {
  const draft = defaultPriceAlertDraft('fixed_price');
  assert.equal(validatePriceAlertDraft(draft), null);
  assert.equal(buildPriceAlertPayload(draft, '600519', 'CN'), null);
});

test('change percentage accepts either direction but rejects zero', () => {
  const draft = defaultPriceAlertDraft('change_pct');
  draft.configured = true;
  draft.enabled = true;
  draft.threshold = -3.5;
  assert.equal(validatePriceAlertDraft(draft), null);
  draft.threshold = 0;
  assert.equal(validatePriceAlertDraft(draft), '涨跌幅阈值不能为 0');
});

test('fixed price must be positive', () => {
  const draft = defaultPriceAlertDraft('fixed_price');
  draft.configured = true;
  draft.threshold = -1;
  assert.equal(validatePriceAlertDraft(draft), '固定价格必须大于 0');
  draft.threshold = 12.34;
  assert.equal(validatePriceAlertDraft(draft), null);
});

test('cooldown is an integer in the inclusive 1..1440 range', () => {
  const draft = defaultPriceAlertDraft('fixed_price');
  draft.configured = true;
  draft.threshold = 10;
  draft.repeat_mode = 'cooldown';
  for (const invalid of [0, 1441, 1.5]) {
    draft.cooldown_minutes = invalid;
    assert.equal(validatePriceAlertDraft(draft), '冷却时间须为 1–1440 分钟的整数');
  }
  draft.cooldown_minutes = 1440;
  assert.equal(validatePriceAlertDraft(draft), null);
});

test('existing IPC fields are retained in a complete save payload', () => {
  const existing: PriceAlert = {
    id: 42,
    code: '600519',
    market: 'CN',
    alert_type: 'fixed_price',
    threshold: 1600,
    enabled: false,
    repeat_mode: 'crossing',
    cooldown_minutes: 0,
    last_triggered_at: '2026-09-09T10:00:00+08:00',
    last_triggered_day: '2026-09-09',
  };
  const draft = toPriceAlertDraft('fixed_price', existing);
  draft.enabled = true;
  draft.threshold = 1650;
  assert.deepEqual(buildPriceAlertPayload(draft, existing.code, existing.market), {
    ...existing,
    enabled: true,
    threshold: 1650,
  });
});

test('non-cooldown payloads normalize cooldown minutes to zero', () => {
  const draft = defaultPriceAlertDraft('change_pct');
  Object.assign(draft, {
    configured: true,
    enabled: true,
    threshold: 5,
    repeat_mode: 'daily',
    cooldown_minutes: 30,
  });
  assert.equal(buildPriceAlertPayload(draft, '000001', 'CN')?.cooldown_minutes, 0);
});
