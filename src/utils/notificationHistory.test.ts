import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { applyHistoryWatermark, mergeHistoryEntries } from './notificationHistory.ts';
const entry = (id: string, history_version: number, received_at = 1) => ({ id, history_version, received_at });
test('清空后迟到的旧事件无法恢复记录', () => {
  const state = applyHistoryWatermark({ version: 0, entries: [entry('old', 0)] }, 1);
  assert.deepEqual(mergeHistoryEntries(state, [entry('old', 0)]).entries, []);
});
test('迟到的清空响应不删除新提醒', () => {
  const state = { version: 1, entries: [entry('new', 1)] };
  assert.equal(applyHistoryWatermark(state, 1).entries.length, 1);
  assert.equal(applyHistoryWatermark(state, 0).entries.length, 1);
});
test('同版本旧快照不能覆盖实时新消息', () => {
  const state = { version: 1, entries: [entry('new', 1, 2)] };
  assert.deepEqual(mergeHistoryEntries(state, [entry('old', 1)]).entries.map(x => x.id), ['new', 'old']);
});
test('新一代事件先到达时自动淘汰清空前历史', () => {
  const state = mergeHistoryEntries({ version: 0, entries: [entry('old', 0)] }, [entry('new', 1)]);
  assert.equal(state.version, 1);
  assert.deepEqual(state.entries.map(x => x.id), ['new']);
});
