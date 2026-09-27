import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';

const assets = join(import.meta.dirname, '..', 'dist', 'assets');
const cssFiles = readdirSync(assets).filter(name => name.endsWith('.css'));
const css = cssFiles.map(name => readFileSync(join(assets, name), 'utf8')).join('\n');
const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)];

test('主题样式不能隐藏 html 根元素', () => {
  for (const [, selector, declarations] of rules) {
    if (!/(?:^|;)\s*display\s*:\s*none\s*(?:;|$)/.test(declarations)) continue;
    for (const part of selector.split(',')) {
      assert.doesNotMatch(part.trim(), /^(?:html)?\[data-style=(?:"|'|)?(?:classic|trading|modern)(?:"|'|)?\]$/, `根元素被隐藏：${part}`);
    }
  }
});

test('原版主题仅隐藏指定的局部元素', () => {
  assert.match(css, /html\[data-style=classic\] \.brand-mark\[data-v-[\da-f]+\][^{}]*\{display:none\}/);
  assert.match(css, /html\[data-style=classic\] \.section-kicker\[data-v-[\da-f]+\][^{}]*\{display:none\}/);
});
