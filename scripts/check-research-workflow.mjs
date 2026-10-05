import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { parse, compileScript, compileTemplate } from '@vue/compiler-sfc';
import ts from 'typescript';
import { uiHarness } from './ui-check-harness.mjs';

// Actual Vue setup and rendering; IPC stays in memory, never in user data.
const root = fileURLToPath(new URL('..', import.meta.url));
const read = file => fs.readFileSync(path.join(root, file), 'utf8');
const envelope = JSON.parse(read('src-tauri/research-evidence.json'));
assert.equal(createHash('sha256').update(envelope.content).digest('hex'), envelope.content_sha256);
const evidence = { ...JSON.parse(envelope.content), content_sha256: envelope.content_sha256, integrity_note: '原始证据正文指纹已核对，导入不建账户或订单' };
const external = JSON.parse(read('src-tauri/external-strategy-evidence.json'));
assert.equal(external.production_admission, false); assert.equal(external.new_usable_models, 0);
assert.ok(external.studies.every(s => s.cash_accounts > 0 && s.comparisons.length));
const financial = JSON.parse(read('research/ashare-fundamental-2026-10-01/research-summary-with-neighbors.json'));
const studies = [...external.studies, { title: '公告时点财务增量37', status: '执行稳健性未通过', message: '同覆盖与执行邻域实测，保留探索。', limitations: ['完整历史修订链未认证'], metrics: Object.entries(financial.models).map(([name,row]) => ({ name, net_return_pct: row.periods.test.net_return_pct, max_drawdown_pct: row.periods.test.max_drawdown_pct, cycles: row.periods.test.completed_cycles })) }];
const calls = [], jobs = [], picks = []; let runs = [], experiments = [], failLoad = false, nextId = 1;
const config = { research_root: 'E:/research', python: 'python', snapshot: 'E:/snapshot', index: 'E:/csi.ndjson' };
const today = new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Shanghai' });
const invoke = async (command, args = {}) => {
  calls.push({ command, args });
  if (command === 'research_dashboard') { if (failLoad) throw Error('isolated read failure'); return { config: { auto_research: false }, experiments, local_ready: true, busy: false }; }
  if (command === 'research_model_runs') return { runs, tasks: [], config, last_error: null };
  if (command === 'research_job_start') { const job = { id: nextId++, kind: args.request.kind, state: 'running', results: [] }; jobs.push(job); return job; }
  if (command === 'research_model_observe') { runs.find(r => r.id === args.id).enabled = args.enabled; return; }
  if (command === 'research_model_config') { Object.assign(config,args.config); return config; }
  if (command === 'research_job_list') return jobs;
  if (command === 'research_job_cancel' || command === 'research_job_resume') { jobs.find(j => j.id === args.id).state = command.endsWith('cancel') ? 'cancelled' : 'running'; return; }
  if (command === 'get_local_history_status') return { state: 'connected', message: '样本读取通过', end_date: today, sample_count: 100 };
  if (command === 'run_stockdb_update') return '隔离更新完成';
  if (command === 'research_action') { assert.equal(args.action,'pause'); experiments.find(e => e.id === args.experimentId).state = 'paused'; return; }
  throw Error('Unexpected backend command: ' + command);
};
const settings = { localHistoryEnginePath: 'stockdb.exe', localHistoryEnabled: true, localHistoryUrl: 'http://127.0.0.1:7899', settings: {}, async fetchStockDbStatus() {}, async runStockDbUpdate() { return invoke('run_stockdb_update'); } };
const vue = { defineComponent: x => x, ref: value => ({ value }), reactive: x => x, computed: fn => ({ get value() { return fn(); } }), watch() {}, onMounted() {}, onBeforeUnmount() {}, nextTick: async () => {} };
function evaluateModule(source, requireStub) {
  const module = { exports: {} };
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  vm.runInNewContext(js, { require: requireStub, exports: module.exports, module, setInterval, clearInterval, Date, console }); return module.exports;
}
const research = evaluateModule(read('src/types/research.ts'), () => ({}));
const pathDisplay = evaluateModule(read('src/utils/pathDisplay.ts'), () => ({}));
function setup(file, props) {
  const { descriptor, errors } = parse(read(file), { filename: file }); assert.deepEqual(errors, []);
  assert.deepEqual(compileTemplate({ source: descriptor.template.content, filename: file, id: 'research-flow-check' }).errors, []);
  const module = evaluateModule(compileScript(descriptor, { id: 'research-flow-check' }).content, id => {
    if (id === 'vue') return vue;
    if (id === '@tauri-apps/api/core') return { invoke };
    if (id === '@tauri-apps/plugin-dialog') return { open: async () => picks.shift() ?? null };
    if (id === '@/stores/settings') return { useSettingsStore: () => settings };
    if (id === '@/types/research') return research;
    if (id === '@/utils/pathDisplay') return pathDisplay;
    return {};
  });
  return module.default.setup(props, { expose() {}, emit() {} });
}
const state = setup('src/components/research/ResearchCenter.vue', { show: false });
assert.equal(state.tab.value,'tracking','Simple mode opens on automatic trading before any dashboard response');
await state.load(true); assert.equal(state.tab.value,'tracking');
state.centerMode.value = 'advanced'; await state.load(true); assert.equal(state.tab.value,'guide');
assert.deepEqual(Array.from(state.primaryTabs.value, t => t.label), ['数据准备与更新','冻结模型回放','研究证据','提醒与记录']);
assert.equal(state.archiveOpen.value,false); assert.equal(state.counts.value.all,0);
await state.refreshSetup(); assert.equal(state.historyReady.value,true);
state.updateHistory(); await new Promise(resolve => setImmediate(resolve)); assert.equal(calls.filter(c => c.command === 'run_stockdb_update').length,1);
const pickedPython = String.fromCharCode(92,92,63,92) + ['E:', 'checked', 'python.exe'].join(String.fromCharCode(92));
picks.push(pickedPython); await state.chooseModelPath('python',false); await state.saveModelPaths();
assert.equal(calls.find(c => c.command === 'research_model_config').args.config.python,pickedPython,'Display formatting must not change filesystem/IPC paths');
assert.equal(state.displayPath(pickedPython),pickedPython.slice(4));
assert.deepEqual(Array.from(state.modelOptions,m => m.id), Array.from(research.MODEL_CATALOG,m => m.id));
await state.runModel('replay'); assert.equal(state.tab.value,'records'); assert.equal(state.jobsExpanded.value,true); assert.equal(runs.length,0,'Starting does not invent a completed ledger');
const first = jobs[0]; runs = [{ id: first.id, model_id: 'breadth22_h20', comparison: 'baseline', mode: 'replay', enabled: false }];
await state.completedJob({ ...first, state: 'complete', results: [{ run_id: first.id }] });
assert.equal(state.modelSelectedId.value,first.id); assert.equal(state.modelSelected.value.mode,'replay');
state.centerMode.value='simple'; await state.openModelRun(first.id); assert.equal(state.centerMode.value,'advanced'); assert.equal(state.tab.value,'model-lab');
await state.openSetting('alerts'); assert.equal(state.tab.value,'records');
await state.compareModel(); assert.deepEqual(Array.from(calls.filter(c => c.command === 'research_job_start').at(-1).args.request.comparisons),['baseline','holding15','cost_double']);
await state.runModel('forward'); const forward = { id: 99, model_id: 'index26_h20', comparison: 'baseline', mode: 'forward', enabled: false }; runs.unshift(forward);
await state.observeModel(forward); assert.equal(forward.enabled,true);
await state.runModel('forward',forward); const request = calls.filter(c => c.command === 'research_job_start').at(-1).args.request;
assert.equal(request.continuation,99); assert.deepEqual(Array.from(request.models),['index26_h20']); assert.equal(request.enable_observation,undefined);
experiments = [{ id: 9, state: 'observing' }]; await state.load(); state.selectedId.value = 9; assert.equal(state.counts.value.all,1);
await state.action('pause'); // action delegates to act; wait for its awaited read.
await new Promise(resolve => setImmediate(resolve)); assert.equal(experiments[0].state,'paused');
const previous = state.dashboard.value; failLoad = true; await state.load(); assert.equal(state.dashboard.value,previous); assert.equal(state.modelRuns.value.length,2); assert.equal(state.busy.value,''); failLoad = false;
const panel = setup('src/components/research/ResearchJobPanel.vue',{}); await panel.refresh(); await panel.action(first,'cancel'); assert.equal(first.state,'cancelled'); await panel.action(first,'resume'); assert.equal(first.state,'running');
assert.equal(calls.some(c => /agent|claude|simulation|follow_create/.test(c.command)),false,'Deep research actions do not call AI or user-account trading');

const mock = `
const evidence = ${JSON.stringify(evidence)}, studies = ${JSON.stringify(studies)};
const copy = value => JSON.parse(JSON.stringify(value));
window.__workflow = { calls: [], jobs: [], runs: [], evidence, studies, picks: [], settings: [], holdRead: localStorage.getItem('workflow-initial-delay')==='1', holdEvidence: false };
const state = window.__workflow;
const config = {auto_research:false,observation_days:28,min_samples:20,max_drawdown_bps:1500,min_return_bps:0,initial_cash:'1000000000',max_active:3,stock_count:5,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:1,slippage_bps:5};
const originalPaths = ['stockdb.exe','数据更新.exe'].map(name=>String.fromCharCode(92,92,63,92)+['E:','隔离',name].join(String.fromCharCode(92)));
const stockdb = {enabled:true,enginePath:originalPaths[0],updaterPath:originalPaths[1],engineDir:'E:/隔离',url:'http://127.0.0.1:7899',state:'running_external',busy:false,updaterAvailable:true,message:'隔离：外部启动的同一本地服务',candidates:[]};
export async function invoke(command,args={}) {
 state.calls.push({command,args:copy(args)});
 if(command==='research_dashboard') {const value=copy({config,experiments:[],local_ready:true,busy:false});if(state.holdRead)return new Promise(resolve=>state.readResolve=()=>resolve(value));return value;}
 if(command==='research_model_runs')return copy({runs:state.runs,tasks:[],config:{research_root:'E:/research',python:'python',snapshot:'E:/snapshot',index:'E:/csi.ndjson'},last_error:null});
 if(command==='get_stockdb_status'||command==='scan_stockdb')return copy(stockdb);
 if(command==='get_local_history_status')return {state:'connected',message:'隔离历史样本读取通过',end_date:new Date().toLocaleDateString('sv-SE',{timeZone:'Asia/Shanghai'}),sample_count:100};
 if(command==='run_stockdb_update')return '隔离手动更新完成；没有操作真实服务';
 if(command==='get_research_evidence'){if(state.holdEvidence)return new Promise(resolve=>state.evidenceResolve=()=>resolve(copy(evidence)));return copy(evidence);}
 if(command==='import_research_evidence')return copy(evidence);
 if(command==='research_extension_report')return copy({studies});
 if(command==='research_job_list')return copy(state.jobs);
 if(command==='research_job_start') {const job={id:state.jobs.length+1,kind:args.request.kind,request:copy(args.request),state:'running',phase:'checking',message:'隔离任务正在核验输入',completed:0,total:args.request.kind==='explore'?5:args.request.comparisons.length,results:[]};state.jobs.unshift(job);return copy(job);}
 if(command==='research_job_cancel'||command==='research_job_resume'){state.jobs.find(j=>j.id===args.id).state=command.endsWith('cancel')?'cancelled':'running';return;}
 if(command==='research_market_data_status')return{state:'no_active_accounts',enabled_accounts:0,missing_accounts:0,accounts:[],last_refresh:{},message:'隔离状态'};
 if(command==='research_follow_accounts')return[];
 if(command==='research_auto_status')return{enabled:true,effective_enabled:true,state:'closed',message:'隔离：等待下一个A股交易日；不操作账户或模型',last_completed_day:null,models:[]};
 if(command==='model_condition_watches')return{presets:[],watches:[],limit_per_tick:20};
 if(command==='get_model_candidates')return{schema:'research-model-candidates-v1',production_admission:false,current_as_of:'2026-09-30',as_of:null,fresh:false,groups:[],message:'隔离手动提醒'};
 if(command==='save_research_config'){Object.assign(config,args.config);return;}
 if(command==='set_setting')return;
 throw Error('Unexpected UI backend command: '+command);
}
export async function open(){return state.picks.shift()??null;}
export async function emit(){} export async function listen(){return()=>{};}
export async function enable(){} export async function disable(){} export async function isEnabled(){return false;}
`;
const entry = `
import {createApp,h,ref} from 'vue';import {createPinia} from 'pinia';
import Center from '/src/components/research/ResearchCenter.vue';import {useSettingsStore} from '/src/stores/settings.ts';
import '/src/assets/styles/variables.css';import '/src/assets/workspace.css';
localStorage.setItem('research-center-mode-v1',localStorage.getItem('workflow-initial-delay')==='1'?'simple':'advanced');
if(localStorage.getItem('workflow-initial-delay')==='1'){window.__unexpectedDataPage=false;new MutationObserver(()=>{if(document.querySelector('.data-setup'))window.__unexpectedDataPage=true;}).observe(document.body,{childList:true,subtree:true});}
document.documentElement.dataset.theme='light';document.documentElement.dataset.style='modern';
const pinia=createPinia(), store=useSettingsStore(pinia);
store.settings={local_history_enabled:'1',local_history_engine_path:'E:/隔离/stockdb.exe',local_history_updater_path:'E:/隔离/数据更新.exe',local_history_url:'http://127.0.0.1:7899',research_notifications_enabled:'1'};
createApp({setup(){const center=ref(null);window.__center=center;return()=>h(Center,{show:true,ref:center,onOpenSettings:s=>window.__workflow.settings.push(s)});}}).use(pinia).mount('#app');
`;
const t = await uiHarness({ name: 'research-workflow', entry, mock });
try {
 await t.call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
 await t.wait('document.querySelector(".data-setup") && window.__center.value?.$.setupState.historyReady');
 assert.deepEqual(await t.evaluate('[...document.querySelectorAll(".research-nav button")].map(b=>b.innerText)'),['数据准备与更新','冻结模型回放','研究证据','提醒与记录']);
 assert.equal(await t.evaluate('document.querySelector(".research-archive").open'),false);
 const visibleShared = '[...document.querySelectorAll(".research-notifications,.research-background-jobs,.research-archive")].filter(e=>e.getClientRects().length>0 && getComputedStyle(e).display!=="none").length';
 assert.equal(await t.evaluate(visibleShared),0,'Shared reminders and records stay off operation pages');
 assert.equal(await t.evaluate('document.querySelectorAll(".data-setup .research-howto ol li").length'),3,'Data setup shows a complete sequence instead of unexplained buttons');
 assert.match(await t.evaluate('document.querySelector(".data-setup .research-howto").innerText'),/自动查找[\s\S]*更新历史数据[\s\S]*保存四项路径[\s\S]*结果在哪里/);
 await t.evaluate('document.querySelector(".research-paths").open=true');
 assert.match(await t.evaluate('document.querySelector(".research-paths").innerText'),/项目根目录[\s\S]*python.exe[\s\S]*行情快照目录[\s\S]*沪深300指数文件/);
 await t.screenshot('data-operation-guide');
 await t.evaluate('document.querySelector(".research-paths").open=false');
 assert.equal(await t.evaluate('document.querySelector(".data-setup .path").innerText.includes(String.fromCharCode(92,92,63,92))'),false,'Extended path prefix is display-only');
 assert.equal(await t.evaluate('document.querySelectorAll(".research-notifications [role=switch]").length'),1,'Only one research reminder switch is mounted');
 assert.equal(await t.evaluate('[...document.querySelectorAll("button")].find(b=>b.innerText==="更新历史数据").disabled'),false,'External StockDB may update');
 await t.click('更新历史数据'); await t.wait('window.__workflow.calls.some(c=>c.command==="run_stockdb_update") && !window.__center.value.$.setupState.busy');
 await t.click('冻结模型回放'); await t.wait('document.querySelector(".mode-grid")');
 assert.equal(await t.evaluate(visibleShared),0);
 assert.equal(await t.evaluate('document.querySelectorAll(".reading .mode-card").length'),5);
 assert.equal(await t.evaluate('document.querySelectorAll(".reading .research-howto ol li").length'),3);
 assert.match(await t.evaluate('document.querySelector(".reading .research-howto").innerText'),/选模型本身不会开始计算[\s\S]*查看账本[\s\S]*已有账本直接点开/);
 const jobsBeforeHelp=await t.evaluate('window.__workflow.jobs.length');
 await t.evaluate('[...document.querySelectorAll(".reading>details")].find(e=>e.querySelector("summary")?.innerText.startsWith("期限、费用")).open=true');
 assert.match(await t.evaluate('document.querySelector(".reading").innerText'),/20日不是必须持满[\s\S]*不重新训练[\s\S]*导入可信模型账本[\s\S]*固定比较这三项/);
 assert.equal(await t.evaluate('window.__workflow.jobs.length'),jobsBeforeHelp,'Reading explanations never starts a research job');
 await t.screenshot('model-operation-guide');
 assert.equal(await t.evaluate('!!document.querySelector(".reading .condition-watches")'),false);
 await t.click('运行真实多年回放'); await t.wait('window.__workflow.jobs.length===1 && document.querySelector("[data-job-state=running]")');
 assert.deepEqual(await t.evaluate('window.__workflow.jobs[0].request'),{kind:'replay',models:['breadth22_h20'],comparisons:['baseline'],continuation:null});
 assert.equal(await t.evaluate('window.__center.value.$.setupState.tab'), 'records', 'Starting research opens the single progress page');
 assert.equal(await t.evaluate(visibleShared),3);
 await t.click('取消'); await t.wait('document.querySelector("[data-job-state=cancelled]")');
 await t.click('继续未完成项'); await t.wait('document.querySelector("[data-job-state=running]")');
 await t.screenshot('frozen-replay');
 await t.click('研究证据'); await t.wait('document.querySelector(".evidence table")');
 assert.equal(await t.evaluate(visibleShared),0,'Evidence page has no repeated reminders, jobs or archives');
 assert.deepEqual(await t.evaluate('[...document.querySelectorAll(".evidence nav button")].map(b=>b.innerText)'),['策略比较','数据覆盖','实验与压力']);
 const geometry = 'JSON.stringify([...document.querySelectorAll(".research-nav,.evidence .head,.evidence .warning,.evidence .table-scroll")].map(e=>{const r=e.getBoundingClientRect();return [r.x,r.y,r.width,r.height]}))';
 const before = await t.evaluate(geometry);
 await t.evaluate('window.__workflow.holdRead=true;void (window.__pendingRead=window.__center.value.$.setupState.load())');
 await t.wait('typeof window.__workflow.readResolve==="function"'); assert.equal(await t.evaluate(geometry),before);
 await t.evaluate('window.__workflow.holdRead=false;window.__workflow.readResolve();window.__pendingRead'); assert.equal(await t.evaluate(geometry),before,'Quiet refresh preserves geometry and last response');
 await t.evaluate('window.__workflow.holdEvidence=true'); await t.click('刷新证据'); await t.wait('typeof window.__workflow.evidenceResolve==="function"'); assert.equal(await t.evaluate(geometry),before);
 await t.evaluate('window.__workflow.holdEvidence=false;window.__workflow.evidenceResolve()'); await t.wait('!document.querySelector(".evidence .head button").disabled'); assert.equal(await t.evaluate(geometry),before);
 await t.evaluate('window.__workflow.picks.push("E:/隔离/证据.json")'); await t.click('导入研究证据包'); await t.wait('window.__workflow.calls.some(c=>c.command==="import_research_evidence")');
 await t.click('数据覆盖'); await t.wait('document.querySelector(".evidence .hash")'); assert.match(await t.evaluate('document.querySelector(".evidence").innerText'),/StockDB|SHA256/);
 // Complete the isolated task while its page is hidden; the mounted observer must still deliver it.
 await t.evaluate(`window.__workflow.runs=[{id:700,model_id:"breadth22_h20",model_name:"隔离模型账本",mode:"replay",comparison:"baseline",as_of:"2026-09-30",holding_days:20,enabled:false,signal_watch:[],limitations:[],ledger:{metrics:{net_return_pct:0,max_drawdown_pct:0,completed_holding_cycles:1,win_rate_pct:100,open_positions:0,max_overdue_sessions:0},curve:[],orders:[],unclosed:[]}}];Object.assign(window.__workflow.jobs[0],{state:"complete",completed:1,results:[{key:"checked",model_id:"breadth22_h20",comparison:"baseline",run_id:700}]});`);
 await t.wait('window.__center.value.$.setupState.jobsBusy===false && document.querySelector("[data-job-state=complete]")');
 assert.equal(await t.evaluate(visibleShared),0);
 await t.click('提醒与记录'); await t.wait('document.querySelector(".research-records-intro")');
 assert.equal(await t.evaluate(visibleShared),3);
 assert.equal(await t.evaluate('document.querySelector(".research-background-jobs").open'),true,'Expanded task progress survives page navigation');
 await t.evaluate('document.querySelector(".research-jobs [data-job-state=complete] details").open=true');
 await t.click('查看账本'); await t.wait('document.querySelector(".model-ledger") || document.querySelector(".reading")?.innerText.includes("隔离模型账本")');
 assert.equal(await t.evaluate('window.__center.value.$.setupState.modelSelectedId'),700);
 assert.equal(await t.evaluate(visibleShared),0);
 await t.click('提醒与记录');
 assert.equal(await t.evaluate('document.querySelectorAll(".research-notifications [role=switch]").length'),1);
 await t.click('暂停提醒'); await t.wait('document.querySelector(".research-notifications [role=switch]").getAttribute("aria-checked")==="false"');
 await t.click('恢复提醒'); await t.wait('document.querySelector(".research-notifications [role=switch]").getAttribute("aria-checked")==="true"');
 await t.evaluate('document.querySelector(".research-archive").open=true'); await t.wait('document.querySelector(".research-archive").innerText.includes("扩展研究报告")');
 await t.evaluate('[...document.querySelectorAll(".research-archive details")].find(e=>e.querySelector("summary")?.innerText.startsWith("盘后 AI 研究")).open=true');
 assert.match(await t.evaluate('document.querySelector(".research-archive").innerText'),/日常自动模型买卖用不到[\s\S]*先检查已有任务书[\s\S]*才请AI提出/);
 await t.evaluate('[...document.querySelectorAll(".research-archive details")].find(e=>e.querySelector("summary")?.innerText==="扩展研究报告").open=true'); await t.wait('document.querySelector(".extension-study .study-meta")');
 assert.equal(await t.evaluate('document.body.innerText.includes("运行财务37真实历史回放")'),false);
 assert.match(await t.evaluate('document.querySelector(".extension-study").innerText'),/TSP公开技术规则对照/);
 assert.match(await t.evaluate('document.querySelector(".extension-study").innerText'),/本页查看步骤|启用、查看与关停步骤|使用步骤/);
 await t.evaluate('[...document.querySelectorAll(".research-archive details")].find(e=>e.querySelector("summary")?.innerText==="技术组合实验记录").open=true');
 await t.wait('document.querySelector(".nested-research")');
 await t.evaluate('document.querySelector(".nested-guide").open=true');
 assert.match(await t.evaluate('document.querySelector(".nested-research").innerText'),/运行五年技术组合研究[\s\S]*研究记录[\s\S]*结果/);
 assert.equal(await t.evaluate('window.__workflow.jobs.length'),jobsBeforeHelp+1,'Opening optional records only reads saved jobs');
 await t.screenshot('optional-operation-guide');
 await t.evaluate('[...document.querySelectorAll(".research-archive details")].find(e=>e.querySelector("summary")?.innerText==="技术组合实验记录").open=false');
 assert.equal(await t.evaluate('document.querySelector(".extension-study").innerText.includes("分钟走势确认")'),false);
 assert.equal(await t.evaluate('window.__workflow.calls.some(c=>c.command==="research_intraday_status")'),false,'Opening reports never starts retired minute checks');
 assert.equal(await t.evaluate('window.__workflow.calls.filter(c=>c.command==="research_extension_report").length'),1,'Static reports are read once without background polling');
 await t.call('Emulation.setDeviceMetricsOverride',{width:540,height:900,deviceScaleFactor:1,mobile:false});
 await t.screenshot('evidence-and-optional-records');
 assert.equal(await t.evaluate('(()=>{const e=document.querySelector(".research-scroll");return e.scrollWidth<=e.clientWidth+1})()'),true,'Narrow research page does not overflow');
 assert.equal(await t.evaluate('[...document.querySelectorAll(".research-center small,.health,.evidence p,.extension-study small")].filter(e=>e.getClientRects().length).every(e=>parseFloat(getComputedStyle(e).fontSize)>=12)'),true);
 await t.click('简化版 · 日常使用'); await t.wait('document.querySelector(".follow-trading") || window.__center.value.$.setupState.tab==="tracking"');
 assert.equal(await t.evaluate(visibleShared),0);
 await t.click('手动条件提醒'); await t.wait('document.querySelector(".daily-research")');
 assert.equal(await t.evaluate(visibleShared),0);
 assert.equal(await t.evaluate('document.querySelectorAll(".research-jobs").length'),1,'Embedded screener shares one progress observer');
 assert.equal(await t.evaluate('document.querySelectorAll(".condition-manager").length'),1,'Manual reminder list appears once');
 const jobsBeforeManual=await t.evaluate('window.__workflow.jobs.length');
 await t.click('筛选符合条件的股票'); await t.wait('window.__center.value.$.setupState.tab==="records" && window.__workflow.jobs.length===2');
 assert.equal(await t.evaluate('window.__workflow.jobs.length'),jobsBeforeManual+1);
 assert.equal(await t.evaluate('window.__workflow.jobs[0].kind'),'scan');
 assert.equal(await t.evaluate('window.__workflow.calls.some(c=>/^(simulation|follow_create)/.test(c.command))'),false);
 await t.click('手动条件提醒'); await t.wait('document.querySelector(".research-model-screen")');
 await t.click('多年验证证据'); await t.wait('window.__center.value.$.setupState.centerMode==="advanced" && window.__center.value.$.setupState.tab==="evidence"');
 assert.equal(await t.evaluate('document.querySelectorAll(".evidence").length'),1);
 await t.click('简化版 · 日常使用');
 await t.click('提醒与记录');
 assert.equal(await t.evaluate('document.querySelectorAll(".research-notifications [role=switch]").length'),1);
 assert.equal(await t.evaluate(visibleShared),2);
 await t.evaluate('document.querySelector(".research-jobs [data-job-state=complete] details").open=true');
 await t.click('查看账本'); await t.wait('window.__center.value.$.setupState.centerMode==="advanced" && window.__center.value.$.setupState.tab==="model-lab"');
 assert.equal(await t.evaluate('window.__center.value.$.setupState.modelSelectedId'),700);
 assert.equal(await t.evaluate('window.__workflow.calls.some(c=>/^(simulation|follow_create|research_discover|open_research_claude)/.test(c.command))'),false);
 // Initial dashboard is deliberately unresolved: entry routing must work before its response.
 const delayedUrl=await t.evaluate('(()=>{localStorage.setItem("workflow-initial-delay","1");return location.origin+"/";})()');
 await t.call('Page.navigate',{url:delayedUrl});
 await t.wait('window.__center?.value && typeof window.__workflow?.readResolve==="function" && document.querySelector(".research-nav")');
 assert.equal(await t.evaluate('window.__center.value.$.setupState.tab'),'tracking');
 assert.equal(await t.evaluate('!!document.querySelector(".data-setup")'),false,'No data preparation page while initial dashboard is pending');
 assert.equal(await t.evaluate('document.querySelector(".research-nav button[aria-current=page]").innerText'),'自动模型交易');
 await t.screenshot('simple-opening-pending');
 await t.evaluate('window.__workflow.holdRead=false;window.__workflow.readResolve()');
 await t.wait('!!window.__center.value.$.setupState.dashboard');
 assert.equal(await t.evaluate('window.__center.value.$.setupState.tab'),'tracking');
 assert.equal(await t.evaluate('window.__unexpectedDataPage'),false,'Data preparation never flashes before or after initial data arrives');
 assert.equal(await t.evaluate('window.__workflow.calls.some(c=>/^(simulation|follow_create|research_discover|open_research_claude|research_job_start)/.test(c.command))'),false);
 assert.deepEqual(t.errors,[]); t.save({ passed: true, checks: ['actual Vue setup/actions','original evidence fingerprint','core operation pages and single reminders/records entry','manual external update','frozen replay request and task cancel/resume','quiet refresh geometry','evidence import and real sources','static reports without minute checks or polling','narrow layout and Chinese text minimum','path formatting preserves original IPC paths','visible setup/replay steps and research path definitions','execution choices and source-ledger operations explained without job creation','optional research instructions preserve read-only navigation','hidden task observer preserves completion and ledger navigation','single reminder switch in simple and advanced mode','embedded manual scan routes to shared jobs and one evidence view','simple initial page never flashes data setup with delayed dashboard'], limitation: 'Rendered UI uses isolated IPC fixtures; does not run native StockDB, research computation, AI or user trades.' });
 console.log('Research workflow passed: actual Vue setup, rendered actions, source evidence, quiet geometry. UI artifacts: '+t.output);
} catch (error) { console.error(t.errors); throw error; } finally { await t.close(); }
