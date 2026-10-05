import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { uiHarness } from './ui-check-harness.mjs';
const root = fileURLToPath(new URL('..', import.meta.url));
const rust = readFileSync(path.join(root,'src-tauri/src/commands/model_follow.rs'),'utf8');
const commandArgs = Object.fromEntries(['research_follow_setup','research_follow_update','research_follow_refresh','research_follow_order','research_follow_delete'].map(name => {
  const signature = rust.match(new RegExp('pub (?:async )?fn '+name+'\\s*\\(([\\s\\S]*?)\\)\\s*->'));
  assert.ok(signature, 'Missing Rust command '+name);
  return [name,[...signature[1].matchAll(/(?:^|,)\s*(\w+)\s*:/g)].map(match=>match[1]).filter(key=>!['db','manager'].includes(key)).map(key=>key.replace(/_([a-z])/g,(_,letter)=>letter.toUpperCase()))];
}));
assert.deepEqual(commandArgs.research_follow_update,['accountId','maxPositions','enabled']);
assert.deepEqual(commandArgs.research_follow_delete,['accountId']);
const allocation = JSON.parse(readFileSync(path.join(root,'src-tauri/model-follow-allocation.json'),'utf8'));
const timing = JSON.parse(readFileSync(path.join(root,'src-tauri/model-follow-timing.json'),'utf8'));
function installMock(catalog, evidence, timing, commandArgs) {
  const clone = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
  const legalRuns = catalog.map((model, i) => ({ id: 101 + i, model_id: model.id, model_name: model.name, mode: 'forward', comparison: 'baseline', holding_days: 20 }));
  const invalidRuns = [
    { ...legalRuns[0], id: 201, model_id: 'financial37_h20', model_name: '基本面来源（必须过滤）' },
    { ...legalRuns[0], id: 202, mode: 'replay', model_name: 'replay 来源（必须过滤）' },
    { ...legalRuns[0], id: 203, mode: 'staged', model_name: 'staged 来源（必须过滤）' },
    { ...legalRuns[0], id: 204, comparison: 'staged', model_name: 'staged 对照（必须过滤）' },
    { ...legalRuns[0], id: 205, holding_days: 10, model_name: '非20日来源（必须过滤）' },
    { ...legalRuns[0], id: 206, comparison: 'double_cost', model_name: '非baseline来源（必须过滤）' },
    { ...legalRuns[0], id: 207, model_id: 'unregistered_model', model_name: '未注册模型（必须过滤）' },
  ];
  const state = {
    calls: [], settings: {research_notifications_enabled:'1', alerts_enabled:'0'}, broadcasts: [], accounts: [], sourceRuns: [], legalRuns, invalidRuns, nextAccountId: 9001,
    failNext: null, deferNext: null, deferred: [],
  };
  const keys = (args, names) => {
    if (!args || JSON.stringify(Object.keys(args).sort()) !== JSON.stringify([...names].sort())) throw Error('Mock contract: unexpected argument keys');
  };
  const findAccount = id => {
    const account = state.accounts.find(view => view.account_id === id);
    if (!account) throw Error('Mock contract: unknown account');
    return account;
  };
  const configureAccount = account => {
    const model=evidence.models[account.model_id], cap=model?.caps[String(account.max_positions)];
    const admitted=account.model_id==='breadth22_rank20'&&account.max_positions===5&&cap?.admitted&&cap.selected_policy_id==='equal50';
    const per=admitted?10:8,total=admitted?50:account.max_positions*8;
    account.allocation_policy=admitted?'rank5_equal50':'baseline8';
    account.automatic_execution=true;
    account.execution={position_pct:per,total_entry_pct:total,holding_days:model?.recommendation.execution.holding_sessions??20,max_gap_pct:model?.recommendation.execution.max_gap_pct??4,buy_window:'09:30–09:35（北京时间）',fee_note:'隔离费用样例，实际费用以服务端成交记录为准。',allocation_note:'隔离账户：按本模型与数量的配置，单票 '+per+'%、总入场预算 '+total+'%。'};
    const timingModel=timing.models[account.model_id], timingConfig=timingModel?.active_configuration;
    const match=timingConfig?.max_positions===account.max_positions&&timingConfig?.allocation_policy===account.allocation_policy;
    account.execution.entry_policy=timingModel?.admitted&&match?'index_support':'baseline';
    account.execution.exit_policy='original';
    account.execution.entry_note=account.execution.entry_policy==='index_support'?'完成日沪深300近5日收益>0，处于60日均线上方或等于均线，个股复权收盘不低于10日均线才尝试买入。':'沿用原模型合格信号、自己的原分数排序及交易检查。';
    account.execution.sell_window='交易日连续竞价时段，14:57前；午休不成交';
    account.execution.intraday_note='日线模型盘后确认条件，盘中程序自动提交并等待新盘口撮合；不新增未回测的盘中止损或午后买入。';
    account.execution.exit_note='最多20个交易日，可因3ATR、ST/状态风险提前退出；不可成交时继续提示。';
    account.timing_research=timingModel?{...clone(timingModel),start:timing.start,end:timing.end,early_period:timing.early_period,later_period:timing.later_period,ledger_count:timing.ledger_count,selection_note:timing.selection_note,current_configuration_matches:!!match,active_policy:match?timingModel.active_policy:'original'}:undefined;
    account.allocation_research=cap?{start:evidence.start,end:evidence.end,early_period:evidence.early_period,later_period:evidence.later_period,decision:evidence.decision,selection_note:evidence.selection_note,ledger_count:evidence.ledger_count,annual_slice_count:evidence.annual_slice_count,...cap,recommendation:model.recommendation,slot_comparison:model.slot_comparison}:undefined;
  };
  const makeOrder = (id, symbol, name, quantity, limit) => ({
    id, symbol, name, side: 'buy', quantity, limit_price_cny: limit, estimated_fee_cny: 5,
    status: 'pending', automatic_submission: true, reason: '隔离样例：原模型阈值与排名满足，单票约8%且按整手计划。',
    signal_date: '20260918', created_at: '2026-09-21T01:30:01Z', confirmed_at: '2026-09-21T01:30:01Z',
    filled_at: null, filled_price_cny: null, fee_cny: null, quote_at: '2026-09-21T01:30:01Z',
    valid_until: '2026-09-21T01:35:00Z', reject_reason: null,
  });
  state.stageBuys = id => {
    const account = findAccount(id);
    account.orders = [makeOrder(9101, 'sh600101', '模拟样例甲', 800, 10.05), makeOrder(9102, 'sz000002', '模拟样例乙', 400, 19.80)];
    account.state = 'listening';
    account.message = '隔离测试操作单；这些股票、价格与评分仅验证界面，不代表行情实测。';
  };
  state.reportFill = (id, orderId) => {
    const account = findAccount(id), order = account.orders.find(row => row.id === orderId);
    if (!order || order.status !== 'pending') throw Error('Mock contract: only pending can receive fill');
    Object.assign(order, { status: 'filled', filled_at: '2026-09-21T01:31:16Z', filled_price_cny: 10, fee_cny: 5, quote_at: '2026-09-21T01:31:16Z' });
    account.cash_cny -= order.quantity * 10 + 5;
    account.positions.push({ symbol: order.symbol, name: order.name, quantity: order.quantity, available_quantity: 0, cost_cny: 10 + 5 / order.quantity, mark_cny: 10, mark_at: '2026-09-21T01:31:16Z', entry_date: '20260921', holding_sessions: 0, exit_reason: null });
    account.equity_cny = account.cash_cny + order.quantity * 10;
    account.net_return_pct = (account.equity_cny / account.initial_cash_cny - 1) * 100;
    account.performance.filled_buys += 1;
    account.performance.confirmation_delay_seconds = 0;
    account.state = 'active';
  };
  state.stageSell = id => {
    const account = findAccount(id), position = account.positions[0];
    position.available_quantity = position.quantity;
    position.holding_sessions = 20;
    position.exit_reason = '隔离样例：达到原模型20个交易日持有期。';
    const order = makeOrder(9103, position.symbol, position.name, position.quantity, 9.99);
    Object.assign(order, { side: 'sell', reason: position.exit_reason, created_at: '2026-10-21T06:30:00Z', confirmed_at: '2026-10-21T06:30:00Z', signal_date: '20261020', quote_at: '2026-10-21T06:30:00Z', valid_until: '2026-10-21T06:57:00Z' });
    account.orders.push(order);
  };
  state.invoke = async (command, args) => {
    state.calls.push({ command, ...(args === undefined ? {} : { args: clone(args) }) });
    if (state.deferNext === command) {
      state.deferNext = null;
      await new Promise(resolve => state.deferred.push(resolve));
    }
    if (state.failNext?.command === command) {
      const failure = state.failNext; state.failNext = null; throw Error(failure.message);
    }
    if (command === 'get_settings') return clone(state.settings);
    if (command === 'get_stockdb_status') return {enabled:false,state:'disabled',enginePath:null};
    if (command === 'list_datasources') return [['mock','隔离行情']];
    if (command === 'get_portable_mode') return false;
    if (command === 'set_setting') { keys(args,['key','value']);state.settings[args.key]=args.value;return; }
    if (command === 'simulation_adjust_capital') {
      keys(args,['input']);keys(args.input,['accountId','initialCash']);
      if(typeof args.input.initialCash!=='number')throw Error('Mock contract: yuan number required by Rust capital IPC');
      const account=findAccount(args.input.accountId), cash=args.input.initialCash;
      if(!Number.isFinite(cash)||cash<1000||cash>100000000||Math.round(cash*100)/100!==cash)throw Error('Mock contract: invalid capital');
      const delta=cash-account.initial_cash_cny;
      if(account.cash_cny+delta<0)throw Error('可用现金不足，无法减少资金');
      account.initial_cash_cny=cash;account.cash_cny+=delta;
      if(account.equity_cny!==null)account.equity_cny+=delta;
      account.net_return_pct=account.equity_cny===null?null:(account.equity_cny/cash-1)*100;
      return {id:account.account_id,initial_cash:String(Math.round(cash*10000)),current_cash:String(Math.round(account.cash_cny*10000)),managed_by:'model_follow'};
    }
    if (command === 'research_follow_accounts') {
      if (args != null) throw Error('Mock contract: accounts takes no args');
      return clone(state.accounts);
    }
    if (command === 'research_follow_setup') {
      keys(args,commandArgs[command]);keys(args.input,['model_id','initial_cash_cny','max_positions']);
      const input=args.input;const model=catalog.find(row=>row.id===input.model_id);if(!model)throw Error('Mock contract: only registered frozen models');
      const existing=state.accounts.filter(row=>row.model_id===input.model_id).sort((a,b)=>b.account_id-a.account_id)[0];
      if(existing){existing.enabled=true;existing.state='waiting_session';return {...clone(existing),setup_reused:true};}
      let source=state.sourceRuns.find(row=>row.model_id===model.id&&row.mode==='forward'&&row.comparison==='baseline'&&row.holding_days===20);
      if(!source){source={id:50000+catalog.indexOf(model),model_id:model.id,model_name:model.name,mode:'forward',comparison:'baseline',holding_days:20};state.sourceRuns.push(source);}
      const created=await state.invoke('research_follow_start',{input:{source_run_id:source.id,initial_cash_cny:input.initial_cash_cny,max_positions:input.max_positions}});
      return {...created,setup_reused:false};
    }
    if (command === 'research_follow_start') {
      keys(args, ['input']); keys(args.input, ['source_run_id', 'initial_cash_cny', 'max_positions']);
      const input = args.input;
      const source = state.sourceRuns.find(row => row.id === input.source_run_id && catalog.some(model => model.id === row.model_id) && row.mode === 'forward' && row.comparison === 'baseline' && row.holding_days === 20);
      if (!source) throw Error('Mock contract: illegal source');
      if (!Number.isFinite(input.initial_cash_cny) || input.initial_cash_cny < 1000 || input.initial_cash_cny > 100000000 || input.initial_cash_cny !== Math.round(input.initial_cash_cny * 100) / 100) throw Error('Mock contract: invalid initial cash');
      if (!Number.isInteger(input.max_positions) || input.max_positions < 1 || input.max_positions > 10) throw Error('Mock contract: invalid maximum');
      const view = {
        account_id: state.nextAccountId++, source_run_id: source.id, model_id: source.model_id, model_name: source.model_name,
        as_of: '20260918', max_positions: input.max_positions, enabled: true,
        initial_cash_cny: input.initial_cash_cny, cash_cny: input.initial_cash_cny, equity_cny: input.initial_cash_cny,
        net_return_pct: 0, state: 'waiting_session', message: '隔离样例：从空仓等待原模型计划。', source_sha256: 'a'.repeat(64),
        positions: [], orders: [], candidates: [
          { symbol: 'sh600103', score: 0.025, rank: 3, reference_quantity: 300, reason: '隔离样例排名3' },
          { symbol: 'sh600101', score: 0.052, rank: 1, reference_quantity: 800, reason: '隔离样例排名1' },
          { symbol: 'sz000002', score: 0.041, rank: 2, reference_quantity: 400, reason: '隔离样例排名2' },
        ],
        execution: { position_pct: 8, holding_days: 20, max_gap_pct: 4, buy_window: '09:30–09:35（北京时间）', fee_note: '隔离费用样例，实际费用以服务端成交记录为准。' },
        comparison_note: '少买后不能沿用原研究10只组合收益；以本账户实际模拟成交为准。',
        performance: { filled_buys: 0, filled_sells: 0, cancelled: 0, expired: 0, confirmation_delay_seconds: null },
      };
      configureAccount(view); state.accounts.push(view); return clone(view);
    }
    if (command === 'research_follow_update') {
      keys(args, commandArgs[command]);
      const account = findAccount(args.accountId);
      if (args.maxPositions !== null) {
        const maximum = args.maxPositions;
        const occupied = new Set([...account.positions.map(row => row.symbol), ...account.orders.filter(row => row.side === 'buy' && ['awaiting_confirmation', 'pending'].includes(row.status)).map(row => row.symbol)]).size;
        if (!Number.isInteger(maximum) || maximum < 1 || maximum > 10 || maximum < occupied) throw Error('最多持股数不能小于当前持仓及待买名额');
        const previousPolicy=account.allocation_policy,previousExecution=account.execution.entry_policy;
        account.max_positions = maximum; configureAccount(account);
        if(previousPolicy!==account.allocation_policy||previousExecution!==account.execution.entry_policy) for(const order of account.orders.filter(row=>['pending','awaiting_confirmation'].includes(row.status))){order.status='cancelled';order.reject_reason='隔离样例：仓位配置更新';account.performance.cancelled+=1;}
      }
      if (args.enabled !== null) {
        if (typeof args.enabled !== 'boolean') throw Error('Mock contract: enabled must be boolean');
        account.enabled = args.enabled; account.effective_enabled = args.enabled && state.auto?.effective_enabled !== false; account.state = args.enabled ? 'waiting_session' : 'paused';
        if (!args.enabled) for (const order of account.orders.filter(row => ['pending', 'awaiting_confirmation'].includes(row.status))) {
          order.status = 'cancelled'; order.reject_reason = '隔离样例：用户暂停并撤销未成交操作'; account.performance.cancelled += 1;
        }
      }
      return clone(account);
    }
    if (command === 'research_follow_delete') { keys(args, commandArgs[command]);findAccount(args.accountId);state.accounts=state.accounts.filter(view=>view.account_id!==args.accountId);return; }
    if (command === 'research_follow_refresh') { keys(args, commandArgs[command]); return clone(findAccount(args.accountId)); }
    if (command === 'research_follow_order') {
      keys(args, commandArgs[command]);
      const account = state.accounts.find(view => view.orders.some(row => row.id === args.orderId));
      const order = account?.orders.find(row => row.id === args.orderId);
      if (!order || !['awaiting_confirmation', 'pending'].includes(order.status)) throw Error('Mock contract: inactive order');
      if (args.action === 'cancel') {
        order.status = 'cancelled'; order.reject_reason = '隔离样例：用户撤销操作，没有记作成交'; account.performance.cancelled += 1;
      } else throw Error('Mock contract: unsupported action');
      return clone(account);
    }
    throw Error('Mock contract: forbidden/unmocked IPC ' + command);
  };
  window.__followMock = state;
  return state;
}

const mock = '\nimport {MODEL_CATALOG} from \'/src/types/research.ts\';const catalog=MODEL_CATALOG'+';const evidence='+JSON.stringify(allocation)+';const timing='+JSON.stringify(timing)+';const commandArgs='+JSON.stringify(commandArgs)+';\n('+installMock.toString()+')(catalog,evidence,timing,commandArgs);\n'+
`const s=window.__followMock, original=s.invoke, copy=x=>JSON.parse(JSON.stringify(x));
s.sourceRuns=copy(s.legalRuns);
for(const source of s.legalRuns)await original('research_follow_start',{input:{source_run_id:source.id,initial_cash_cny:100000,max_positions:evidence.models[source.model_id].recommendation.default_max_positions}});
s.stageBuys(s.accounts[0].account_id);s.reportFill(s.accounts[0].account_id,9101);
s.accounts[1].enabled=false;s.accounts[1].state='paused';s.calls=[];
s.auto={enabled:true,effective_enabled:true,state:'complete',message:'今天模型检查已完成，等待新交易日',next_check_at:null,as_of:'20260930',last_completed_day:'2026-10-05',last_completed_at:'2026-10-05T01:11:00Z',last_error:null,failures:0,models:s.accounts.map(a=>({model_id:a.model_id,account_id:a.account_id,state:a.enabled?'following':'paused',message:a.enabled?'复用独立账户，保留用户配置':'保留用户暂停设置'}))};
s.preset={initial_cash_cny:100000};
const simAccount=a=>({id:a.account_id,name:a.model_name,managed_by:'model_follow',initial_cash:String(Math.round(a.initial_cash_cny*10000)),current_cash:String(Math.round(a.cash_cny*10000)),mode:'auto',auto_enabled:a.enabled,manual_source_enabled:false,rule_source_enabled:true,ai_source_enabled:false,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:0,slippage_bps:0});
export async function invoke(command,args){
 if(command==='research_auto_status'){
   s.calls.push({command,args:copy(args)});if(args.enabled!==null){s.auto.enabled=args.enabled;s.auto.effective_enabled=args.enabled;for(const a of s.accounts)a.effective_enabled=a.enabled&&args.enabled;s.auto.state=args.enabled?'complete':'paused';s.auto.message=args.enabled?'今天模型检查已完成，等待新交易日':'自动模型交易已暂停，账户和持仓保留';}
   return copy(s.auto);
 }
 if(command==='simulation_auto_preset'){s.calls.push({command});return copy(s.preset);}
 if(command==='simulation_save_auto_preset'){s.calls.push({command,args:copy(args)});s.preset=copy(args.input);return copy(s.preset);}
 if(command==='simulation_list_accounts'){s.calls.push({command});return s.accounts.map(simAccount);}
 if(command==='simulation_get_detail'){s.calls.push({command,args:copy(args)});const a=s.accounts.find(a=>a.account_id===args.accountId);return {account:simAccount(a),targets:[],positions:a.positions.map(p=>({...p,cost_price:String(Math.round(p.cost_cny*10000))})),orders:[],recent_runs:[],metrics:{},source_stats:[],capital_adjustments:[]};}
 if(command==='simulation_live_status'){s.calls.push({command,args:copy(args)});return{engine:'daily_raw_open',message:'isolated',updated_at:null,plans:[],executions:[]};}
 return original(command,args);
}
export async function listen(){return()=>{};}`;
const entry = `
import {createApp,h,reactive} from 'vue';import {NConfigProvider,NMessageProvider,darkTheme,lightTheme} from 'naive-ui';
import Follow from '/src/components/research/ModelFollowTrading.vue';import '/src/assets/styles/variables.css';import '/src/assets/workspace.css';
const app=window.__followApp=reactive({active:true,preferred:undefined,theme:'light'});
createApp({render:()=>h(NConfigProvider,{theme:app.theme==='dark'?darkTheme:lightTheme},()=>h(NMessageProvider,null,()=>h('main',{style:'padding:16px;box-sizing:border-box;width:100%'},[h(Follow,{ref:c=>window.__followComponent=c,sourceRuns:[],active:app.active,preferredAccountId:app.preferred})])))}).mount('#app');
`;
const t=await uiHarness({name:'model-follow-ui',entry,mock});const checks=[];let refreshGeometry;const secondaryFontSizes=[];
const check=async(name,fn)=>{await fn();checks.push(name);console.log('PASS '+name);};
const calls=command=>t.evaluate('window.__followMock.calls.filter(c=>c.command==='+JSON.stringify(command)+')');
const refresh=()=>t.evaluate('window.__followComponent.refresh()');
const click=async selector=>{await t.evaluate('document.querySelector('+JSON.stringify(selector)+').scrollIntoView({block:"center"})');const p=await t.evaluate('(()=>{const r=document.querySelector('+JSON.stringify(selector)+').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()');await t.call('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',clickCount:1,...p});await t.call('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...p});};

try {
 await t.call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
 await t.wait('document.querySelectorAll(".follow-account-card").length===5');
 const ids=await t.evaluate('window.__followMock.accounts.map(a=>a.account_id)');

 await check('Three real polling cycles retain the selected detail and button geometry, including failure and explicit refresh',async()=>{
   await click('.follow-delete-trigger');await t.wait('document.querySelector(".follow-delete-confirm")');
   await t.evaluate('document.fonts.ready.then(()=>new Promise(r=>setTimeout(r,400)))');
   const deleteFontSize=await t.evaluate('parseFloat(getComputedStyle(document.querySelector(".follow-delete-heading p")).fontSize)');assert.equal(deleteFontSize,12);secondaryFontSizes.push({selector:".follow-delete-heading p",fontSizePx:[deleteFontSize]});
   await t.evaluate('(()=>{const c=window.__followComponent.$.setupState,a=window.__followRefreshAudit={samples:[],problems:[],quiet:true,id:c.account.account_id,account:JSON.stringify(c.account),node:document.querySelector(".follow-selected-account")};a.sample=()=>{a.samples.push([".follow-open-accounts",".follow-automatic-switch",".follow-trading-switch",".follow-delete-confirm"].map(selector=>{const r=document.querySelector(selector).getBoundingClientRect();return{selector,width:r.width,height:r.height}}));if(c.account?.account_id!==a.id||JSON.stringify(c.account)!==a.account||document.querySelector(".follow-selected-account")!==a.node)a.problems.push("selected detail changed or disappeared");if(a.quiet&&(c.reading||c.busy))a.problems.push("poll displayed loading");if(document.querySelector(".follow-delete-confirm").innerText!=="删除此模型账户")a.problems.push("read was shown as deletion");};a.sample();a.timer=setInterval(a.sample,25);})()');
   for(let cycle=0;cycle<3;cycle++){
     await t.evaluate('window.__followMock.deferNext="research_follow_accounts"');
     await t.evaluate('new Promise((resolve,reject)=>{const start=performance.now(),timer=setInterval(()=>{if(window.__followMock.deferred.length){clearInterval(timer);resolve();}else if(performance.now()-start>11500){clearInterval(timer);reject(Error("Timed account poll did not run"));}},25)})');
     assert.equal(await t.evaluate('window.__followComponent.$.setupState.reading'),false);
     await t.evaluate('new Promise(r=>setTimeout(r,350))');
     await t.evaluate((cycle===2?'window.__followMock.failNext={command:"research_follow_accounts",message:"隔离定时账户读取失败"};':'')+'window.__followMock.deferred.shift()()');
     await t.wait(cycle===2?'document.querySelector(".follow-error")?.innerText.includes("隔离定时账户读取失败")':'!window.__followComponent.$.setupState.busy');
   }
   assert.equal(await t.evaluate('window.__followComponent.$.setupState.account.account_id'),ids[0]);
   assert.match(await t.evaluate('document.querySelector(".follow-selected-account").innerText'),/模拟样例甲/);
   await t.evaluate('window.__followRefreshAudit.quiet=false;window.__followMock.deferNext="research_follow_accounts";void window.__followComponent.refresh()');await t.wait('window.__followMock.deferred.length===1');
   assert.equal(await t.evaluate('window.__followComponent.$.setupState.reading'),true);
   await t.evaluate('new Promise(r=>setTimeout(r,350))');await t.evaluate('window.__followMock.deferred.shift()()');await t.wait('!window.__followComponent.$.setupState.busy');
   const audit=await t.evaluate('(()=>{const a=window.__followRefreshAudit;clearInterval(a.timer);return{samples:a.samples,problems:a.problems,error:window.__followComponent.$.setupState.readError}})()');
   assert.deepEqual(audit.problems,[]);assert.equal(audit.error,'');assert.ok(audit.samples.length>30);
   refreshGeometry={cycles:3,samples:audit.samples.length,buttons:Object.fromEntries(audit.samples[0].map((row,index)=>[row.selector,Object.fromEntries(['width','height'].map(key=>{const sizes=audit.samples.map(sample=>sample[index][key]);const delta=Math.max(...sizes)-Math.min(...sizes);assert.ok(delta<=1,row.selector+' '+key+' changed '+delta+'px');return[key,delta];}))]))};
   await click('.follow-delete-cancel');
 });
 await check('Daily signal and intraday execution are visible outside help; explicit fields override legacy notes and unknown modes stay unregistered',async()=>{
   assert.match(await t.evaluate('document.querySelector(".follow-signal-schedule").innerText'),/完成日日线选股.*次日开盘实时盘口买入.*盘中/);
   assert.equal(await t.evaluate('document.querySelector(".follow-signal-schedule").closest("details")'),null);
   await t.evaluate('window.__followMock.originalExecution=JSON.parse(JSON.stringify(window.__followMock.accounts[0].execution));Object.assign(window.__followMock.accounts[0].execution,{signal_basis:"completed_daily",signal_label:"完成日日线选股",entry_schedule:"next_session_open",execution_basis:"live_depth",schedule_note:"前一交易日完整日线确定候选，下一交易日09:30–09:35用实时盘口买入；盘中实时检查委托与卖出。"})');await refresh();
   assert.match(await t.evaluate('document.querySelector(".follow-signal-schedule").innerText'),/前一交易日完整日线.*09:30–09:35.*实时盘口.*盘中实时检查委托与卖出/);
   await t.evaluate('window.__followMock.accounts[0].execution.signal_basis="intraday"');await refresh();
   assert.match(await t.evaluate('document.querySelector(".follow-signal-schedule").innerText'),/模式未登记/);assert.doesNotMatch(await t.evaluate('document.querySelector(".follow-signal-schedule b").innerText'),/完成日日线/);
   await t.evaluate('window.__followMock.accounts[0].execution=window.__followMock.originalExecution');await refresh();
 });
 await check('Daily view has no model/source/capital setup or manual transaction controls',async()=>{
   assert.equal(await t.evaluate('document.querySelectorAll(".model-follow .follow-setup,.model-follow .follow-create,.model-follow input,.model-follow select").length'),0);
   assert.equal((await calls('research_follow_setup')).length,0);assert.equal((await calls('research_follow_start')).length,0);
   assert.equal(await t.evaluate('[...document.querySelectorAll(".model-follow button")].some(b=>["确认买入","确认卖出","提交模拟指令","暂停研究提醒","恢复研究提醒"].includes(b.innerText))'),false);
 });

 await check('Status reads are idempotent and show completed/reused work without a rerun control',async()=>{
   const before=(await calls('research_auto_status')).length;await refresh();await refresh();
   assert.equal((await calls('research_auto_status')).length,before+2);assert.deepEqual((await calls('research_auto_status')).at(-1).args,{enabled:null});
   assert.match(await t.evaluate('document.querySelector(".follow-automatic-control").innerText'),/今日已处理/);assert.match(await t.evaluate('document.querySelector(".follow-account-card").innerText'),/已复用/);
   assert.equal((await calls('research_follow_setup')).length,0);assert.equal((await calls('research_follow_start')).length,0);
 });
 await check('Global pause/resume keeps independently paused models paused and does not create accounts',async()=>{
   await click('.follow-automatic-switch');await t.wait('document.querySelector(".follow-automatic-switch").getAttribute("aria-checked")==="false"');
   assert.deepEqual((await calls('research_auto_status')).at(-2).args,{enabled:false});assert.equal(await t.evaluate('window.__followComponent.$.setupState.totals.running'),0);assert.match(await t.evaluate('document.querySelector(".follow-account-card").innerText'),/全局已暂停/);
   await t.wait('!window.__followComponent.$.setupState.busy');await click('.follow-automatic-switch');await t.wait('document.querySelector(".follow-automatic-switch").getAttribute("aria-checked")==="true"');
   assert.equal(await t.evaluate('window.__followMock.accounts[1].enabled'),false);assert.equal((await calls('research_follow_setup')).length,0);
 });

 await check('Waiting-source and globally paused native states have explicit Chinese labels',async()=>{
   await t.evaluate('window.__followMock.auto.models.push({model_id:"test_source",state:"waiting_source",message:"等候模型数据"});window.__followMock.accounts[2].state="automatic_paused"');await refresh();
   await t.evaluate('document.querySelector(".follow-model-outcomes").open=true');assert.match(await t.evaluate('document.querySelector(".follow-model-outcomes").innerText'),/等待模型来源/);
   assert.match(await t.evaluate('document.querySelectorAll(".follow-account-card")[2].innerText'),/全局已暂停/);
   await t.evaluate('window.__followMock.auto.models.pop();window.__followMock.accounts[2].state="waiting_session";document.querySelector(".follow-model-outcomes").open=false');await refresh();
 });
 await check('All isolated model accounts show cash/equity/status and a holdings preview',async()=>{
   const cards=await t.evaluate('[...document.querySelectorAll(".follow-account-card")].map(b=>b.innerText)');
   assert.equal(cards.length,5);assert.match(cards[0],/现金/);assert.match(cards[0],/净值/);assert.match(cards[0],/模拟样例甲 800股/);assert.match(cards[1],/已暂停买卖/);await t.evaluate('window.scrollTo(0,0)');await t.screenshot('accounts-with-holdings');
 });
 await check('Selecting account cards changes the active holdings without transferring evidence',async()=>{
   await click('[data-account-id="'+ids[1]+'"]');await t.wait('document.querySelector(".follow-selected-account").dataset.selectedAccountId==="'+ids[1]+'"');
   assert.match(await t.evaluate('document.querySelector(".follow-selected-account").innerText'),/当前空仓/);
   await click('[data-account-id="'+ids[0]+'"]');await t.wait('document.querySelector(".follow-position").innerText.includes("模拟样例甲")');
 });
 await check('Only server-reported filled orders change holdings, and times are Beijing time',async()=>{
   assert.equal(await t.evaluate('document.querySelectorAll(".follow-position").length'),1);
   assert.match(await t.evaluate('document.querySelector(".follow-fill-price").innerText'),/800 股/);
   assert.match(await t.evaluate('document.querySelector(".follow-fill-price").innerText'),/2026-09-21 09:31:16/);
   assert.match(await t.evaluate('document.querySelector(".follow-order").innerText'),/待撮合/);
 });
 await check('Per-account pause submits the real camelCase IPC and does not alter other accounts',async()=>{
   const before=await t.evaluate('window.__followMock.accounts[0].cash_cny');await click('.follow-trading-switch');await t.wait('document.querySelector(".follow-trading-switch").innerText.includes("恢复")');
   assert.deepEqual((await calls('research_follow_update')).at(-1).args,{accountId:ids[0],maxPositions:null,enabled:false});
   assert.equal(await t.evaluate('window.__followMock.accounts[0].positions.length'),1);assert.equal(await t.evaluate('window.__followMock.accounts[0].cash_cny'),before);assert.equal(await t.evaluate('window.__followMock.accounts[2].enabled'),true);
   await click('.follow-trading-switch');await t.wait('window.__followMock.accounts[0].enabled');
 });
 await check('Failed pause preserves state; pending action cannot duplicate commands',async()=>{
   await t.evaluate('window.__followMock.failNext={command:"research_follow_update",message:"隔离失败可重试"}');await click('.follow-trading-switch');await t.wait('document.querySelector(".follow-error")?.innerText.includes("隔离失败可重试")');
   assert.equal(await t.evaluate('window.__followMock.accounts[0].enabled'),true);
   const n=(await calls('research_follow_update')).length;await t.evaluate('window.__followMock.deferNext="research_follow_update"');await click('.follow-trading-switch');await t.wait('window.__followMock.deferred.length===1');
   await t.evaluate('document.querySelector(".follow-trading-switch").click()');assert.equal((await calls('research_follow_update')).length,n+1);
   await t.evaluate('window.__followMock.deferred.shift()()');await t.wait('!window.__followComponent.$.setupState.busy');
 });
 await check('Model rules, allocation and timing research stay accessible but closed in daily use',async()=>{
   assert.equal(await t.evaluate('document.querySelector(".follow-account-options").open'),false);
   await t.evaluate('document.querySelector(".follow-account-options").open=true;document.querySelectorAll(".follow-account-options details").forEach(e=>e.open=true)');
   const text=await t.evaluate('document.querySelector(".follow-account-options").innerText');assert.match(text,/来源校验/);assert.match(text,/累计历史/);assert.match(text,/3ATR/);assert.match(text,/不代表全新样本外验证/);assert.equal(await t.evaluate('document.querySelectorAll(".follow-allocation-policy").length>=7'),true);
   await t.evaluate('document.querySelector(".follow-account-options").open=false');
 });
 await check('Unknown equity and prices are not converted to zero or NaN',async()=>{
   await t.evaluate('window.__followMock.accounts[0].equity_cny=null;window.__followMock.accounts[0].net_return_pct=null;window.__followMock.accounts[0].positions[0].mark_cny=null');await refresh();
   assert.match(await t.evaluate('document.querySelector(".follow-metrics").innerText'),/--/);assert.doesNotMatch(await t.evaluate('document.querySelector(".model-follow").innerText'),/NaN|Infinity/);
   assert.equal(await t.evaluate('document.querySelectorAll(".follow-overview dd")[2].innerText'), '--');
 });
 await check('Details opens the same dedicated simulation account, and configuration is outside research',async()=>{
   await click('[data-account-id="'+ids[1]+'"]');await click('.follow-account-details');await t.wait('document.querySelector(".simulation-account-cards button[aria-pressed=true]")?.dataset.accountId==="'+ids[1]+'"');
   assert.equal(await t.evaluate('document.querySelectorAll("[data-account-ownership=model_follow]").length'),1);
   await t.evaluate('[...document.querySelectorAll(".n-tabs-tab")].find(e=>e.innerText.includes("账户配置")).click()');await t.wait('document.querySelector(".model-account-config")');
   assert.match(await t.evaluate('document.querySelector(".model-account-config").innerText'),/独立配置/);assert.equal(await t.evaluate('document.querySelectorAll(".capital-editor").length'),1);
   await click('.dialog .n-base-close');await t.wait('!document.querySelector(".simulation-account-cards")');assert.equal(await t.evaluate('document.querySelector(".follow-selected-account").dataset.selectedAccountId'),String(ids[1]));await click('[data-account-id="'+ids[0]+'"]');
 });
 await check('Deleting an account shows a scoped confirmation, supports cancel and failures, then removes only that model',async()=>{
   const before=(await calls('research_follow_delete')).length;await click('.follow-delete-trigger');await t.wait('document.querySelector(".follow-delete-panel")');
   assert.match(await t.evaluate('document.querySelector(".follow-delete-panel").innerText'),/持仓/);await click('.follow-delete-cancel');assert.equal((await calls('research_follow_delete')).length,before);
   await click('.follow-delete-trigger');await t.evaluate('window.__followMock.failNext={command:"research_follow_delete",message:"隔离删除失败"}');await click('.follow-delete-confirm');await t.wait('document.querySelector(".follow-error")?.innerText.includes("隔离删除失败")');assert.equal(await t.evaluate('document.querySelectorAll(".follow-account-card").length'),5);
   await click('.follow-delete-confirm');await t.wait('document.querySelectorAll(".follow-account-card").length===4');assert.deepEqual((await calls('research_follow_delete')).at(-1).args,{accountId:ids[0]});assert.equal(await t.evaluate('window.__followMock.accounts.some(a=>a.account_id==='+ids[1]+')'),true);await t.evaluate('document.querySelector(".follow-model-outcomes").open=true');assert.match(await t.evaluate('document.querySelector(".follow-model-outcomes").innerText'),/已删除 · 不再自动建/);
 });
 await check('Inactive views stop polling and all current-account mutations',async()=>{
   await t.evaluate('window.__followApp.active=false');const n=(await calls('research_follow_accounts')).length;await refresh();assert.equal((await calls('research_follow_accounts')).length,n);
   assert.equal(await t.evaluate('document.querySelector(".follow-trading-switch").disabled'),true);await t.evaluate('window.__followApp.active=true');await t.wait('!window.__followComponent.$.setupState.busy');
 });
 await check('Normal and narrow light/dark layouts have no horizontal overflow',async()=>{
   await t.evaluate('document.querySelector(".follow-model-outcomes").open=false;window.scrollTo(0,0)');
   for(const width of [1280,390])for(const theme of ['light','dark']){await t.call('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:false});await t.evaluate('window.__followApp.theme='+JSON.stringify(theme)+';document.documentElement.dataset.theme='+JSON.stringify(theme));
   assert.equal(await t.evaluate('document.documentElement.scrollWidth<=innerWidth+1'),true);const sizes=await t.evaluate('[...document.querySelectorAll(".follow-overview small,.follow-card-money,.follow-automatic-control small")].map(e=>parseFloat(getComputedStyle(e).fontSize))');assert.deepEqual([...new Set(sizes)],[12]);secondaryFontSizes.push({width,theme,fontSizePx:[...new Set(sizes)]});await t.screenshot('accounts-'+theme+'-'+width);}
 });
 assert.equal(t.errors.length,0,JSON.stringify(t.errors));t.save({schema:'model-follow-ui-v2',passed:checks.length,checks,refreshGeometry,secondaryFontSizes,scope:'Real Vue and synthetic IPC; no native execution or market measurement'});console.log(JSON.stringify({passed:checks.length,output:t.output}));
} finally { await t.close(); }
