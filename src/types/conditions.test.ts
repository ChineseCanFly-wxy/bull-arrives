import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { defaultConditionTree, describeCondition, isConditionTree } from './conditions.ts';
test('恢复观察偏好时只接受当前有效的条件树', () => {
  const valid=defaultConditionTree();
  assert.equal(isConditionTree(valid),true);
  assert.equal(describeCondition(valid),'(强势主线领涨 且 涨跌幅 ≥ 1%)');
  assert.equal(isConditionTree(null),false);
  assert.equal(isConditionTree({op:'and',children:[]}),false);
  assert.equal(isConditionTree({op:'change_above',value:NaN}),false);
  assert.equal(isConditionTree({op:'change_below',value:31}),false);
  assert.equal(isConditionTree({op:'and',children:[{op:'mainline'},{op:'intraday'}]}),false);
  let deep:unknown={op:'mainline'};
  for(let i=0;i<4;i++)deep={op:'or',children:[deep,{op:'mainline'}]};
  assert.equal(isConditionTree(deep),false);
});
