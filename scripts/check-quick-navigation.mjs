import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { parse, compileScript, compileTemplate } from '@vue/compiler-sfc';
import ts from 'typescript';
import * as Vue from 'vue';

// Execute real Vue setup/watchers against memory-only IPC. No app, user DB,
// StockDB service, native menu or notification is launched by this check.
const calls = [], handlers = new Map(), mounted = [], errors = [], disposers = [];
let pending = 'research', confirmDiscard = false, confirmations = 0;
const settings = {
  settings: {}, datasources: [], activeDatasource: 'test', theme: 'light', visualStyle: 'classic',
  stockDbStatus: {}, localHistoryEnabled: false, localHistoryUrl: 'http://127.0.0.1:7899',
  tickerOpacity: 100, refreshInterval: 0, marketSession: { interval_secs: 3 },
  async fetchMarketSession() {}, async fetchStockDbStatus() {},
};
const invoke = async (command, args) => {
  calls.push({ command, args });
  if (command === 'take_pending_navigation') { const value = pending; pending = null; return value; }
  if (command === 'get_notification_identity_status') return { supported: false };
  if (command === 'get_agent_status') return { installed: false };
  if (command === 'get_build_info' || command === 'get_data_paths') return {};
  throw Error(`Unexpected IPC: ${command}`);
};
const listen = async (name, fn) => { calls.push({ subscription: name }); handlers.set(name, fn); return () => handlers.delete(name); };
const storage = new Map();
const fakeWindow = { confirm() { confirmations++; return confirmDiscard; }, addEventListener() {}, removeEventListener() {} };
function load(file, props, emit) {
  const { descriptor, errors: parseErrors } = parse(fs.readFileSync(file, 'utf8'), { filename: file });
  assert.deepEqual(parseErrors, []);
  assert.deepEqual(compileTemplate({ source: descriptor.template.content, filename: file, id: 'navigation-check' }).errors, []);
  const compiled = compileScript(descriptor, { id: 'navigation-check' });
  const vue = {
    ...Vue, defineAsyncComponent: () => ({}), onMounted: fn => mounted.push(fn),
    onUnmounted: fn => disposers.push(fn), onBeforeUnmount: fn => disposers.push(fn),
    watch: (...args) => { const stop = Vue.watch(...args); disposers.push(stop); return stop; },
  };
  const requireStub = id => {
    if (id === 'vue') return vue;
    if (id === '@tauri-apps/api/core') return { invoke };
    if (id === '@tauri-apps/api/event') return { listen };
    if (id === '@/stores/settings') return { useSettingsStore: () => settings, REFRESH_INTERVAL_AUTO: 0 };
    if (id === '@/stores/rank') return { useRankStore: () => ({}) };
    if (id === '@/stores/universe') return { useUniverseStore: () => ({ pageSize: 20 }) };
    if (id === 'naive-ui') return { useMessage: () => ({ error: text => errors.push(text), success() {}, warning() {} }) };
    return {};
  };
  const content = ts.transpileModule(compiled.content, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  const module = { exports: {} };
  vm.runInNewContext(content, { require: requireStub, exports: module.exports, module,
    window: fakeWindow, navigator: { userAgent: 'Windows' },
    sessionStorage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) }, console,
  });
  let exposed;
  const state = module.exports.default.setup(props, { expose: value => { exposed = value; }, emit });
  return { state, exposed };
}
const top = load('src/components/layout/TopBar.vue', {}, () => {}).state;
assert.equal(top.quickOptions.filter(item=>String(item.key).startsWith('settings')).length,1,'Application menu exposes one settings item');
assert.equal(top.quickOptions.find(item=>item.label==='设置').key,'settings');
assert.equal(top.quickOptions.find(item=>item.label==='市场主线').key,'mainline','Mainline has an independent quick destination');
const topSource=fs.readFileSync('src/components/layout/TopBar.vue','utf8');
assert.ok(topSource.indexOf('<n-dropdown')<topSource.indexOf('<span>研究中心</span>'),'Datasource precedes research center');
assert.ok(!topSource.includes('M16.4 12.4'),'Old distorted settings icon is removed');
await mounted[0]();
assert.equal(top.showResearch.value, true, 'Tray navigation pending before mount must open the research center');
assert.equal(pending, null, 'The startup destination is consumed once');
assert.ok(calls.findIndex(v => v.subscription === 'quick-navigation') < calls.findIndex(v => v.command === 'take_pending_navigation'), 'Subscribe before consuming pending navigation');
top.openMainline();
assert.equal(top.showMainline.value,true,'Topbar opens independent market mainline');
assert.equal(top.showResearch.value,false,'Market mainline does not share the automatic research dialog');
assert.equal(top.mainlineTarget.value,null,'A fresh entry starts from all-market candidates');
const savedTarget={kind:'industry',code:'SW801150',name:'申万主线',snapshotFingerprint:'a'.repeat(64)};
top.openMainline(savedTarget);
assert.deepEqual(JSON.parse(JSON.stringify(top.mainlineTarget.value)),savedTarget,'Sector and stored snapshot identity is retained');
top.navigate('research');
assert.equal(top.showMainline.value,false,'Research entry closes manual mainline view');
assert.equal(top.showResearch.value,true);
const props = Vue.reactive({ show: false, initialSection: 'ai' });
const modal = load('src/components/settings/SettingsDialog.vue', props, (name, value) => { if (name === 'update:show') { props.show = value; top.showSettings.value = value; } });
const dialog = modal.state;
assert.equal(dialog.activeSection.value, 'ai', 'An explicit initial section overrides the remembered section');
top.settingsDialog.value = modal.exposed;
function syncProps() { props.initialSection = top.settingsSection.value; props.show = top.showSettings.value; }
async function dispatch(destination) {
  pending = destination;
  handlers.get('quick-navigation')({ payload: null });
  for (let i = 0; i < 4; i++) await Vue.nextTick();
  syncProps(); await Vue.nextTick();
}
await dispatch('settings:alerts');
assert.equal(top.showResearch.value, false);
assert.equal(dialog.activeSection.value, 'alerts');
assert.equal(props.show, true);
dialog.agentPathDraft.value = 'unsaved-claude.exe';
await dispatch('settings:market');
assert.equal(dialog.activeSection.value, 'market');
assert.equal(dialog.agentPathDraft.value, 'unsaved-claude.exe', 'Changing settings categories must retain drafts');
await dispatch('settings:market');
assert.equal(dialog.agentPathDraft.value, 'unsaved-claude.exe', 'A repeated destination must not recreate settings');
dialog.savingKeys.value = new Set(['agent-path']);
await dispatch('simulation');
assert.equal(top.showSimulation.value, false, 'Saving settings blocks a shortcut close');
assert.equal(top.showSettings.value, true);
assert.match(dialog.actionError.value, /保存/);
await dispatch('mainline');
assert.equal(top.showMainline.value,false,'Saving settings also blocks market mainline navigation');
dialog.savingKeys.value = new Set();
await dispatch('simulation');
assert.equal(top.showSimulation.value, false, 'Declining draft discard blocks navigation');
assert.equal(top.showSettings.value, true);
assert.equal(confirmations, 1);
confirmDiscard = true;
await dispatch('simulation');
assert.equal(top.showSettings.value, false);
assert.equal(top.showSimulation.value, true);
await dispatch('settings:market');
assert.equal(top.showSimulation.value, false);
assert.equal(dialog.activeSection.value, 'market');
assert.equal(dialog.agentPathDraft.value, '', 'Reopening settings reloads saved values');
await dispatch('mainline');
assert.equal(top.showMainline.value,true,'Pending navigation can open mainline when the backend registers the destination');
assert.equal(top.showSettings.value,false);
assert.equal(top.mainlineTarget.value,null,'A repeated mainline destination clears the prior sector target');
await dispatch('settings:market');
props.initialSection = 'alerts'; await Vue.nextTick();
assert.equal(dialog.activeSection.value, 'alerts');
props.show = false; await Vue.nextTick();
props.initialSection = 'market'; props.show = true; await Vue.nextTick();
assert.equal(dialog.activeSection.value, 'market', 'Opening again with an explicit section routes correctly');
assert.deepEqual(errors, []);
for (const stop of disposers) stop();
console.log('Quick navigation check passed: subscribe-before-consume, initial and repeated routes, retained drafts, save/discard guards, close-on-new-destination, explicit reopen section; independent mainline, retained snapshot identity, settings guards, close-on-new-route.');
