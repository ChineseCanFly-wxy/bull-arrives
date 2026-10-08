import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { noticeCategory, noticeSection } from './notificationCategory.ts';
test('资讯、主线与每种研究提醒各归其位', () => {
  assert.equal(noticeCategory({signal_kind:'news', sector_code:'BK0001'}), 'news');
  assert.equal(noticeCategory({signal_kind:'news', model_snapshot:{follow_account_id:1}}), 'news');
  assert.equal(noticeCategory({signal_kind:'research', sector_code:'BK0001'}), 'mainline');
  assert.equal(noticeCategory({signal_kind:'research', model_snapshot:{follow_account_id:1}}), 'trades');
  assert.equal(noticeCategory({signal_kind:'research', condition_event:{}}), 'conditions');
  assert.equal(noticeCategory({signal_kind:'research', condition_events:[{}]}), 'conditions');
  assert.equal(noticeCategory({signal_kind:'research', model_snapshot:{}}), 'research');
  assert.equal(noticeCategory({signal_kind:'risk'}), 'risk');
  assert.equal(noticeCategory({signal_kind:'price'}), 'price');
  assert.equal(noticeCategory({signal_kind:'system',stockdb_update_alert:{schema:'stockdb-update-failed-v1'}}), 'data');
});

test('合并入口保留每种提醒的独立分类', () => {
  assert.deepEqual(
    ['news', 'mainline', 'trades', 'conditions', 'research', 'price', 'risk', 'data', 'briefs', 'operations', 'floating'].map(noticeSection),
    ['news', 'mainline', 'research', 'research', 'research', 'price', 'price', 'operations', 'news', 'operations', 'floating'],
  );
});
