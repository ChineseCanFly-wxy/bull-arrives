import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';

// Real screener, store, analysis dialog and layout. Market/Claude IPC is mocked.
const root = fileURLToPath(new URL('..', import.meta.url));
const target = path.join(root, 'src-tauri', 'target');
const profile = mkdtempSync(path.join(target, 'simplified-screener-ui-check-'));
const previewDir = path.join(target, 'ui-previews');
const edge = process.env.NEWS_TEST_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const mock = `
const restored = location.search.includes('restored');
const initialSettings = location.search.includes('persisted') ? JSON.parse(localStorage.getItem('screener-test-settings') || '{}') :
 location.search.includes('new') ? {} : {
  universe_preset:restored?'all':'steady_trend',universe_manual_schema:'2',
  universe_filter:JSON.stringify(restored?{boards:['sh_main','bse'],price_min:3,volume_ratio_min:1.5,above_ma_days:20,macd_bullish:true,kdj_bullish:true,new_high_days:30,rise_from_low_days:60,rise_from_low_min:3,volume_price_rising:true}:{boards:['bse'],price_min:99}),
  universe_custom_presets:JSON.stringify([{id:'custom_saved',label:'我的历史条件'}])
 };
if(location.search.includes('corrupt'))initialSettings.universe_quote_conditions='{broken-saved-conditions';
window.__screenMock={calls:[],added:false,unsupported:false,settings:initialSettings,partialAll:false,batchDelay:false,pendingBatches:[]};
const validRows = Array.from({length:43},(_,i)=>({code:String(600101+i),name:'测试股'+String(i+1).padStart(2,'0'),board:'sh_main',is_st:false,is_delisting:false,price:10+i*.2,change_pct:2,amount:(43-i)*1e8,turnover_rate:2,total_market_cap:1e10,industry:'测试行业',concepts:['测试概念甲','测试概念乙','测试概念丙']}));
const excludedRows = [{...validRows[0],code:'920001',board:'bse',name:'北交所测试'},{...validRows[0],code:'600099',name:'*ST测试',is_st:true}];
function response(args,state){
 const all = [...excludedRows,...validRows];
 const rows = args.page ? all.slice((args.page-1)*(args.pageSize||20),args.page*(args.pageSize||20)) : state.partialAll?all.slice(0,-1):all;
 return {total_all:5000,total_matched:all.length,stale:false,source_label:'测试行情通道',volume_ratio_supported:!state.unsupported,change_60d_supported:!state.unsupported,listing_date_supported:!state.unsupported,sector_supported:!state.unsupported,skipped_conditions:state.unsupported?['量比']:[],history_notice:null,history_evaluated:0,rows};
}
export async function invoke(command,args){
 const state=window.__screenMock;state.calls.push({command,args:args==null?args:JSON.parse(JSON.stringify(args))});
 if(command==='get_settings')return {...state.settings};
 if(command==='set_setting'){state.settings[args.key]=args.value;localStorage.setItem('screener-test-settings',JSON.stringify(state.settings));return;}
 if(command==='get_filter_presets'){
  if(location.search.includes('fallback'))throw Error('catalog offline');
  const base={price_min:3,price_max:100,amount_min_wan:10000,market_cap_min_yi:30};
  return [{id:'all',label:'沪深基础股票池',filter:{},builtin:true},
   {id:'quote_liquidity',label:'流动性优先',description:'成交额≥1亿',filter:base,builtin:true},
   {id:'quote_active',label:'今日活跃',description:'成交额≥2亿',filter:{...base,amount_min_wan:20000,turnover_min:2,turnover_max:15,change_pct_min:0,change_pct_max:8},builtin:true},
   {id:'quote_gentle_rise',label:'温和上涨',description:'涨幅0—5%',filter:{...base,market_cap_max_yi:1000,turnover_min:1,turnover_max:8,change_pct_min:0,change_pct_max:5},builtin:true},
   {id:'steady_trend',label:'不应复活',filter:{},builtin:true}];
 }
 if(command==='get_sector_catalog')return [{kind:'industry',code:'BK0001',name:'行业测试'},{kind:'concept',code:'BK0002',name:'概念测试'}];
 if(command==='get_market_universe')return response(args,state);
 if(command==='batch_stock_status'){
  const result=args.symbols.map(symbol=>{
   const code=Number(symbol.slice(2));const fail=code===600109;
   return {symbol,score:fail?null:code===600110?99:code===600111?98:code===600112?97:40+code-600100,
    ready:!fail,waiting_for:null,buy_low:null,buy_high:null,stop_loss:null,take_profit:null,risk_reward:null,error:fail?'本地历史缺失':null,
    history:fail?null:{source:'local_stockdb',source_label:'测试StockDB',start_date:'2020-01-01',end_date:code===600111?'2026-09-29':code===600112?null:'2026-09-30',bars:1600,adjustment:'qfq',stale:code===600110,warning:code===600110?'行情陈旧':null}};
  });
  if(state.batchDelay)return new Promise(resolve=>state.pendingBatches.push(()=>resolve(result)));
  return result;
 }
 if(command==='get_research_model_screen')return {schema:'research-model-screen-view-v1',current_as_of:'2026-09-30',fresh:true,revalidation_complete:true,models:[],limitations:[],screen:{schema:'ashare-model-screen-v1',as_of:'2026-09-30',run_id:'mock',exploration:true,production_admission:false,models:[],limitations:[]}};
 if(command==='analyze_stock')return {total_score:60,verdict:'关注',factors:[{name:'测试因子',score:60,weight:1,note:'仅测试'}],close:10,ma20:9,ma60:8,macd_dif:null,rsi12:null,boll_upper:null,boll_lower:null,momentum20:.02,momentum60:.1,volume_ratio:1,trade_plan:null,backtest:null,levels:[],rule_match:null,history:{source:'local_stockdb',source_label:'测试StockDB',end_date:'2026-09-30',bars:100,adjustment:'qfq',stale:false},agent_context_fingerprint:null,research_context:null};
 if(command==='get_agent_status')return {installed:false,state:'not_found',message:'测试未调用Claude',guidance:'仅测试交互'};
 if(command==='add_watch'){state.added=true;return;}
 if(command==='get_group_snapshot')return {active_group_id:0,groups:[],items:state.added?[{id:1,code:'sh600101',market:'CN',name:'测试股01',sort_order:0,added_at:''}]:[]};
 throw Error('Unexpected IPC: '+command);
}
export async function listen(){return ()=>{};}
export async function emit(){}
export async function open(){return null;}
export async function enable(){} export async function disable(){} export async function isEnabled(){return false;}
`;
const server = await createServer({
 configFile:false,root,cacheDir:path.join(profile,'vite-cache'),
 optimizeDeps:{entries:[],include:['vue','pinia','naive-ui'],exclude:['@tauri-apps/api/core','@tauri-apps/api/event','@tauri-apps/plugin-dialog']},
 resolve:{alias:{'@':path.join(root,'src')},dedupe:['vue']},
 plugins:[vue(),{
  name:'screener-workflow-check',enforce:'pre',
  resolveId(id){if(id.endsWith('/detail/StockDetail.vue'))return '\0model-detail';if(id.startsWith('@tauri-apps/'))return '\0model-test-ipc';if(id==='/model-test-entry.js')return '\0model-test-entry';},
  load(id){
   if(id==='\0model-detail')return "import {h} from 'vue';export default {props:['item'],render(){return h('p',{class:'mock-detail'},'行情详情 '+this.item.code);}};";
   if(id==='\0model-test-ipc')return mock;
   if(id==='\0model-test-entry')return `
    import {createApp,h,ref} from 'vue';import {createPinia} from 'pinia';import {NConfigProvider,darkTheme} from 'naive-ui';
    import Universe from '/src/components/screener/UniverseScreenerDialog.vue';import {useUniverseStore} from '/src/stores/universe.ts';
    import '/src/assets/styles/variables.css';import '/src/assets/workspace.css';
    const visible=ref(true),theme=ref('light');window.__screenVisible=visible;window.__screenTheme=theme;document.documentElement.dataset.theme='light';
    const app=createApp({render:()=>h(NConfigProvider,{theme:theme.value==='dark'?darkTheme:null},()=>h(Universe,{show:visible.value,'onUpdate:show':value=>visible.value=value}))});const pinia=createPinia();app.use(pinia);window.__universe=useUniverseStore(pinia);app.mount('#app');
   `;
  },
  configureServer(s){s.middlewares.use((req,res,next)=>{if(req.url?.split('?')[0]!=='/')return next();res.setHeader('Content-Type','text/html');res.end('<html><head><meta charset="utf-8"></head><body><div id="app"></div><script type="module" src="/model-test-entry.js"></script></body></html>');});},
 }],server:{host:'127.0.0.1',port:0,strictPort:false,watch:{ignored:['**/src-tauri/**']}},
});
let browser,ws,nextId=0;const pending=new Map();
try{
 await server.listen();const port=server.httpServer.address().port;
 browser=spawn(edge,['--headless=new','--disable-gpu','--disable-extensions','--no-first-run','--no-default-browser-check','--remote-debugging-port=0','--window-size=1360,940',`--user-data-dir=${profile}`,`http://127.0.0.1:${port}/`],{windowsHide:true,stdio:'ignore'});
 let debugPort;for(let i=0;i<100;i++){try{debugPort=Number(readFileSync(path.join(profile,'DevToolsActivePort'),'utf8').split('\n')[0]);break;}catch{await new Promise(r=>setTimeout(r,100));}}
 assert.ok(debugPort,'Headless Edge did not start');
 const pages=await(await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();ws=new WebSocket(pages.find(p=>p.type==='page'&&p.url.startsWith('http://127.0.0.1:'+port+'/')).webSocketDebuggerUrl);
 await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
 ws.addEventListener('message',event=>{const result=JSON.parse(event.data);const task=pending.get(result.id);if(task){pending.delete(result.id);result.error?task.reject(Error(JSON.stringify(result.error))):task.resolve(result.result);}});
 const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++nextId;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
 const evaluate=async expression=>{const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
 const waitFor=async expression=>{for(let i=0;i<120;i++){if(await evaluate(expression))return;await new Promise(r=>setTimeout(r,100));}console.log(await evaluate(`({body:document.body.innerText,calls:window.__screenMock?.calls})`));throw Error('UI timeout: '+expression);};
 const count=command=>evaluate(`window.__screenMock.calls.filter(c=>c.command===${JSON.stringify(command)}).length`);
 const screenshot=async name=>{mkdirSync(previewDir,{recursive:true});const image=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(path.join(previewDir,name),Buffer.from(image.data,'base64'));};
 await call('Runtime.enable');await call('Emulation.setDeviceMetricsOverride',{width:1360,height:940,deviceScaleFactor:1,mobile:false});
 await waitFor(`document.querySelector('.pool-toolbar')&&window.__universe?.migrationNotice`);
 assert.equal(await count('get_market_universe'),0,'Opening only restores settings');
 assert.deepEqual(await evaluate(`window.__universe.presets.map(p=>p.id)`),['all','quote_liquidity','quote_active','quote_gentle_rise']);
 assert.equal(await evaluate(`document.querySelector('#pool-tab').getAttribute('aria-selected')`),'true');
 assert.ok(await evaluate(`document.querySelector('#pool-tab').innerText.includes('当前')`));
 assert.equal(await evaluate(`getComputedStyle(document.querySelector('.pool-condition-panel')).display`),'none');
 for(const field of ['above_ma_days','new_high_days','macd_bullish','kdj_bullish','volume_price_rising','rise_from_low_days'])assert.equal(await evaluate(`!!document.querySelector('[data-filter="${field}"]')`),false);
 await evaluate(`document.querySelectorAll('.pool-presets button')[2].click()`);await waitFor(`window.__universe.activePreset?.id==='quote_active'`);
 assert.equal(await count('get_market_universe'),0,'Preset applies conditions without querying');
 await evaluate(`document.querySelector('.pool-query').click()`);await waitFor(`window.__universe.hasLoaded&&!window.__universe.loading`);
 assert.deepEqual(await evaluate(`(()=>{const f=window.__screenMock.calls.find(c=>c.command==='get_market_universe').args.filter;return [f.price_min,f.price_max,f.amount_min_wan,f.turnover_min,f.turnover_max,f.change_pct_min,f.change_pct_max,f.exclude_suspended,f.exclude_limit_locked,f.macd_bullish,f.above_ma_days]})()`),[3,100,20000,2,15,0,8,true,true,false,null]);
 assert.equal(await evaluate(`document.querySelectorAll('.pool-table-scroll th').length`),9);
 assert.equal(await evaluate(`document.querySelectorAll('.pool-table-scroll tbody tr').length`),18,'ST and BSE hidden');
 assert.equal(await count('batch_stock_status'),0,'Query does not compute scores');
 assert.equal(await evaluate(`Array.from(document.querySelectorAll('[data-score-status]')).every(el=>el.dataset.scoreStatus==='not-scored'&&el.innerText.includes('--'))`),true);
 assert.match(await evaluate(`document.querySelector('.sector-cell').innerText`),/测试行业.*\n.*测试概念/);
 const firstQueryCount=await count('get_market_universe');
 await evaluate(`window.__universe.filter.price_min=4`);await waitFor(`document.querySelector('.pool-toolbar-summary').innerText.includes('待查询')`);
 assert.equal(await count('get_market_universe'),firstQueryCount);
 await evaluate(`window.__universe.fetchPage(2)`);await waitFor(`window.__universe.page===2&&!window.__universe.loading`);
 assert.equal(await evaluate(`window.__screenMock.calls.filter(c=>c.command==='get_market_universe').at(-1).args.filter.price_min`),3,'Paging uses the last submitted filter');
 await evaluate(`document.querySelector('.pool-query').click()`);await waitFor(`!window.__universe.loading&&!window.__universe.conditionsChanged`);
 assert.equal(await evaluate(`window.__screenMock.calls.filter(c=>c.command==='get_market_universe').at(-1).args.filter.price_min`),4);
 await evaluate(`document.querySelector('.pool-filter-toggle').click()`);await waitFor(`getComputedStyle(document.querySelector('.pool-condition-panel')).display!=='none'`);
 const beforeInvalid=await count('get_market_universe');await evaluate(`window.__universe.filter.price_max=2;document.querySelector('.pool-query').click()`);await waitFor(`document.querySelector('.pool-error')?.innerText.includes('下限不能大于上限')`);
 assert.equal(await count('get_market_universe'),beforeInvalid,'Invalid range rejected before IPC');await evaluate(`window.__universe.filter.price_max=100;document.querySelector('.pool-query').click()`);await waitFor(`!window.__universe.loading&&!window.__universe.conditionsChanged`);
 const beforeSave=await count('get_market_universe');
 await evaluate(`Array.from(document.querySelectorAll('.pool-saved button')).find(b=>b.innerText==='另存为').click()`);await waitFor(`document.querySelector('.condition-editor input')`);
 await evaluate(`var input=document.querySelector('.condition-editor input');input.value='我的放量条件';input.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.condition-editor form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))`);
 await waitFor(`window.__universe.savedConditions.length===1&&!document.querySelector('.condition-editor')`);
 assert.equal(await count('get_market_universe'),beforeSave,'Saving does not query');
 assert.equal(await evaluate(`window.__universe.selectedCondition.label`),'我的放量条件');
 await evaluate(`window.__universe.filter.price_min=5;Array.from(document.querySelectorAll('.pool-saved button')).find(b=>b.innerText==='改名').click()`);await waitFor(`document.querySelector('.condition-editor input')`);
 await evaluate(`var input=document.querySelector('.condition-editor input');input.value='重命名条件';input.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.condition-editor form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))`);await waitFor(`window.__universe.selectedCondition?.label==='重命名条件'`);
 assert.equal(await evaluate(`window.__universe.selectedCondition.filter.price_min`),4,'Rename preserves the saved conditions, not unsaved edits');
 await evaluate(`Array.from(document.querySelectorAll('.pool-saved button')).find(b=>b.innerText==='覆盖条件').click()`);await waitFor(`document.querySelector('.condition-editor input')`);await evaluate(`document.querySelector('.condition-editor form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))`);await waitFor(`window.__universe.selectedCondition?.filter.price_min===5`);
 await evaluate(`window.__universe.saveCondition('另一个条件')`);await evaluate(`(async()=>{try{await window.__universe.saveCondition('不限条件')}catch(e){window.__duplicateConditionError=String(e)}})()`);assert.match(await evaluate(`window.__duplicateConditionError`),/同名条件/);
 const deleteId=await evaluate(`window.__universe.selectedCondition.id`);await evaluate(`Array.from(document.querySelectorAll('.pool-saved button')).find(b=>b.innerText==='删除').click()`);await waitFor(`document.body.innerText.includes('删除“另一个条件”')`);await evaluate(`Array.from(document.querySelectorAll('.editor-actions button')).find(b=>b.innerText==='删除条件').click()`);await waitFor(`window.__universe.savedConditions.length===1`);assert.equal(await evaluate(`window.__universe.savedConditions.some(p=>p.id===${JSON.stringify(deleteId)})`),false);
 assert.equal(await evaluate(`window.__screenMock.calls.some(c=>c.command==='set_setting'&&c.args.key==='universe_custom_presets')`),false,'Retired archive is untouched');
 await evaluate(`(async()=>{await window.__universe.applyPreset(window.__universe.savedConditions[0].id);document.querySelector('.pool-query').click()})()`);await waitFor(`!window.__universe.loading&&!window.__universe.conditionsChanged`);
 await evaluate(`document.querySelector('.row-actions .accent').click()`);await waitFor(`document.querySelector('.analysis')`);assert.equal(await count('analyze_stock'),1,'Analyze row opens real manual stock analysis');assert.equal(await evaluate(`document.querySelector('.ai-disclosure').open`),false);assert.equal(await evaluate(`window.__screenMock.calls.some(c=>/^(analyze_stock_agent|analyze_stock_team|start_live_analysis|start_interactive_analysis)$/.test(c.command))`),false);
 await evaluate(`Array.from(document.querySelectorAll('.n-card-header__close')).at(-1).click()`);await waitFor(`!document.querySelector('.analysis')`);
 await evaluate(`document.querySelectorAll('.pool-score-actions button')[0].click()`);await waitFor(`!window.__universe.scoring&&window.__universe.scoreDone>0`);assert.ok(await count('batch_stock_status'));assert.equal(await evaluate(`window.__screenMock.calls.filter(c=>c.command==='batch_stock_status').at(-1).args.rule`),'auto');
 const beforeAll=await count('get_market_universe');await evaluate(`document.querySelectorAll('.pool-score-actions button')[1].click()`);await waitFor(`!window.__universe.scoring&&window.__universe.rankingSorted`);
 assert.equal(await count('get_market_universe'),beforeAll+1);assert.equal(await evaluate(`window.__screenMock.calls.filter(c=>c.command==='get_market_universe').at(-1).args.page`),undefined,'All matches are requested without page or top-N cap');
 assert.deepEqual(await evaluate(`window.__screenMock.calls.filter(c=>c.command==='batch_stock_status').slice(-3).map(c=>c.args.symbols.length)`),[20,20,3]);
 assert.equal(await evaluate(`window.__universe.scoreTotal`),43);assert.equal(await evaluate(`window.__universe.scoreDone`),43);assert.equal(await evaluate(`window.__universe.scoreFailed`),1);assert.equal(await evaluate(`window.__universe.scoreComparable`),39);
 assert.equal(await evaluate(`window.__universe.rows[0].code`),'600143','Highest same-date successful score wins');
 assert.equal(await evaluate(`window.__screenMock.calls.filter(c=>c.command==='batch_stock_status').flatMap(c=>c.args.symbols).some(s=>s.startsWith('bj')||s==='sh600099')`),false);
 const beforeRankPage=await count('get_market_universe');await evaluate(`window.__universe.fetchPage(3)`);await waitFor(`window.__universe.page===3`);assert.equal(await count('get_market_universe'),beforeRankPage,'Ranked paging uses the fully scored list');
 await evaluate(`window.__universe.fetchPage(2)`);await waitFor(`window.__universe.page===2&&!!document.querySelector('[data-score-status="failed"]')`);assert.ok(await evaluate(`document.querySelector('[data-score-status="failed"]')`));await evaluate(`window.__universe.fetchPage(3)`);await waitFor(`window.__universe.page===3&&!!document.querySelector('[data-score-status="stale"]')`);assert.ok(await evaluate(`document.querySelector('[data-score-status="stale"]')`));assert.ok(await evaluate(`document.querySelector('[data-score-status="outdated"]')`));
 await evaluate(`window.__universe.fetchPage(1);document.querySelector('.pool-filter-toggle').click();document.querySelector('.screener-scroll').scrollTop=0`);await new Promise(r=>setTimeout(r,450));await screenshot('market-screener-light.png');
 const positions=await evaluate(`(()=>{const q=s=>document.querySelector(s).getBoundingClientRect();return {nav:q('.market-screener-nav').top,toolbar:q('.pool-toolbar').top,pager:q('.pool-pagination').top,close:q('.universe-screener-modal .n-card-header__close').top};})()`);
 await evaluate(`document.querySelector('.screener-scroll').scrollTop=99999`);await new Promise(r=>setTimeout(r,100));const afterPositions=await evaluate(`(()=>{const q=s=>document.querySelector(s).getBoundingClientRect();return {nav:q('.market-screener-nav').top,toolbar:q('.pool-toolbar').top,pager:q('.pool-pagination').top,close:q('.universe-screener-modal .n-card-header__close').top};})()`);assert.deepEqual(afterPositions,positions,'Tabs/query/pager/close stay visible while results scroll');
 await evaluate(`window.__screenTheme.value='dark';document.documentElement.dataset.theme='dark'`);await new Promise(r=>setTimeout(r,150));await screenshot('market-screener-dark.png');
 await call('Emulation.setDeviceMetricsOverride',{width:640,height:640,deviceScaleFactor:1,mobile:false});await new Promise(r=>setTimeout(r,200));
 assert.equal(await evaluate(`document.documentElement.scrollWidth<=innerWidth`),true,'Compact layout does not widen the outer window');assert.ok(await evaluate(`document.querySelector('.pool-pagination').getBoundingClientRect().bottom<=innerHeight`));assert.ok(await evaluate(`document.querySelector('.screener-scroll').clientHeight>80`));await screenshot('market-screener-compact.png');
 await call('Emulation.setDeviceMetricsOverride',{width:1360,height:940,deviceScaleFactor:1,mobile:false});
 await evaluate(`window.__screenMock.batchDelay=true;void window.__universe.scoreResults('all')`);await waitFor(`window.__screenMock.pendingBatches.length===1`);await evaluate(`window.__screenMock.pendingBatches[0]()`);await waitFor(`window.__universe.scoreDone===20&&window.__screenMock.pendingBatches.length===2`);const beforeCancelBatch=await count('batch_stock_status');await evaluate(`window.__universe.cancelScoring();window.__screenMock.pendingBatches[1]()`);await waitFor(`!window.__universe.scoring`);assert.equal(await count('batch_stock_status'),beforeCancelBatch);assert.equal(await evaluate(`window.__universe.scoreDone`),20);assert.equal(await evaluate(`window.__universe.rankingSorted`),false);assert.match(await evaluate(`document.querySelector('.pool-ranking-report').innerText`),/已停止.*部分评分/);
 await evaluate(`window.__screenMock.batchDelay=false;window.__screenMock.partialAll=true;window.__universe.scoreResults('all')`);await waitFor(`!window.__universe.scoring&&window.__universe.scoreError.includes('未返回全部')`);assert.equal(await count('batch_stock_status'),beforeCancelBatch,'Incomplete all-market response never enters the score loop');
 await evaluate(`window.__screenMock.partialAll=false;window.__universe.scoreResults('all')`);await waitFor(`!window.__universe.scoring&&window.__universe.rankingSorted`);
 await evaluate(`window.__screenVisible.value=false`);await waitFor(`!document.querySelector('.market-screener')`);await evaluate(`window.__screenVisible.value=true`);await waitFor(`document.querySelector('.pool-toolbar')`);assert.equal(await evaluate(`Object.keys(window.__universe.statuses).length`),0,'Reopening leaves scores blank until manual calculation');
 assert.equal(await evaluate(`window.__universe.activePreset?.label`),'重命名条件');
 const beforeTab=await count('get_market_universe');await evaluate(`document.querySelector('#models-tab').click()`);await waitFor(`document.querySelector('.research-model-screen')`);assert.equal(await count('get_market_universe'),beforeTab);await evaluate(`window.__screenVisible.value=false`);await waitFor(`!document.querySelector('.market-screener')`);await evaluate(`window.__screenVisible.value=true`);await waitFor(`document.querySelector('.research-model-screen')`);assert.equal(await evaluate(`document.querySelector('#models-tab').getAttribute('aria-selected')`),'true','Last tab restored');
 await call('Page.navigate',{url:`http://127.0.0.1:${port}/?persisted`});await waitFor(`window.__universe?.savedConditions.length===1&&document.querySelector('.research-model-screen')`);assert.equal(await evaluate(`window.__universe.filter.price_min`),5);assert.equal(await evaluate(`window.__universe.savedConditions[0].label`),'重命名条件');assert.equal(await count('get_market_universe'),0);assert.equal(await count('batch_stock_status'),0);
 await call('Page.navigate',{url:`http://127.0.0.1:${port}/?restored`});await waitFor(`window.__universe?.migrationNotice&&document.querySelector('.pool-toolbar')`);assert.deepEqual(await evaluate(`(()=>{const f=window.__universe.filter;return [f.price_min,f.volume_ratio_min,f.above_ma_days,f.new_high_days,f.macd_bullish,f.kdj_bullish,f.volume_price_rising,f.rise_from_low_days,f.rise_from_low_min]})()`),[3,1.5,null,null,false,false,false,null,null]);assert.equal(await evaluate(`window.__screenMock.calls.some(c=>c.command==='set_setting'&&c.args.key==='universe_archived_manual_filter'&&JSON.parse(c.args.value).above_ma_days===20)`),true);
 await call('Page.navigate',{url:`http://127.0.0.1:${port}/?new`});await waitFor(`window.__universe?.presets.length===4&&document.querySelector('.pool-toolbar')`);assert.equal(await evaluate(`window.__universe.activePreset?.id`),'all','New install uses no optional conditions');assert.equal(await count('get_market_universe'),0);
 await call('Page.navigate',{url:`http://127.0.0.1:${port}/?corrupt`});await waitFor(`document.querySelector('.pool-toolbar')&&window.__universe?.restoreError.includes('已备份')`);assert.equal(await evaluate(`window.__screenMock.settings.universe_quote_conditions_backup`),'{broken-saved-conditions');assert.equal(await evaluate(`window.__screenMock.settings.universe_quote_conditions`),'{broken-saved-conditions','Corrupt source stays unchanged on open');await evaluate(`window.__universe.saveCondition('新的手动条件')`);assert.equal(await evaluate(`window.__screenMock.settings.universe_quote_conditions_backup`),'{broken-saved-conditions','New condition does not erase the recovery backup');
 await call('Page.navigate',{url:`http://127.0.0.1:${port}/?fallback`});await waitFor(`window.__universe?.migrationNotice&&document.querySelector('.pool-toolbar')`);assert.deepEqual(await evaluate(`window.__universe.presets.map(p=>p.id)`),['all']);
 console.log('Market screener workflow passed: explicit query/preset, submitted-filter paging, condition save/overwrite/rename/delete/reload, empty initial scores, manual stock analysis, 43-stock complete scoring/ranking, missing/stale/date mismatch handling, cancellation, incomplete-response refusal, remembered tab, fixed layout at 1360 and 640 pixels. Market and Claude IPC mocked.');
 console.log('UI previews: '+previewDir);
}finally{
 ws?.close();if(browser?.exitCode===null){const exited=new Promise(r=>browser.once('exit',r));browser.kill();await Promise.race([exited,new Promise(r=>setTimeout(r,2000))]);}await server.close();
 assert.equal(path.dirname(path.resolve(profile)),path.resolve(target));assert.ok(path.basename(profile).startsWith('simplified-screener-ui-check-'));
 try{rmSync(profile,{recursive:true,force:true,maxRetries:10,retryDelay:100});}catch{console.log('Temporary browser profile remains in '+profile);}
}
