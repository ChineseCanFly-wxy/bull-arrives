import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { calculateHoldingMetrics, decimalToScaledPrice, formatScaledMoney, scaledPriceToDecimal } from './holdings.ts';

test('成本四位小数往返、空成本和零成本区分', () => {
  assert.equal(decimalToScaledPrice('12.3456'), '123456');
  assert.equal(scaledPriceToDecimal('123456'), '12.3456');
  assert.equal(decimalToScaledPrice(''), null);
  assert.equal(decimalToScaledPrice('0'), '0');
  assert.throws(() => decimalToScaledPrice('-1'));
  assert.throws(() => decimalToScaledPrice('1.12345'));
});
test('盈亏方向、收益率和市值按定点计算', () => {
  assert.deepEqual(calculateHoldingMetrics({ cost_price: '100000', shares: 100 }, 10.5), { marketValue: '1050.00', profit: '50.00', profitRate: '5.00%' });
  assert.deepEqual(calculateHoldingMetrics({ cost_price: '100000', shares: 100 }, 9.5), { marketValue: '950.00', profit: '-50.00', profitRate: '-5.00%' });
});
test('零成本、无成本、零股数不除零', () => {
  assert.equal(calculateHoldingMetrics({cost_price: '0', shares: 100}, 10).profitRate, null);
  assert.equal(calculateHoldingMetrics({cost_price: '0', shares: 100}, 10).profit, '1000.00');
  assert.equal(calculateHoldingMetrics({cost_price: null, shares: 100}, 10).profit, null);
  assert.equal(calculateHoldingMetrics({cost_price: '10000', shares: 0}, 10).profitRate, null);
});
test('金额半入舍入正负对称且拒绝无效输入', () => {
  assert.equal(formatScaledMoney(1050n), '0.11');
  assert.equal(formatScaledMoney(-1050n), '-0.11');
  assert.throws(() => calculateHoldingMetrics({cost_price:'1',shares:0.5},10));
  assert.throws(() => calculateHoldingMetrics({cost_price:'1',shares:100},NaN));
});
