import assert from 'node:assert/strict';
import { mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { uiHarness } from './ui-check-harness.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = path.join(root, 'src-tauri/target/tmp');
mkdirSync(temp, { recursive: true });
for (const key of ['TEMP', 'TMP', 'TMPDIR']) process.env[key] = temp;

// Real layout, components and stores; every IPC call is isolated in memory.
function installMock() {
  const indices = ['上证指数', '深证成指', '创业板指', '沪深300', '中证500', '中证1000', '科创50'].map((name, i) => ({
    code: ['sh000001', 'sz399001', 'sz399006', 'sh000300', 'sh000905', 'sh000852', 'sh000688'][i], name,
    price: [3308.25, 10873.64, 2189.28, 3960.12, 6128.72, 6748.35, 1085.92][i],
    change: i % 2 ? -35.28 : 21.56, change_pct: i % 2 ? -0.84 : 1.25, volume: 123456789, turnover: 4567890123,
  }));
  const items = ['浦发银行', '贵州茅台', '宁德时代'].map((name, i) => ({
    id: i + 1, code: ['sh600000', 'sh600519', 'sz300750'][i], name, market: 'CN', sort_order: i, added_at: '2026-10-05',
  }));
  const quotes = items.map((item, i) => ({ ...item, price: [10.25, 1680.56, 248.32][i], change: i % 2 ? -2.45 : 1.26,
    change_pct: i % 2 ? -0.84 : 1.25, prev_close: [8.99, 1683.01, 247.06][i], open: 10, high: 1700, low: 8,
    volume: 1234567, turnover: 23456789, turnover_rate: 1.52, timestamp: Date.now() }));
  const handlers = new Map();
  const state = window.__layoutMock = { calls: [], unexpected: [], indices,
    status: { enabled: true, platformSupported: true, state: 'ready', phase: null, enginePath: null, updaterPath: null,
      updaterAvailable: true, owned: false, busy: false, message: '合成 IPC 就绪', lastError: null, candidates: [] },
    dispatch(name, payload) { for (const fn of handlers.get(name) || []) fn({ payload }); },
  };
  return {
    async invoke(command, args) {
      state.calls.push({ command, args });
      if (command === 'get_settings') return { theme: 'light', visual_style: 'modern', local_history_enabled: '1', active_datasource: 'tencent' };
      if (command === 'get_stockdb_status') return structuredClone(state.status);
      if (command === 'list_datasources') return [['tencent', '腾讯行情'], ['sina', '新浪行情']];
      if (command === 'get_portable_mode') return true;
      if (command === 'get_indices') return structuredClone(indices);
      if (command === 'get_quotes') return structuredClone(quotes);
      if (command === 'get_group_snapshot') return { active_group_id: 0, groups: [{ id: 0, name: '全部', sort_order: 0, item_count: items.length }], items };
      if (['get_holdings', 'get_monitors', 'get_intraday'].includes(command)) return [];
      if (command === 'take_pending_navigation') return null;
      if (command === 'run_stockdb_update') return '合成 IPC：更新请求已提交';
      if (command === 'switch_datasource') return null;
      state.unexpected.push(command);
      throw Error('Unexpected synthetic IPC: ' + command);
    },
    async listen(name, fn) {
      if (!handlers.has(name)) handlers.set(name, new Set());
      handlers.get(name).add(fn);
      return () => handlers.get(name).delete(fn);
    },
  };
}
const mock = 'const api = (' + installMock.toString() + ')();\n' + `
export const invoke = api.invoke;
export const listen = api.listen;
export async function emit(name, payload) { window.__layoutMock.dispatch(name, payload); }
export async function enable() {}
export async function disable() {}
export async function isEnabled() { return false; }
export async function getVersion() { return '2.2.6'; }
export async function openUrl() { throw Error('External navigation is outside this check'); }
export async function open() { return null; }
`;
const entry = `
import { createApp, h, reactive, nextTick } from 'vue';
import { createPinia } from 'pinia';
import { NConfigProvider, NMessageProvider, darkTheme, lightTheme } from 'naive-ui';
import AppLayout from '/src/components/layout/AppLayout.vue';
import { useSettingsStore } from '/src/stores/settings.ts';
import { useQuoteStore } from '/src/stores/quote.ts';
import { useWatchlistStore } from '/src/stores/watchlist.ts';
import '/src/assets/styles/variables.css';
import '/src/assets/styles/dark.css';
import '/src/assets/workspace.css';
const pinia = createPinia();
const settings = useSettingsStore(pinia), quote = useQuoteStore(pinia), watchlist = useWatchlistStore(pinia);
window.__layoutErrors = [];
await Promise.all([settings.fetchSettings(), settings.initStockDbListener(), quote.startListening(), watchlist.fetchWatchlist()]);
const view = reactive({ theme: 'light', style: 'modern' });
window.__layoutView = async (theme, style) => {
  settings.applyTheme(theme); settings.applyVisualStyle(style);
  view.theme = theme; view.style = style;
  await nextTick(); await document.fonts.ready;
};
const app = createApp({ render() {
  const tokens = getComputedStyle(document.documentElement);
  return h(NConfigProvider, { theme: view.theme === 'dark' ? darkTheme : lightTheme,
    themeOverrides: { common: { fontFamily: tokens.getPropertyValue('--font-sans').trim(), fontSize: tokens.fontSize,
      primaryColor: tokens.getPropertyValue('--color-accent').trim(), borderRadius: tokens.getPropertyValue('--radius-sm').trim() } } },
    () => h(NMessageProvider, () => h(AppLayout, { initReady: true })));
} });
app.config.errorHandler = error => window.__layoutErrors.push(String(error));
app.use(pinia).mount('#app');
window.__layoutReady = true;
`;
const h = await uiHarness({ name: 'workspace-layout', entry, mock });
const { evaluate, wait } = h;
const results = [];
async function clickElement(expression) {
  await evaluate('(()=>{const el=(' + expression + ');if(!el)throw Error("Missing target");el.scrollIntoView({block:"nearest",inline:"nearest"})})()');
  await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
  const point = await evaluate('(()=>{const el=(' + expression + '),r=el.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;if(!el.contains(document.elementFromPoint(x,y)))throw Error("Target clipped or covered");return {x,y}})()');
  await h.call('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
  await h.call('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
  await h.call('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 });
}
const snapshot = `(() => {
  const rect = el => el.getBoundingClientRect().toJSON();
  const css = el => getComputedStyle(el);
  const bar = document.querySelector('.index-bar'), header = document.querySelector('.watchlist-header');
  const props = ['height', 'paddingLeft', 'paddingRight', 'borderWidth', 'borderStyle', 'borderColor', 'borderRadius',
    'backgroundColor', 'color', 'fontSize', 'fontFamily', 'fontWeight', 'lineHeight', 'gap'];
  const buttons = [...document.querySelectorAll('.nav-primary')];
  const button = el => ({ label: el.innerText.trim(), rect: rect(el), style: Object.fromEntries(props.map(p => [p, css(el)[p]])),
    icon: rect(el.querySelector('svg')), text: rect(el.querySelector('span')) });
  const cards = [...bar.querySelectorAll('.index-card')].map(el => {
    const price = el.querySelector('.index-price'), row = el.querySelector('.index-change-row');
    return { rect: rect(el), background: css(el).backgroundColor, align: css(el).alignItems,
      border: css(el).borderLeftWidth, name: rect(el.querySelector('.index-name')), price: rect(price), priceFont: css(price).fontFamily,
      priceSize: parseFloat(css(price).fontSize), priceColor: css(price).color, footer: rect(row), footerAlign: css(row).justifyContent,
      change: [...row.children].map(rect) };
  });
  return { viewport: [innerWidth, innerHeight], documentWidth: document.documentElement.scrollWidth,
    theme: document.documentElement.dataset.theme, style: document.documentElement.dataset.style,
    appearance: document.documentElement.dataset.appearance || null,
    top: rect(document.querySelector('.top-bar')), toolbar: rect(document.querySelector('.top-bar-right')), topHeight: parseFloat(css(document.documentElement).getPropertyValue('--header-height')), scrollbar: bar.offsetHeight - bar.clientHeight, bar: rect(bar), header: rect(header), padding: [parseFloat(css(bar).paddingLeft), parseFloat(css(bar).paddingRight)],
    barHeight: parseFloat(css(document.documentElement).getPropertyValue('--index-bar-height')), gap: parseFloat(css(bar).columnGap), scrollWidth: bar.scrollWidth, clientWidth: bar.clientWidth, scrollLeft: bar.scrollLeft,
    nav: buttons.map(button), source: button(document.querySelector('.ds-tag')), cards };
})()`;
const near = (a, b, label) => assert.ok(Math.abs(a - b) <= 1, label + ': ' + a + ' vs ' + b);
function verify(layout, style, desktop) {
  assert.ok(layout.documentWidth <= layout.viewport[0], 'Document horizontal overflow: ' + JSON.stringify(layout.viewport));
  assert.equal(layout.cards.length, 7);
  near(layout.bar.height, layout.barHeight + layout.scrollbar, 'Index height matches the detail positioning token');
  near(layout.top.height, layout.topHeight, 'Header matches the layout token');
  near(layout.top.left + layout.padding[0], layout.header.left, 'Toolbar/index left gutter');
  near(layout.toolbar.right, layout.header.right, 'Toolbar/index right gutter');
  assert.equal(layout.nav.length, 8, 'All navigation and manual data update remain present');
  assert.equal(layout.appearance, style === 'elegant' ? 'elegant' : null);
  const regular = layout.nav[0].style;
  for (const button of layout.nav) {
    assert.deepEqual(button.style, regular, 'Inconsistent navigation: ' + button.label);
    near(button.icon.width, button.icon.height, 'Square icon');
    near(button.icon.width, style === 'elegant' ? 16 : 14, 'Uniform icon size');
    near(button.icon.y + button.icon.height / 2, button.rect.y + button.rect.height / 2, 'Icon vertical alignment');
    near(button.text.y + button.text.height / 2, button.rect.y + button.rect.height / 2, 'Text vertical alignment');
  }
  if (style !== 'classic') assert.deepEqual(layout.source.style, regular, 'Data source must coordinate with navigation');
  for (const card of layout.cards) near(card.rect.width, layout.cards[0].rect.width, 'Equal index widths');
  near(layout.cards[0].rect.left, layout.header.left, 'Index/watchlist left alignment');
  near(layout.bar.right - layout.padding[1], layout.header.right, 'Index/watchlist right gutter');
  if (desktop) {
    assert.ok(layout.scrollWidth <= layout.clientWidth + 1, 'Seven cards must fit desktop width');
    near(layout.cards.at(-1).rect.right, layout.header.right, 'Seven cards fill the right edge');
  } else if (style !== 'classic') {
    assert.ok(layout.scrollWidth > layout.clientWidth, 'Narrow indices scroll inside the bar');
    assert.ok(layout.cards[0].rect.width >= 144, 'Readable minimum card width');
  }
  if (style === 'elegant') {
    near(parseFloat(regular.height), 36, 'Elegant navigation height');
    assert.equal(regular.fontSize, '14px');
    for (const card of layout.cards) {
      assert.equal(card.background, layout.cards[0].background, 'Neutral cards for both gains and losses');
      assert.equal(card.align, 'flex-start');
      assert.equal(card.footerAlign, 'space-between');
      assert.equal(card.border, '3px');
      assert.ok(card.priceSize >= 24, 'Large non-code numbers');
      assert.ok(!/Consolas|Menlo|monospace/i.test(card.priceFont), 'Use the appearance body font for prices');
      assert.equal(card.priceColor, layout.cards[0].priceColor);
      near(card.name.left, card.price.left, 'Left-aligned name and price');
      near(card.change[0].left, card.price.left, 'Left-aligned change amount');
      near(card.change[1].right, card.footer.right, 'Right-aligned percentage');
      for (const box of [card.name, card.price, ...card.change]) {
        assert.ok(box.left >= card.rect.left && box.right <= card.rect.right + 1, 'Card content horizontal overflow');
        assert.ok(box.top >= card.rect.top && box.bottom <= card.rect.bottom + 1, 'Card content vertical overflow');
      }
    }
  } else assert.equal(layout.cards[0].align, 'center', 'Original appearance card alignment');
}
try {
  await wait('window.__layoutReady && document.querySelectorAll(".index-card").length === 7');
  for (const style of ['modern', 'elegant']) for (const theme of ['light', 'dark']) {
    await evaluate('window.__layoutView(' + JSON.stringify(theme) + ',' + JSON.stringify(style) + ')');
    for (const width of [2560, 1920, 1280, 640, 360]) {
      await h.call('Emulation.setDeviceMetricsOverride', { width, height: width < 640 ? 780 : 900, deviceScaleFactor: 1, mobile: false });
      await h.call('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 0, y: 0 });
      await evaluate('document.querySelector(".index-bar").scrollLeft=0;document.querySelector(".top-bar-right").scrollLeft=0;new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
      const layout = await evaluate(snapshot);
      verify(layout, style, width >= 1280);
      await h.screenshot(style + '-' + theme + '-' + width);
      if (width < 1280) {
        await evaluate('document.querySelector(".index-bar").scrollLeft=document.querySelector(".index-bar").scrollWidth;document.querySelector(".top-bar-right").scrollLeft=document.querySelector(".top-bar-right").scrollWidth');
        const scrolled = await evaluate(snapshot);
        near(scrolled.cards.at(-1).rect.right, scrolled.header.right, 'Last card reaches the same right gutter after scrolling');
        assert.ok(scrolled.documentWidth <= width);
        await h.screenshot(style + '-' + theme + '-' + width + '-scroll-end');
      }
      results.push({ style, theme, width, layout });
      console.log(style + '/' + theme + ' ' + width + ': 7 equal cards, aligned gutters, uniform navigation, no document overflow');
    }
  }
  await h.call('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
  await evaluate('window.__layoutView("light","classic")');
  await evaluate('document.querySelector(".index-bar").scrollLeft=0;new Promise(r=>requestAnimationFrame(r))');
  const classic = await evaluate(snapshot);
  verify(classic, 'classic', true);
  results.push({ style: 'classic', theme: 'light', width: 1280, layout: classic });
  await h.screenshot('classic-light-1280');
  await evaluate('window.__layoutView("light","modern")');
  await clickElement('document.querySelector(".ds-tag")');
  await wait('[...document.querySelectorAll(".n-dropdown-option")].some(el=>el.innerText.trim()==="新浪行情")');
  await clickElement('[...document.querySelectorAll(".n-dropdown-option-body")].find(el=>el.innerText.trim()==="新浪行情")');
  await wait('document.querySelector(".ds-label").innerText==="新浪行情"');
  assert.equal(await evaluate('window.__layoutMock.calls.filter(c=>c.command==="switch_datasource").at(-1).args.name'), 'sina');
  await clickElement('[...document.querySelectorAll(".nav-primary")].find(el=>el.innerText.trim()==="更新数据")');
  await wait('window.__layoutMock.calls.filter(c=>c.command==="run_stockdb_update").length===1');
  await evaluate('window.__layoutMock.dispatch("stockdb-status-changed", {...window.__layoutMock.status,state:"updating",busy:true,message:"合成 IPC：正在更新"})');
  await wait('document.querySelector(".nav-primary.spinning")?.disabled');
  await clickElement('[...document.querySelectorAll(".nav-primary")].find(el=>el.innerText.trim()==="更新数据")');
  assert.equal(await evaluate('window.__layoutMock.calls.filter(c=>c.command==="run_stockdb_update").length'), 1);
  await evaluate('window.__layoutMock.dispatch("stockdb-status-changed", window.__layoutMock.status);document.querySelector(".index-card").focus()');
  await h.call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
  await h.call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
  await wait('document.querySelector(".index-detail") && document.querySelector(".card-selected")');
  await evaluate('document.querySelector(".index-detail .detail-close").click()');
  await wait('!document.querySelector(".index-detail")');
  await evaluate('document.querySelector(".index-card").click()');
  await wait('document.querySelector(".index-detail")');
  await evaluate('document.querySelector(".index-card").focus()');
  await h.call('Input.dispatchKeyEvent', { type: 'keyDown', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 });
  await h.call('Input.dispatchKeyEvent', { type: 'keyUp', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 });
  await wait('!document.querySelector(".index-detail")');
  for (const style of ['classic', 'modern', 'elegant']) {
    await evaluate('window.__layoutView("light",' + JSON.stringify(style) + ')');
    for (const [width, height] of [[640, 360], [360, 320]]) {
      await h.call('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
      await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
      for (const selector of ['.add-btn', '.group-tabs button:last-child', '.watchlist-table button']) {
        await evaluate('document.querySelector(' + JSON.stringify(selector) + ').scrollIntoView({block:"center",inline:"center"})');
        await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
        const target = await evaluate('(()=>{const b=document.querySelector(' + JSON.stringify(selector) + '),r=b.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return{rect:r.toJSON(),hit:b.contains(document.elementFromPoint(x,y)),text:b.innerText}})()');
        assert.ok(target.hit, style + ' ' + width + 'x' + height + ' unreachable main-window control: ' + JSON.stringify(target));
      }
      assert.ok(await evaluate('document.documentElement.scrollWidth<=innerWidth'), style + ' short window horizontal overflow');
    }
  }
  assert.deepEqual(await evaluate('window.__layoutMock.unexpected'), []);
  assert.deepEqual(await evaluate('window.__layoutErrors'), []);
  assert.deepEqual(h.errors, []);
  h.save({ passed: true, scenarios: results, functional: ['data source switch', 'manual data update', 'busy update disabled', 'index Enter/click/Space/detail close'], syntheticIpc: true });
  console.log('Workspace layout UI regression passed. Screenshots and metrics: ' + h.output);
} catch (error) {
  await h.screenshot('failure').catch(() => {});
  h.save({ passed: false, error: String(error), scenarios: results, browserErrors: h.errors,
    componentErrors: await evaluate('window.__layoutErrors').catch(() => null), unexpectedIpc: await evaluate('window.__layoutMock?.unexpected').catch(() => null) });
  console.error('Failure artifacts: ' + h.output);
  throw error;
} finally {
  await h.close();
}
