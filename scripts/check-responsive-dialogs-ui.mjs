import assert from 'node:assert/strict';
import { uiHarness } from './ui-check-harness.mjs';

// Real dialogs and stores, with isolated IPC. Mouse events are captured before
// application handlers so this layout audit never changes user data.
function mockApi() {
  const state = window.__responsiveMock = { calls: [], unsupported: [] };
  const config = { auto_research: false, observation_days: 28, min_samples: 20,
    max_drawdown_bps: 1500, min_return_bps: 0, initial_cash: '1000000000',
    max_active: 3, stock_count: 5, commission_bps: 3, min_commission: '50000',
    stamp_tax_bps: 5, transfer_fee_bps: 0, slippage_bps: 0 };
  return async (command, args = {}) => {
    state.calls.push({ command, args });
    if (command === 'get_settings') return { theme: 'light', visual_style: 'modern', local_history_enabled: '0' };
    if (command === 'get_holdings') return [{ watch_id: 1, cost_price: '100000', shares: 100 }];
    if (['get_price_alerts', 'get_filter_presets', 'get_monitors', 'get_monitor_rules',
      'list_monitor_rules', 'research_job_list', 'research_follow_accounts',
      'list_interactive_analyses', 'get_daily_briefs', 'get_news_history',
      'get_news_archive', 'get_alert_archive', 'get_mainline_alert_history'].includes(command)) return [];
    if (command === 'list_datasources') return [['tencent', '腾讯行情']];
    if (command === 'get_portable_mode') return true;
    if (command === 'set_setting') return;
    if (command === 'get_market_session') return { session: '休市', interval_secs: 30, is_trading: false };
    if (command === 'get_group_snapshot') return { active_group_id: 0, groups: [], items: [] };
    if (['get_stockdb_status', 'scan_stockdb'].includes(command)) return { enabled: false, platformSupported: true,
      state: 'disabled', message: '隔离布局检查', busy: false, candidates: [] };
    if (command === 'get_agent_status') return { installed: false, state: 'unavailable',
      path: null, message: '隔离布局检查', guidance: '', run_dir: null };
    if (command === 'get_notification_history') return { version: 0, entries: [] };
    if (command === 'get_automatic_operations_log') return { enabled: true, entries: [], version: 0 };
    if (command === 'research_dashboard') return { config, experiments: [], local_ready: false,
      agent_installed: false, last_auto_message: '', busy: false };
    if (command === 'research_model_runs') return { runs: [], tasks: [],
      config: { research_root: '', python: 'python', snapshot: '', index: '' }, last_error: null };
    if (command === 'research_auto_status') return { enabled: false, state: 'paused', models: [], message: '隔离检查' };
    if (command === 'get_mainline_discovery_status') return { enabled: false, busy: false,
      as_of: '', total: 0, processed: 0, failed: [], candidates: [], catalog_errors: [] };
    if (command === 'get_sector_summaries') return { items: [], total: 0, page: 1,
      page_size: 30, kind: 'industry', as_of: '', source: 'cache', stale: false };
    if (command === 'scan_and_rank') return { items: [], scanned: 0, total_matched: 0,
      candidates: 0, scored: 0, failed: 0, skipped_conditions: [], source: 'cache', stale: false };
    if (command === 'model_condition_watches') return { watches: [], presets: [], limit_per_tick: 20 };
    state.unsupported.push(command);
    throw Error('No isolated fixture for ' + command);
  };
}
const mock = `export const invoke = (${mockApi.toString()})();
export async function listen() { return () => {}; }
export async function emit() {}
export async function open() { return null; }
export async function openUrl() {}
export async function isEnabled() { return false; }
export async function enable() {}
export async function disable() {}
export async function getVersion() { return '3.0.4'; }
`;
const entry = `
import { createApp, h, reactive, nextTick, markRaw } from 'vue';
import { createPinia } from 'pinia';
import { NConfigProvider, NMessageProvider, NModal, NCard, NButton, darkTheme } from 'naive-ui';
import { useSettingsStore } from '/src/stores/settings.ts';
import { useUniverseStore } from '/src/stores/universe.ts';
import { useUpdaterStore } from '/src/stores/updater.ts';
import '/src/assets/styles/variables.css';
import '/src/assets/styles/dark.css';
import '/src/assets/workspace.css';
const pinia = createPinia(), settings = useSettingsStore(pinia), updater = useUpdaterStore(pinia);
const view = reactive({ component: null, props: {}, theme: 'light', instance: null, clicks: 0 });
window.__responsiveView = view;
window.__responsiveErrors = [];
window.__responsiveSetStyle = async (style, theme) => {
  settings.applyVisualStyle(style); settings.applyTheme(theme); view.theme = theme;
  await nextTick();
};
window.__responsiveOpen = async (name, props) => {
  view.component = null; view.instance = null; await nextTick();
  if (name === 'updater') {
    updater.dialogVisible = true; updater.updateStatus = 'available';
    updater.updateInfo = { version: '3.0.5', current_version: '3.0.4',
      notes: '## 更新内容\\n' + Array.from({ length: 30 }, (_, i) => '- 改进说明 ' + i).join('\\n') };
  }
  const files = {
    settings: 'settings/SettingsDialog', holding: 'watchlist/HoldingDialog',
    price: 'watchlist/PriceAlertDialog', add: 'watchlist/AddStockDialog',
    updater: 'updater/UpdateDialog', monitor: 'monitor/MonitorDialog',
    sector: 'sector/SectorDialog', rank: 'rank/RankDialog',
    research: 'research/ResearchCenter', screener: 'screener/UniverseScreenerDialog',
    mainline: 'sector/MarketMainlineDialog', notices: 'settings/AlertNotifications',
  };
  if (name === 'card' || name === 'dialog') {
    view.component = markRaw({ render() {
      const body = () => Array.from({ length: 50 }, (_, i) => h('p', '长内容 ' + i));
      if (name === 'dialog') return h(NModal, { show: true, preset: 'dialog',
        title: '删除分组确认', positiveText: '确认删除', negativeText: '取消' }, body);
      return h(NModal, { show: true }, () => h(NCard, { title: '长内容与底部操作', closable: true,
        style: { width: '760px' } }, { default: body, footer: () => h(NButton, '保存') }));
    } });
  } else view.component = markRaw((await import(/* @vite-ignore */ '/src/components/' + files[name] + '.vue')).default);
  view.props = { show: true, ...props };
  if (name === 'rank') view.props.filter = useUniverseStore(pinia).filter;
  await nextTick();
  if (name === 'notices') view.instance.$.setupState.showHistory = true;
};
const app = createApp({ render() {
  return h(NConfigProvider, { theme: view.theme === 'dark' ? darkTheme : null },
    () => h(NMessageProvider, () => view.component ? h(view.component, {
      ...view.props, ref: instance => { view.instance = instance; },
    }) : null));
} });
app.config.errorHandler = error => window.__responsiveErrors.push(error.stack || String(error));
app.use(pinia).mount('#app');
document.addEventListener('click', event => {
  const target = window.__responsiveAuditTarget;
  if (target && target.contains(event.target)) {
    view.clicks++; event.preventDefault(); event.stopImmediatePropagation();
  }
}, true);
`;

const t = await uiHarness({ name: 'responsive-dialogs', entry, mock });
const results = [];
const item = { id: 1, code: 'sh600000', market: 'CN', name: '浦发银行', sort_order: 0, added_at: '2026-10-10' };
const dialogs = [
  ['settings', { initialSection: 'market' }], ['holding', { item }], ['price', { item }],
  ['add', {}], ['updater', {}], ['monitor', {}], ['sector', {}], ['rank', {}],
  ['research', {}], ['screener', {}], ['mainline', {}], ['notices', {}], ['card', {}], ['dialog', {}],
];
const idle = async () => {
  await t.evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
  await t.evaluate('Promise.all(document.getAnimations().filter(a=>Number.isFinite(a.effect.getComputedTiming().iterations)).map(a=>a.finished.catch(()=>{})))');
};
try {
  await t.wait('window.__responsiveOpen');
  for (const [name, props] of dialogs) {
    await t.call('Emulation.setDeviceMetricsOverride', { width: 1000, height: 680, deviceScaleFactor: 1, mobile: false });
    await t.evaluate('window.__responsiveOpen(' + JSON.stringify(name) + ',' + JSON.stringify(props) + ')');
    await t.wait('document.querySelector(".n-modal")'); await idle();
    for (const style of ['classic', 'modern', 'elegant']) for (const theme of ['light', 'dark']) {
      await t.evaluate('window.__responsiveSetStyle(' + JSON.stringify(style) + ',' + JSON.stringify(theme) + ')');
      for (const [width, height] of [[1000, 680], [800, 450], [360, 320]]) {
        await t.call('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
        await idle();
        const label = name + '/' + style + '/' + theme + ' ' + width + 'x' + height;
        const card = await t.evaluate('document.querySelector(".n-modal").getBoundingClientRect().toJSON()');
        assert.ok(card.top >= -1 && card.left >= -1 && card.bottom <= height + 1 && card.right <= width + 1,
          label + ' dialog escapes viewport: ' + JSON.stringify(card));
        // Audit bottom actions and close controls with actual pointer events.
        const count = await t.evaluate(`(() => {
          const card = document.querySelector('.n-modal');
          const buttons = [...card.querySelectorAll('button')].filter(b => !b.disabled && b.checkVisibility() && !b.closest('details:not([open])') && b.getBoundingClientRect().width > 0);
          const footer = [...card.querySelectorAll('.n-card__footer button,.n-dialog__action button')].filter(b => !b.disabled);
          window.__responsiveTargets = [...new Set([...buttons.slice(-2), ...footer, ...card.querySelectorAll('button[aria-label="close"]')])];
          return window.__responsiveTargets.length;
        })()`);
        assert.ok(count > 0, label + ' has no controls');
        for (let i = 0; i < count; i++) {
          await t.evaluate('window.__responsiveAuditTarget=window.__responsiveTargets[' + i + '];window.__responsiveAuditTarget.scrollIntoView({block:"center",inline:"nearest"})');
          await idle();
          const target = await t.evaluate(`(() => {
            const b = window.__responsiveAuditTarget, r = b.getBoundingClientRect(), x = r.x+r.width/2, y = r.y+r.height/2;
            return { x, y, label: b.innerText || b.ariaLabel, rect: r.toJSON(), hit: b.contains(document.elementFromPoint(x,y)), before: window.__responsiveView.clicks };
          })()`);
          assert.ok(target.hit, label + ' clipped/covered action: ' + JSON.stringify(target));
          await t.call('Input.dispatchMouseEvent', { type: 'mousePressed', x: target.x, y: target.y, button: 'left', clickCount: 1 });
          await t.call('Input.dispatchMouseEvent', { type: 'mouseReleased', x: target.x, y: target.y, button: 'left', clickCount: 1 });
          assert.equal(await t.evaluate('window.__responsiveView.clicks'), target.before + 1, label + ' pointer did not reach control');
        }
        await t.evaluate('window.__responsiveAuditTarget=null');
        results.push({ name, style, theme, width, height, controls: count });
        if (style === 'elegant' && theme === 'light' && width === 360) await t.screenshot(name + '-360x320');
      }
    }
    assert.deepEqual(await t.evaluate('window.__responsiveErrors'), [], name + ' render errors');
    console.log('PASS ' + name + ': viewport bounds and real pointer reachability in 18 scenarios');
  }
  assert.deepEqual(await t.evaluate('window.__responsiveErrors'), []);
  assert.deepEqual(t.errors, []);
  t.save({ passed: true, scenarios: results, unsupportedFixtures: await t.evaluate('window.__responsiveMock.unsupported'),
    limitation: 'Synthetic IPC; click audit intercepts application handlers and makes no real changes' });
  console.log(JSON.stringify({ passed: results.length, output: t.output }));
} catch (error) {
  await t.screenshot('failure').catch(() => {});
  t.save({ passed: false, scenarios: results, error: String(error),
    errors: await t.evaluate('window.__responsiveErrors'), unsupported: await t.evaluate('window.__responsiveMock.unsupported') });
  throw error;
} finally { await t.close(); }
