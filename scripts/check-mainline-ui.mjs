import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';

// Real Vue components in an isolated Edge profile. All IPC and files are fixtures.
const sample = {
  sector_code:'BK0001',sector_name:'测试主线',as_of:'2026-09-30',sector_source:'测试日线',member_as_of:'2026-09-30T15:30:00+08:00',member_source:'全分页测试',
  total_members:12,excluded_members:2,covered_members:10,missing_members:[],complete:true,strong:true,status:'持续趋势满足，研究观察',
  metrics:{r5:5.2,r20:12.3,r60:20.4,rs20_vs_hs300:3.5,rs60_vs_hs300:7.5,ma20:100,ma60:90,persistence:4,removed_non_trading_rows:3},
  trend_breadth:.6,trending_members:6,benchmark:{symbol:'sh000300',source:'测试真实指数日线',as_of:'2026-09-30'},
  membership_observed_at:'2026-10-01T00:00:00Z',membership_source_is_current_snapshot:true,
  leaders:[{symbol:'sh600001',name:'测试强股',as_of:'2026-09-30',score:10,r20:14,r60:22,rs20_vs_hs300:5.2,rs60_vs_hs300:9.1,ma20:9.8,ma60:9.3,atr:.4,close:10,
    plan:{status:'unvalidated_observation',max_position_pct:8,reference_prices:{as_of:'2026-09-30',close:10,ma20:9.8,atr:.4,prior_high20:10.5},trial:{position_pct:1.6,buy_low:9.7,buy_high:9.9,condition:'完成分钟站上VWAP'},confirm:{position_pct:2.4,buy_low:10.5,buy_high:10.7,condition:'突破确认'},final:{position_pct:4,condition:'回踩确认'},profit_trim:{sell_position_pct:30,atr_above_cost:2,trigger:'成本盈利两倍ATR后回落'},weakness_trim:{sell_position_pct:50,trigger_price:9.8,condition:'完成日跌破MA20'},exit:{atr_from_cost:3,preferred_days:15,max_holding_days:20,review_range_days:[5,20]},price_basis:'截止日未复权参考价格'}}],
  news:[{id:'news:1',title:'<img src=x onerror=alert(1)>测试公告',body:'公司公告原文',source:'发行人公告',published_at:null,url:'javascript:alert(1)'},{id:'news:2',title:'来源2',body:'另一原文',source:'公司',published_at:'2026-09-30',url:'https://example.com/report'}],
  limitations:['当前成分非历史时点；本机制尚未验证'],fingerprint:'a'.repeat(64),generated_at:'2026-10-01T09:00:00Z'
};
const modelContext={schema:'stock-research-context-v1',requested_as_of:'2026-09-30',frozen_as_of:'2026-09-30',date_matches:true,run_id:'frozen-real-model',snapshot_sha256:'b'.repeat(64),screen_sha256:'c'.repeat(64),evidence_sha256:'d'.repeat(64),exploration:true,production_admission:false,
  models:[{id:'breadth22_h20',name:'技术+市场广度20日',feature_count:22,holding_days:20,signal_status:'positive_record',score:.0123,score_as_of:'2026-09-30',observation:{signal_eligible:true},performance:{train:{net_return_pct:-5.51,max_drawdown_pct:33},validation:{net_return_pct:12.58,max_drawdown_pct:34},test:{net_return_pct:55.88,max_drawdown_pct:21},double_cost_return_pct:49.41,status:'未准入研究'}},
    {id:'index26_h20',name:'技术+广度+大盘20日',feature_count:26,holding_days:20,signal_status:'nonpositive_record',score:-.0234,score_as_of:'2026-09-30',observation:null,performance:{train:{net_return_pct:-6,max_drawdown_pct:35},validation:{net_return_pct:7,max_drawdown_pct:28},test:{net_return_pct:22,max_drawdown_pct:24},double_cost_return_pct:19,status:'未准入研究'}}],limitations:['真实冻结记录，未知不猜分数，未准入']};
sample.model_research=structuredClone(modelContext);sample.leaders[0].model_research=structuredClone(modelContext);sample.content_sha256='e'.repeat(64);
sample.news[0].received_at='2026-10-01T00:01:00Z';sample.news[0].published_after_market_asof=true;
sample.news[1].published_at=null;sample.news[1].published_date='2026-09-30';sample.news[1].publication_precision='date';sample.news[1].source_index_only=true;
const evidenceSample = {
  schema:'research-evidence-v1',run_id:'mock frozen study',as_of:'2026-09-30',
  data:{stocks:5000,sessions:2123,first_date:20100104,last_date:20260930,sha256:'b'.repeat(64),source:'<img src=x onerror=alert(1)>测试本地导出'},
  models:Array.from({length:15},(_,i)=>({id:i===0?'breadth22_h20':`model_${i}`,name:i===14?'失败模型完整保留':`模型${i}`,features:[14,22,26][i%3],holding_days:[5,10,15,20,30][i%5],train:{net_return_pct:-7-i,max_drawdown_pct:30},validation:{net_return_pct:4,max_drawdown_pct:20},test:{net_return_pct:i===14?null:55,max_drawdown_pct:21},double_cost_return_pct:i===14?null:48,win_rate_pct:54,closed_cycles:i===14?0:187,unique_stocks:100,baseline_percentile:{train:20,validation:50,test:70},status:i===14?'fit_failed: 保留失败原因':'unadmitted_research'})),
  limitations:['历史ST并非已认证PIT'],next_checks:['冻结后新增行情前向检验'],integrity_note:'内部内容完整，不代表获准',content_sha256:'c'.repeat(64),
  annual_choices:{schema:'<script>window.__evidenceXss=true</script>',2024:'冻结后选择'},stress:{schema:'<img src=x onerror=alert(1)>',delay_return:-1},checks:{schema:'<img src=x onerror=alert(1)>',causal:true}
};
const snapshot29=structuredClone(sample);Object.assign(snapshot29,{kind:'industry',as_of:'2026-09-29',member_as_of:'2026-09-29',fingerprint:'f'.repeat(64)});snapshot29.metrics.rs20_vs_hs300=5.5;snapshot29.benchmark.as_of='2026-09-29';snapshot29.leaders[0].name='留存领涨股';snapshot29.leaders[0].as_of='2026-09-29';snapshot29.leaders[0].plan.reference_prices.as_of='2026-09-29';
const snapshot28=structuredClone(snapshot29);Object.assign(snapshot28,{as_of:'2026-09-28',member_as_of:'2026-09-28',fingerprint:'d'.repeat(64),strong:false});snapshot28.metrics.rs20_vs_hs300=3.5;snapshot28.benchmark.as_of='2026-09-28';snapshot28.leaders=[];
const historySamples=[{schema:'mainline-research-notification-v1',fingerprint:snapshot29.fingerprint,mainline_snapshot:snapshot29},{schema:'mainline-research-notification-v1',fingerprint:snapshot28.fingerprint,mainline_snapshot:snapshot28},{schema:'stock-model-research-notification-v1'},{schema:'mainline-research-notification-v1',fingerprint:'invalid',mainline_snapshot:{...snapshot29,fingerprint:'invalid'}}];
const sectorRows = [
  {kind:'industry',code:'BK0001',name:'测试行业板块',rank:1,latest:1234.56,change_amount:12.34,change_pct:2.5,amount:300_000_000,market_cap:900_000_000,turnover_rate:3.2,up_count:8,down_count:2,leader_name:'测试板块成分股',leader_change_pct:4.2,change_pct_3d:-1.5,change_pct_5d:2.5,change_pct_10d:4.5,main_net_inflow:200_000_000,main_net_ratio:6.5,super_large_net_inflow:120_000_000,large_net_inflow:80_000_000,medium_net_inflow:-50_000_000,small_net_inflow:-30_000_000},
  {kind:'concept',code:'BK0003',name:'测试概念板块',rank:1,latest:2345.67,change_amount:23.45,change_pct:1.5,amount:200_000_000,market_cap:800_000_000,turnover_rate:2.2,up_count:7,down_count:3,leader_name:'测试板块成分股',leader_change_pct:4.2,change_pct_3d:1,change_pct_5d:3,change_pct_10d:5,main_net_inflow:-30_000_000,main_net_ratio:-1.5,super_large_net_inflow:-20_000_000,large_net_inflow:-10_000_000,medium_net_inflow:20_000_000,small_net_inflow:10_000_000},
];
const sectorMember={code:'600001',market:'SH',name:'测试板块成分股',price:10.5,change_amount:.5,change_pct:5,amount:20_000_000,turnover_rate:2.1,market_cap:100_000_000,pe:12.3,pb:1.2};
const sectorBars=Array.from({length:10},(_,i)=>({date:'2026-09-'+String(15+i).padStart(2,'0'),open:100+i,close:101+i,high:102+i,low:99+i,volume:1000+i*100,amount:100_000+i*1000,change_pct:1,turnover_rate:2}));
const mock = `
export const calls=[];
window.__mainlineMock={calls,variant:'complete',holdStart:false,holdBackground:false,startPending:false,startReplies:0,scanTimer:null,fail:false,watchFail:false,historyFail:false,expired:false,watched:false,opened:null,importPath:'C:/research/frozen.json',evidence:${JSON.stringify(evidenceSample)},history:${JSON.stringify(historySamples)},discovery:{enabled:false,busy:false,as_of:'2026-09-30',total:3,processed:0,failed:[],candidates:[],finished:false,last_error:null}};
export async function invoke(command,args){
 calls.push({command,args:args?JSON.parse(JSON.stringify(args)):args});const state=window.__mainlineMock;
 if(command==='get_sector_mainline'){if(state.holdResearch){state.researchPending=true;await new Promise(resolve=>state.finishResearch=resolve);state.researchPending=false;}if(state.fail)throw Error('temporary local read failure');const row=${JSON.stringify(sample)};row.sector_code=args.sectorCode;row.sector_name=args.sectorName;if(args.fingerprint){if(state.expired)throw Error('历史快照未留存或已过期，不能用当前结果代替');Object.assign(row,structuredClone(state.history.find(n=>n.fingerprint===args.fingerprint)?.mainline_snapshot));return row;}row.generated_at=new Date(Date.now()).toISOString();if(state.variant==='scaled-prices'){Object.assign(row.leaders[0],{close:20,ma20:19.6,ma60:18.6,atr:.8});}if(state.variant==='missing-reference')delete row.leaders[0].plan.reference_prices;if(state.variant==='legacy-plan')row.leaders[0].plan={status:'unvalidated_observation'};if(state.variant==='reference-date-mismatch')row.leaders[0].plan.reference_prices.as_of='2026-09-29';if(state.variant==='reference-wrong-scale')row.leaders[0].plan.reference_prices.atr=.8;if(state.variant==='reference-invalid-price')row.leaders[0].plan.reference_prices.close=Infinity;if(state.variant==='reference-expired')row.generated_at='2026-10-03T04:00:00Z';if(state.variant==='reference-future-time')row.generated_at='2026-10-05T05:00:00Z';if(state.variant==='incomplete'){row.complete=false;row.strong=false;row.total_members=88;row.covered_members=85;row.trending_members=51;row.status='数据覆盖或时点不完整，停止提醒';row.missing_members=[{symbol:'sh600009',reason:'missing daily quote'}];}if(state.variant==='incomplete-empty'){row.complete=false;row.strong=false;row.covered_members=9;row.missing_members=[{symbol:'sh600009',reason:'missing daily quote'}];row.leaders=[];}if(state.variant==='complete-empty'){row.strong=false;row.leaders=[];}if(state.variant==='sector-not-strong'){row.strong=false;row.trend_breadth=.4;}if(state.variant==='date-mismatch')row.member_as_of='2026-09-29';if(state.variant==='invalid-date'){row.as_of=row.member_as_of=row.benchmark.as_of=row.leaders[0].as_of='2026-02-30';}if(state.variant==='mixed-leaders'){const valid=structuredClone(row.leaders[0]);row.leaders=[valid,...[{as_of:'2026-09-29'},{as_of:'2026-02-30'},{rs20_vs_hs300:0},{rs60_vs_hs300:-1},{atr:Infinity},{close:20},{strong:false},{ma60:10},{r20:12}].map((overrides,i)=>({...structuredClone(valid),symbol:'sh6000'+String(i+2).padStart(2,'0'),name:'不合格个股'+(i+2),...overrides}))];}if(state.variant==='leader-stale')row.leaders[0].as_of='2026-09-29';if(state.variant==='model-stale'){row.leaders[0].model_research.date_matches=false;row.leaders[0].model_research.models.forEach(m=>{m.signal_status='date_mismatch';m.score=null;m.score_as_of=null;m.observation=null;});}if(state.variant==='model-forward-current'){row.as_of='2026-10-08';row.member_as_of='2026-10-08';row.benchmark.as_of='2026-10-08';row.leaders[0].as_of='2026-10-08';row.leaders[0].model_research.date_matches=false;row.leaders[0].model_research.models.forEach(m=>{m.score_as_of='2026-10-08';});}if(state.variant.startsWith('model-legacy')){row.leaders[0].model_research.models.forEach(m=>{delete m.score_as_of;});if(state.variant==='model-legacy-stale')row.leaders[0].model_research.date_matches=false;}if(state.variant==='model-unknown'){row.leaders[0].model_research.models.forEach(m=>{m.signal_status='not_in_scored_export';m.score=null;m.observation=null;});}if(['many-leaders','leader-limit'].includes(state.variant))row.leaders=Array.from({length:state.variant==='leader-limit'?12:5},(_,i)=>({...structuredClone(row.leaders[0]),symbol:'sh60000'+(i+1),name:'测试强股'+(i+1),score:10-i}));if(state.variant==='calendar-zero')row.metrics.removed_non_trading_rows=0;if(state.variant==='calendar-missing')delete row.metrics.removed_non_trading_rows;if(state.variant==='calendar-invalid')row.metrics.removed_non_trading_rows='3';if(state.variant==='benchmark-stale')row.benchmark.as_of='2026-09-29';if(state.variant==='missing-extra'){delete row.metrics.rs20_vs_hs300;delete row.metrics.rs60_vs_hs300;delete row.trend_breadth;delete row.trending_members;}return row;}
 if(command==='get_mainline_watchlist'){if(state.watchFail)throw Error('watchlist unavailable');return state.watched?[{kind:'industry',code:'BK0001',name:'测试主线'}]:[];}
 if(command==='set_mainline_watch'){state.watched=args.enabled;return [];}
 if(command==='analyze_mainline'){await new Promise(r=>setTimeout(r,100));return {overview:'仅汇总本次证据',positives:[{text:'有原文支持',source_ids:['news:1'],evidence:'公司公告原文'}],negatives:[{text:'无原文的断言',source_ids:['missing'],evidence:'不存在的引文'}],uncertainties:['历史机制尚未验证']};}
 if(command==='plugin:opener|open_url')return;
 if(command==='get_mainline_discovery_status'){if(state.holdDiscovery){state.discoveryPending=true;await new Promise(resolve=>state.finishDiscovery=resolve);state.discoveryPending=false;}if(state.fail)throw Error('discovery read failure');return structuredClone(state.discovery);}
 if(command==='set_mainline_discovery_enabled'){clearTimeout(state.scanTimer);state.discovery.enabled=args.enabled;if(!args.enabled)state.discovery.busy=false;return structuredClone(state.discovery);}
 if(command==='run_mainline_discovery'){
  if(state.discovery.finished&&(state.discovery.failed.length||state.discovery.catalog_errors?.length))Object.assign(state.discovery,{finished:false,retry_total:state.discovery.failed.length,retry_processed:0});
  Object.assign(state.discovery,{enabled:true,busy:true});const response=structuredClone(state.discovery);
  if(state.holdStart){state.startPending=true;await new Promise(resolve=>state.finishStart=resolve);state.startPending=false;state.startReplies++;return response;}
  if(!state.holdBackground)state.scanTimer=setTimeout(()=>{if(state.discovery.enabled)Object.assign(state.discovery,{busy:false,processed:2,failed:[{code:'BK0002',name:'失败板块',reason:'日线截止日期不一致'}],candidates:[{kind:'industry',code:'BK0001',name:'测试主线',as_of:'2026-09-30',metrics:{r5:5.2,r20:12.3,r60:20.4,rs20_vs_hs300:3.5}}]});},100);
  return response;
 }
 if(command==='take_pending_navigation')return null;
 if(command==='get_sector_summaries'){const items=[];for(const row of ${JSON.stringify(sectorRows)})if(row.kind===args.kind&&(!args.keyword||row.name.includes(args.keyword)||row.code.includes(args.keyword)))items.push(row);return {kind:args.kind,page:args.page,page_size:args.pageSize,total:items.length,as_of:'2026-09-30',source:'mock readonly catalog',stale:false,items};}
 if(command==='get_sector_members')return {kind:args.kind,sector_code:args.sectorCode,page:args.page,page_size:args.pageSize,total:1,as_of:'2026-09-30',source:'mock readonly members',stale:false,items:[${JSON.stringify(sectorMember)}]};
 if(command==='get_sector_limit_up_stats')return {sector_code:args.sectorCode,limit_up_count:2,member_count:10,as_of:'2026-09-30',source:'mock derived count',methodology:'isolated fixture'};
 if(command==='get_sector_rotation')return {items:${JSON.stringify(sectorRows)},statuses:[{kind:'industry',ok:true,error:null,as_of:'2026-09-30'},{kind:'concept',ok:true,error:null,as_of:'2026-09-30'}],source:'mock readonly rotation'};
 if(command==='get_sector_history')return {sector_code:args.sectorCode,period:args.period,as_of:'2026-09-30',source:'mock readonly '+args.period,items:${JSON.stringify(sectorBars)}};
 if(command==='get_mainline_alert_history'){if(state.historyFail)throw Error('archive unavailable');return structuredClone(state.history);}
 if(command==='get_research_evidence')return structuredClone(state.evidence);
 if(command==='import_research_evidence'){if(state.fail)throw Error('schema/hash rejected');state.evidence.run_id='mock imported validated evidence';return structuredClone(state.evidence);}
 throw Error('Unexpected IPC '+command);
}
export async function listen(){return()=>{};}
export async function emit(){}
export async function enable(){}
export async function disable(){}
export async function isEnabled(){return false;}
export async function openUrl(url){calls.push({command:'openUrl',url});}
export async function open(args){calls.push({command:'dialogOpen',args});return window.__mainlineMock.importPath;}
`;

const h=await uiHarness({name:'mainline',entry:"import {createApp,h,reactive} from 'vue';\nimport {createPinia} from 'pinia';\nimport {NMessageProvider} from 'naive-ui';\nimport Mainline from '/src/components/sector/MainlineResearch.vue';\nimport Discovery from '/src/components/sector/MainlineDiscovery.vue';\nimport Evidence from '/src/components/research/ResearchEvidence.vue';\nimport TopBar from '/src/components/layout/TopBar.vue';\nimport '/src/assets/styles/variables.css';\nimport '/src/assets/workspace.css';\nDate.now=()=>Date.parse('2026-10-05T04:00:00Z');\ndocument.documentElement.dataset.theme='light';document.documentElement.dataset.style='modern';\nconst state=reactive({mode:'research'});window.__mainlineView=state;\ncreateApp({render:()=>h(NMessageProvider,()=>state.mode==='evidence'?h(Evidence):state.mode==='discovery'?h(Discovery,{onOpen:row=>window.__mainlineMock.opened=row}):state.mode==='topbar'?h(TopBar):h(Mainline,{kind:'industry',sectorCode:'BK0001',sectorName:'测试主线'}))}).use(createPinia()).mount('#app');",mock,styles:'body{padding:20px;background:#f5f8fc;color:#16263e;font-family:system-ui,sans-serif;font-size:13px} .mainline-research,.mainline-discovery{max-width:1050px;margin:auto}'});
const {evaluate,wait:waitFor}=h;
const clickTarget=async selector=>{
  const point=await evaluate('(()=>{const el=document.querySelector('+JSON.stringify(selector)+');if(!el)throw Error("Missing click target");el.scrollIntoView({block:"center"});const r=el.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()');
  await h.call('Input.dispatchMouseEvent',{type:'mousePressed',...point,button:'left',clickCount:1});
  await h.call('Input.dispatchMouseEvent',{type:'mouseReleased',...point,button:'left',clickCount:1});
};
const clickScan=()=>clickTarget('.discovery-scan-control');
const setCost=async value=>evaluate(`(()=>{const input=document.querySelector('.mainline-cost-input');input.value=${JSON.stringify(value)};input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
const referenceValues=()=>evaluate(`Object.fromEntries([...document.querySelector('.mainline-reference').querySelectorAll('[data-reference]')].map(el=>[el.dataset.reference,el.innerText]))`);
const preview=async name=>{await evaluate('new Promise(resolve=>setTimeout(resolve,400))');await h.screenshot(name);};
try {
  await h.call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
  await waitFor(`document.querySelector('.mainline-leader') && window.__mainlineMock`);
  assert.equal(await evaluate(`[...document.querySelectorAll('.mainline-evidence,.mainline-sources,.mainline-method,.mainline-reference-toggle')].every(el=>!el.open)`),true,'Professional evidence and plans start folded');
  assert.match(await evaluate(`document.querySelector('.mainline-observation').innerText`),/无需账户.*不会自动买卖.*不会使用自动模型账户/);
  assert.ok(await evaluate(`document.querySelector('.mainline-leader').compareDocumentPosition(document.querySelector('.mainline-evidence')) & Node.DOCUMENT_POSITION_FOLLOWING`),'Leaders precede professional model evidence');
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-plan').length`),0,'Repeated original parameter table was removed');
  assert.equal(await evaluate(`document.querySelector('.mainline-leader').innerText.includes('原始分层计划')`),false);
  assert.equal(await evaluate(`document.body.innerText.includes('我的持仓成本')`),false,'All per-stock strategy/model details start hidden');
  await preview('mainline-compact-default');
  const expandCallsBefore=await evaluate(`window.__mainlineMock.calls.length`);
  await clickTarget('.mainline-reference-toggle>summary');
  await waitFor(`document.querySelector('.mainline-reference-toggle').open`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.length`),expandCallsBefore,'Opening a reference is local and never starts a quote/research/trade request');
  assert.equal(await evaluate(`document.querySelector('.mainline-reference').dataset.referenceState`),'reference','A fresh valid result displays a conditional reference, never an executed signal');
  assert.deepEqual(await referenceValues(),{pullback:'9.70–9.90元',breakout:'10.50–10.70元',weakness:'9.80元',profit:'10.80元',exit:'8.80元'});
  assert.match(await evaluate(`document.querySelector('.mainline-cost-basis').innerText`),/未持仓演示.*收盘 10.00元.*未读取任何账户成本/);
  assert.match(await evaluate(`document.querySelector('.mainline-reference-note').textContent`),/尚未通过多年主线策略验证.*触价不等于买卖信号.*不监测参考点位.*不发送买卖点告警/);
  assert.match(await evaluate(`document.querySelector('.mainline-reference-execution').textContent`),/T\+1.*当日新买入部分不能当天卖出.*停牌.*流动性.*不保证成交/);
  const costCallsBefore=await evaluate(`window.__mainlineMock.calls.length`);
  await setCost('12');await waitFor(`document.querySelector('[data-reference="profit"]').innerText==='12.80元'`);
  assert.deepEqual(await referenceValues(),{pullback:'9.70–9.90元',breakout:'10.50–10.70元',weakness:'9.80元',profit:'12.80元',exit:'10.80元'});
  await clickTarget('.mainline-reference-toggle>summary');
  await waitFor(`!document.querySelector('.mainline-reference-toggle').open`);
  assert.equal(await evaluate(`document.body.innerText.includes('我的持仓成本')`),false);
  await clickTarget('.mainline-reference-toggle>summary');
  await waitFor(`document.querySelector('.mainline-reference-toggle').open`);
  assert.equal(await evaluate(`document.querySelector('.mainline-cost-input').value`),'12','Closing and reopening a stock preserves the user reference cost');
  assert.match(await evaluate(`document.querySelector('.mainline-cost-basis').innerText`),/填写的成本 12.00元.*仅本页参考/);
  for(const cost of ['-1','0','abc','1e309']) {
    await setCost(cost);await waitFor(`document.querySelector('.mainline-cost-input').getAttribute('aria-invalid')==='true'`);
    const values=await referenceValues();assert.equal(values.profit,'--');assert.equal(values.exit,'--');assert.equal(values.pullback,'9.70–9.90元');
  }
  await setCost('0.5');await waitFor(`document.querySelector('.mainline-cost-input').getAttribute('aria-invalid')==='false'`);
  assert.equal((await referenceValues()).exit,'--','A nonpositive ATR exit line is not shown as an executable zero/negative price');
  assert.match(await evaluate(`document.querySelector('.mainline-reference-grid').innerText`),/不大于0.*无法给出有效退出价/);
  await setCost('');await waitFor(`document.querySelector('[data-reference="exit"]').innerText==='8.80元'`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.length`),costCallsBefore,'Cost estimates neither load accounts nor quote services nor create trade/AI IPC');
  await evaluate(`document.querySelector('.mainline-reference').scrollIntoView({block:'start'})`);
  await preview('mainline-reference-default');
  await setCost('12');await waitFor(`document.querySelector('[data-reference="profit"]').innerText==='12.80元'`);
  await evaluate(`document.querySelector('.mainline-reference').scrollIntoView({block:'start'})`);
  await preview('mainline-reference-own-cost');
  await evaluate(`window.__mainlineMock.variant='scaled-prices';document.querySelector('.mainline-refresh-control').click()`);
  await waitFor(`document.querySelector('.mainline-leader').innerText.includes('前复权指标收盘 20.00元')`);
  assert.match(await evaluate(`document.querySelector('.mainline-price-basis').innerText`),/未复权人民币.*收盘 10.00元.*MA20 9.80元.*ATR 0.40元/);
  assert.deepEqual(await referenceValues(),{pullback:'9.70–9.90元',breakout:'10.50–10.70元',weakness:'9.80元',profit:'12.80元',exit:'10.80元'},'Adjusted indicators are mapped to raw prices before combining ATR with a personal cost');
  for(const variant of ['missing-reference','legacy-plan','reference-date-mismatch','reference-wrong-scale','reference-invalid-price']) {
    await evaluate(`window.__mainlineMock.variant=${JSON.stringify(variant)};document.querySelector('.mainline-refresh-control').click()`);
    await waitFor(`document.querySelector('.mainline-reference-unavailable') && !document.querySelector('.mainline-reference')`);
    assert.match(await evaluate(`document.querySelector('.mainline-reference-unavailable').innerText`),/暂无价位参考.*不会用前复权指标直接替代买卖价位/);
    assert.equal(await evaluate(`document.querySelectorAll('[data-reference]').length`),0,'Missing/misaligned reference input never invents monetary price levels');
  }
  for(const variant of ['reference-expired','reference-future-time']) {
    await evaluate(`window.__mainlineMock.variant=${JSON.stringify(variant)};document.querySelector('.mainline-refresh-control').click()`);
    await waitFor(`document.querySelector('.mainline-reference')?.dataset.referenceState==='stale'`);
    assert.doesNotMatch(await evaluate(`document.querySelector('.mainline-reference-state').textContent`),/等待人工确认/,'Expired/future timestamps cannot imply a current recommendation');
  }
  await evaluate(`window.__mainlineMock.variant='complete';document.querySelector('.mainline-refresh-control').click()`);
  await waitFor(`document.querySelector('.mainline-reference')?.dataset.referenceState==='reference'`);
  await setCost('');await waitFor(`document.querySelector('[data-reference="exit"]').innerText==='8.80元'`);
  await h.call('Emulation.setDeviceMetricsOverride',{width:540,height:850,deviceScaleFactor:1,mobile:false});
  const referenceLayout=await evaluate(`(()=>{const root=document.querySelector('.mainline-reference'),cells=[...root.querySelectorAll('.mainline-reference-item')];return {overflow:document.documentElement.scrollWidth>innerWidth+1,clipped:cells.some(el=>el.scrollWidth>el.clientWidth+1),fontSizes:[...root.querySelectorAll('span,label,p,input')].map(el=>parseFloat(getComputedStyle(el).fontSize))};})()`);
  assert.equal(referenceLayout.overflow,false);assert.equal(referenceLayout.clipped,false);assert.ok(referenceLayout.fontSizes.every(size=>size>=12),'Reference conditions and cost input remain readable at narrow width');
  await evaluate(`document.querySelector('.mainline-reference').scrollIntoView({block:'start'})`);
  await preview('mainline-reference-narrow');
  await h.call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
  await evaluate(`document.querySelectorAll('details').forEach(el=>el.open=true)`);
  assert.match(await evaluate(`document.querySelector('.mainline-status').innerText`),/强主线观察/);
  assert.match(await evaluate(`document.querySelector('.mainline-metrics').innerText`),/5.20%/);
  assert.match(await evaluate(`document.querySelector('.mainline-metrics').innerText`),/3.50个百分点/);
  assert.match(await evaluate(`document.querySelector('.mainline-metrics').innerText`),/60.00%/);
  assert.match(await evaluate(`document.querySelector('.mainline-breadth').innerText`),/6 \/ 已覆盖 10/);
  assert.match(await evaluate(`document.querySelector('.mainline-membership').innerText`),/2026-10-01T00:00:00Z.*不等于历史名单生效日期或成分股报价时间/);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-plan').length`),0);
  assert.match(await evaluate(`document.querySelector('.mainline-reference-grid').innerText`),/5–20交易日内复核[\s\S]*上限 20 交易日[\s\S]*未验证观察计划/,'Holding references remain translated without duplicating raw parameters');
  assert.doesNotMatch(await evaluate(`document.querySelector('.mainline-reference-toggle').innerText`),/max_holding_days|review_range_days|reference_cash_cny/);
  assert.equal(await evaluate(`document.querySelector('.mainline-calendar').innerText`),'行情已按真实交易日对齐。');
  assert.match(await evaluate(`document.querySelector('.mainline-models').innerText`),/交集 1 只/);
  assert.match(await evaluate(`document.querySelector('.mainline-models').innerText`),/-5.51%.*[\s\S]*33.00%.*[\s\S]*49.41%/);
  assert.match(await evaluate(`document.querySelector('.mainline-stock-models').innerText`),/1.23%.*[\s\S]*非正分记录.*[\s\S]*-2.34%/);
  assert.match(await evaluate(`document.querySelector('.mainline-news').innerText`),/发布时间晚于行情截点/);
  assert.match(await evaluate(`document.querySelectorAll('.mainline-news')[1].innerText`),/发布 2026-09-30（仅日期，日内时点未知）.*[\s\S]*本机没有完整正文/);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-news img').length`),0,'Original text stays escaped');
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-news .mainline-link').length`),1,'Unsafe source URL is not actionable');
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='analyze_mainline').length`),0,'Opening and scanning never invoke Claude');
  await evaluate(`document.querySelector('.mainline-claude button').click();document.querySelector('.mainline-claude button').click()`);
  await waitFor(`document.querySelector('.mainline-summary')?.innerText.includes('无原文的断言')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='analyze_mainline').length`),1,'Manual analysis keeps one in-flight request');
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-summary a').length`),1,'Only known source references link');
  assert.match(await evaluate(`document.querySelector('.mainline-summary').innerText`),/未引用本次原文，待核实/);
  assert.equal(await evaluate(`document.querySelector('.mainline-summary blockquote').innerText`),'原文摘录：公司公告原文');
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-summary blockquote').length`),1,'Unreferenced evidence is not represented as a sourced quotation');
  await evaluate(`document.querySelector('.mainline-actions button').click()`);
  await waitFor(`document.querySelector('.mainline-actions button').innerText.includes('暂停主线提醒')`);
  await evaluate(`document.querySelector('.mainline-actions button').click()`);
  await waitFor(`document.querySelector('.mainline-actions button').innerText.includes('开启主线提醒')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='set_mainline_watch').length`),2);
  await evaluate(`window.__mainlineMock.variant='model-forward-current';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-stock-models')`);
  await evaluate(`document.querySelectorAll('.mainline-reference-toggle,.mainline-evidence').forEach(el=>el.open=true)`);
  await waitFor(`document.querySelector('.mainline-stock-models')?.innerText.includes('2026-10-08')`);
  assert.match(await evaluate(`document.querySelector('.mainline-models').innerText`),/交集 1 只/,'New-day per-model score remains eligible although old frozen date differs');
  assert.match(await evaluate(`document.querySelector('.mainline-stock-models').innerText`),/1.23%/);
  assert.equal(await evaluate(`document.querySelector('.mainline-stock-models').innerText.includes('冻结日期不同')`),false);
  await evaluate(`window.__mainlineMock.variant='model-legacy';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-stock-models')`);
  await evaluate(`document.querySelectorAll('.mainline-reference-toggle,.mainline-evidence').forEach(el=>el.open=true)`);
  await waitFor(`document.querySelector('.mainline-stock-models')?.innerText.includes('2026-09-30')`);
  assert.match(await evaluate(`document.querySelector('.mainline-models').innerText`),/交集 1 只/,'Legacy missing score date only falls back to matching frozen date');
  await evaluate(`window.__mainlineMock.variant='model-legacy-stale';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-stock-models')`);
  await evaluate(`document.querySelectorAll('.mainline-reference-toggle,.mainline-evidence').forEach(el=>el.open=true)`);
  await waitFor(`document.querySelector('.mainline-stock-models')?.innerText.includes('冻结日期不同')`);
  assert.match(await evaluate(`document.querySelector('.mainline-models').innerText`),/交集 0 只/,'Legacy score without matching frozen date stays unknown');
  await evaluate(`window.__mainlineMock.variant='model-stale';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-stock-models')`);
  await evaluate(`document.querySelectorAll('.mainline-reference-toggle,.mainline-evidence').forEach(el=>el.open=true)`);
  await waitFor(`document.querySelector('.mainline-stock-models')?.innerText.includes('冻结日期不同')`);
  assert.match(await evaluate(`document.querySelector('.mainline-models').innerText`),/交集 0 只/);
  assert.match(await evaluate(`document.querySelector('.mainline-stock-models').innerText`),/当前评分未知 · --/);
  await evaluate(`window.__mainlineMock.variant='model-unknown';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-stock-models')`);
  await evaluate(`document.querySelectorAll('.mainline-reference-toggle,.mainline-evidence').forEach(el=>el.open=true)`);
  await waitFor(`document.querySelector('.mainline-stock-models')?.innerText.includes('评分未知')`);
  assert.match(await evaluate(`document.querySelector('.mainline-models').innerText`),/交集 0 只/);
  for(const variant of ['calendar-zero','calendar-missing','calendar-invalid']){
    await evaluate(`window.__mainlineMock.variant=${JSON.stringify(variant)};document.querySelectorAll('.mainline-actions button')[1].click()`);
    await waitFor(`document.querySelector('.mainline-leader') && !document.querySelector('.mainline-calendar')`);
    assert.equal(await evaluate(`document.querySelector('.mainline-calendar')`),null,'No calendar alignment claim when no valid positive removal count was returned');
  }
  await evaluate(`window.__mainlineMock.variant='incomplete';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-status')?.innerText.includes('覆盖未通过')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),1,'85/86 coverage keeps the independently valid observation candidate');
  assert.match(await evaluate(`document.querySelector('.mainline-cycle').innerText`),/覆盖 85 \/ 86.*缺失 1/);
  assert.match(await evaluate(`document.querySelector('.mainline-leaders-heading').innerText`),/部分成分观察候选/);
  assert.doesNotMatch(await evaluate(`document.querySelector('.mainline-status').innerText`),/强主线观察/);
  assert.match(await evaluate(`document.querySelector('.mainline-observation').innerText`),/覆盖、日期和主线条件全部通过后才发提醒/);
  assert.equal(await evaluate(`document.querySelector('.mainline-reference').dataset.referenceState`),'partial');
  assert.match(await evaluate(`document.querySelector('.mainline-reference-state').textContent`),/个股形态参考.*板块待确认/);
  await preview('mainline-partial-coverage');
  await evaluate(`window.__mainlineMock.variant='mixed-leaders';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelectorAll('.mainline-leader').length===1 && document.querySelector('.mainline-leaders-heading').innerText.includes('部分成分')`);
  assert.match(await evaluate(`document.querySelector('.mainline-leader').innerText`),/测试强股 · sh600001/);
  assert.doesNotMatch(await evaluate(`document.querySelector('.mainline-leader').innerText`),/不合格个股/,'Each stale date, impossible date, nonpositive 20/60-day strength, infinite ATR, extended price, failed trend, reversed MA and sector-lagging return stays excluded');
  await evaluate(`window.__mainlineMock.variant='sector-not-strong';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-status')?.innerText.includes('条件未通过') && document.querySelectorAll('.mainline-leader').length===1`);
  assert.match(await evaluate(`document.querySelector('.mainline-leaders-heading').innerText`),/成分观察候选/,'Individual candidates do not imply a confirmed sector mainline');
  assert.equal(await evaluate(`document.querySelector('.mainline-reference').dataset.referenceState`),'weak');
  assert.match(await evaluate(`document.querySelector('.mainline-reference-state').textContent`),/暂不建议新开仓/);
  for(const variant of ['incomplete-empty','complete-empty']){
    await evaluate(`window.__mainlineMock.variant=${JSON.stringify(variant)};document.querySelectorAll('.mainline-actions button')[1].click()`);
    await waitFor(`document.querySelector('.mainline-empty')`);
    assert.match(await evaluate(`document.querySelector('.mainline-empty').innerText`),/已核验成分中暂无.*20\/60日相对强势.*MA20\/ATR条件/,'Empty list names actual stock conditions');
    assert.equal(await evaluate(`document.querySelector('.mainline-empty').innerText.includes('未覆盖成分尚无结论')`),variant==='incomplete-empty','Missing coverage is explained separately from no eligible stock');
  }
  for(const variant of ['leader-stale','invalid-date']){
    await evaluate(`window.__mainlineMock.variant=${JSON.stringify(variant)};document.querySelectorAll('.mainline-actions button')[1].click()`);
    await waitFor(`document.querySelector('.mainline-status')?.innerText.includes('覆盖未通过') && document.querySelector('.mainline-empty')`);
    assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),0,'Invalid dates never create observation candidates');
    assert.match(await evaluate(`document.querySelector('.mainline-empty').innerText`),variant==='invalid-date'?/日期未通过同日核验/:/未通过同日行情/);
  }
  await evaluate(`window.__mainlineMock.variant='date-mismatch';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-status')?.innerText.includes('覆盖未通过')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),0,'Date mismatch suppresses leaders even if backend strong=true');
  await evaluate(`window.__mainlineMock.variant='benchmark-stale';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-status')?.innerText.includes('覆盖未通过')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),0,'Stale CSI300 benchmark cannot confirm a same-day mainline');
  await evaluate(`window.__mainlineMock.fail=true;document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-error')?.innerText.includes('temporary local read failure')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),0,'Retained stale benchmark evidence cannot expose eligible leaders');
  await evaluate(`window.__mainlineMock.fail=false;window.__mainlineMock.variant='complete';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`!!document.querySelector('.mainline-leader')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='analyze_mainline').length`),1,'Refreshes never auto call Claude');
  const refreshBox=await evaluate(`(()=>{const r=document.querySelector('.mainline-refresh-control').getBoundingClientRect();return {width:r.width,height:r.height}})()`);
  await evaluate(`window.__mainlineMock.holdResearch=true;document.querySelector('.mainline-refresh-control').click()`);
  await waitFor(`window.__mainlineMock.researchPending`);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),1,'Same-sector manual refresh retains the visible leaders while waiting');
  const loadingBox=await evaluate(`(()=>{const r=document.querySelector('.mainline-refresh-control').getBoundingClientRect();return {width:r.width,height:r.height}})()`);
  assert.ok(Math.abs(refreshBox.width-loadingBox.width)<=1 && Math.abs(refreshBox.height-loadingBox.height)<=1,'Research refresh button keeps its loading geometry');
  await evaluate(`window.__mainlineMock.fail=true;window.__mainlineMock.holdResearch=false;window.__mainlineMock.finishResearch()`);
  await waitFor(`document.querySelector('.mainline-error')?.innerText.includes('保留上次结果')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),1,'Failed refresh preserves the old leaders as readonly evidence');
  assert.equal(await evaluate(`document.querySelector('.mainline-actions button').disabled`),true,'Stale retained data cannot change reminders');
  assert.equal(await evaluate(`document.querySelector('.mainline-reference').dataset.referenceState`),'stale');
  assert.match(await evaluate(`document.querySelector('.mainline-reference-state').textContent`),/刷新失败.*仅旧结果参考/);
  await evaluate(`window.__mainlineMock.fail=false;document.querySelector('.mainline-refresh-control').click()`);
  await waitFor(`!document.querySelector('.mainline-error') && !document.querySelector('.mainline-refresh-control').classList.contains('n-button--loading')`);
  await evaluate(`window.__mainlineMock.variant='missing-extra';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-leader')&&document.querySelector('.mainline-metrics')?.innerText.includes('--')`);
  assert.doesNotMatch(await evaluate(`document.querySelector('.mainline-status').innerText`),/强主线观察/,'Missing sector metrics cannot confirm strong mainline but do not erase independent valid stocks');
  assert.match(await evaluate(`document.querySelector('.mainline-metrics').innerText`),/相对沪深300 · 20日\s*--/);
  assert.match(await evaluate(`document.querySelector('.mainline-metrics').innerText`),/内部趋势广度\s*--/);
  await evaluate(`window.__mainlineMock.variant='many-leaders';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelectorAll('.mainline-leader').length===5`);
  assert.equal(await evaluate(`document.querySelector('select[aria-label="个股展示数量"]').value`),'10','Default display is ten, with no invented rows for a shorter result');
  assert.equal(await evaluate(`document.querySelector('.mainline-leaders-heading small').innerText`),'显示 5 / 5 只');
  await evaluate(`window.__mainlineMock.variant='leader-limit';document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelectorAll('.mainline-leader').length===10`);
  await evaluate(`document.querySelectorAll('.mainline-reference-toggle').forEach(el=>el.open=false)`);
  assert.equal(await evaluate(`[...document.querySelectorAll('.mainline-reference-toggle')].every(el=>!el.open)`),true);
  await preview('mainline-compact-ten-stocks');
  const singleOpenCalls=await evaluate(`window.__mainlineMock.calls.length`);
  await clickTarget('.mainline-reference-toggle>summary');
  assert.equal(await evaluate(`[...document.querySelectorAll('.mainline-reference-toggle')].filter(el=>el.open).length`),1,'Clicking one stock reveals only its reference');
  assert.equal(await evaluate(`window.__mainlineMock.calls.length`),singleOpenCalls);
  const countCallsBefore = await evaluate(`window.__mainlineMock.calls.length`);
  const watchedBeforeCount = await evaluate(`window.__mainlineMock.watched`);
  for (const count of [3, 5, 10, 5]) {
    await evaluate(`(()=>{const select=document.querySelector('select[aria-label="个股展示数量"]');select.value=` + JSON.stringify(String(count)) + `;select.dispatchEvent(new Event('change',{bubbles:true}));})()`);
    await waitFor(`document.querySelectorAll('.mainline-leader').length===`+count);
    assert.equal(await evaluate(`localStorage.getItem('mainline-leader-display-count-v1')`),String(count));
  }
  assert.equal(await evaluate(`window.__mainlineMock.calls.length`),countCallsBefore,'Changing display count does not request research, notifications or orders');
  assert.equal(await evaluate(`window.__mainlineMock.watched`),watchedBeforeCount,'Display preference leaves the observation switch unchanged');
  await evaluate(`window.__mainlineView.mode='discovery'`);
  await waitFor(`document.querySelector('.mainline-discovery')`);
  await evaluate(`window.__mainlineView.mode='research'`);
  await waitFor(`document.querySelectorAll('.mainline-leader').length===5`);
  assert.equal(await evaluate(`document.querySelector('select[aria-label="个股展示数量"]').value`),'5','Preference survives closing and reopening the detail');
  assert.equal(await evaluate(`[...document.querySelectorAll('.mainline-reference-toggle')].every(el=>!el.open)`),true,'Reopening the sector starts with compact stock cards');
  await evaluate(`localStorage.setItem('mainline-leader-display-count-v1','100');window.__mainlineView.mode='discovery'`);
  await waitFor(`document.querySelector('.mainline-discovery')`);
  await evaluate(`window.__mainlineView.mode='research'`);
  await waitFor(`document.querySelectorAll('.mainline-leader').length===10`);
  assert.equal(await evaluate(`document.querySelector('select[aria-label="个股展示数量"]').value`),'10','Invalid saved preference falls back to ten');
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),10,'Maximum observation display remains ten, independent of backend length');
  await evaluate(`window.__mainlineMock.watchFail=true;document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-error')?.innerText.includes('watchlist unavailable')`);
  assert.equal(await evaluate(`document.querySelector('.mainline-actions button').disabled`),true,'Failed watchlist cannot be mistaken for disabled reminders');
  await evaluate(`window.__mainlineMock.watchFail=false;window.__mainlineMock.variant='complete'`);
  await evaluate(`window.__mainlineMock.calls.length=0;window.__mainlineMock.fail=false;window.__mainlineView.mode='discovery'`);
  await waitFor(`document.querySelector('.mainline-discovery')&&window.__mainlineMock?.calls.some(c=>c.command==='get_mainline_discovery_status')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='run_mainline_discovery').length`),0,'Opening discovery only reads status');
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-discovery .n-switch').length`),0,'One visible control replaces the folded duplicate scan switch');
  await evaluate(`window.__mainlineMock.holdStart=true`);
  await clickScan();
  await waitFor(`window.__mainlineMock.startPending && document.querySelector('.discovery-scan-control').innerText.includes('暂停')`);
  assert.equal(await evaluate(`document.querySelector('.discovery-scan-control').disabled`),false,'Pause remains clickable while the start request is still pending');
  await clickScan();
  await waitFor(`!window.__mainlineMock.discovery.enabled && document.querySelector('.discovery-scan-control').innerText.includes('继续')`);
  assert.equal(await evaluate(`window.__mainlineMock.discovery.processed`),0,'Pausing pending start preserves its current progress');
  assert.deepEqual(await evaluate(`window.__mainlineMock.calls.find(c=>c.command==='set_mainline_discovery_enabled').args`),{enabled:false});
  await evaluate(`window.__mainlineMock.finishStart()`);
  await waitFor(`window.__mainlineMock.startReplies===1`);
  assert.equal(await evaluate(`document.querySelector('.discovery-scan-control').getAttribute('aria-pressed')`),'false','Late enabled start response cannot overwrite a newer pause');
  assert.match(await evaluate(`document.querySelector('.discovery-progress').innerText`),/已暂停 · 进度保留/);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='run_mainline_discovery').length`),1,'Pending start and pause issue only one start');
  await evaluate(`window.__mainlineMock.holdStart=false`);
  await clickScan();
  await waitFor(`window.__mainlineMock.discovery.processed===2`);
  await clickTarget('.discovery-toolbar .discovery-actions button:last-child');
  await waitFor(`document.querySelector('.discovery-candidates button')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='run_mainline_discovery').length`),2,'Explicit resume starts the background worker once');
  assert.equal(await evaluate(`window.__mainlineMock.calls.some(c=>c.command==='analyze_mainline')`),false,'All-market discovery never calls Claude');
  assert.match(await evaluate(`document.querySelector('.discovery-progress').innerText`),/已处理 2 \/ 3.*[\s\S]*失败 1.*[\s\S]*待处理 1.*[\s\S]*目录尚未完成/);
  assert.match(await evaluate(`document.querySelector('.discovery-failures').innerText`),/不能视为弱势/);
  assert.match(await evaluate(`document.querySelector('.discovery-candidates').innerText`),/5.20%/);
  assert.match(await evaluate(`document.querySelector('.discovery-candidates').innerText`),/3.50个百分点/);
  assert.equal(await evaluate(`document.querySelector('.candidate-observation').innerText`),'入场距离待核验','Legacy data with no distance does not imply entry readiness');
  await evaluate(`window.__mainlineMock.discovery.catalog_errors=['BK catalog failed'];window.__mainlineMock.discovery.candidates[0].code='SW801150';document.querySelectorAll('.discovery-actions button')[1].click()`);
  await waitFor(`document.querySelector('.discovery-candidates').innerText.includes('申万一级行业')`);
  assert.match(await evaluate(`document.querySelector('.mainline-discovery').innerText`),/目录来源失败.*BK catalog failed.*当前扫描范围不完整/);
  await evaluate(`document.querySelector('.discovery-candidates button').click()`);
  assert.deepEqual(await evaluate(`window.__mainlineMock.opened`),{kind:'industry',code:'SW801150',name:'测试主线'});
  await clickScan();
  await waitFor(`!window.__mainlineMock.discovery.enabled && document.querySelector('.discovery-scan-control').innerText.includes('继续')`);
  assert.deepEqual(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='set_mainline_discovery_enabled').at(-1).args`),{enabled:false});
  assert.equal(await evaluate(`window.__mainlineMock.discovery.processed`),2,'Pause retains processed rows and candidates');
  assert.match(await evaluate(`document.querySelector('.discovery-note').innerText`),/关闭本窗口也会继续.*暂停会停止在途扫描并保留进度.*新交易日收盘后自动扫描更新/);
  await evaluate(`window.__mainlineMock.discovery.finished=true;window.__mainlineMock.discovery.processed=3;window.__mainlineMock.discovery.candidates[0].as_of='2026-09-29';document.querySelectorAll('.discovery-actions button')[1].click()`);
  await waitFor(`document.querySelector('.discovery-progress').innerText.includes('目录已处理 · 范围不完整')`);
  assert.equal(await evaluate(`document.querySelector('.discovery-candidates button').disabled`),true,'Mismatched dates cannot open as current candidates');
  await evaluate(`window.__mainlineMock.holdBackground=true;window.__mainlineMock.discovery.candidates[0].as_of='2026-09-30'`);
  await clickScan();
  await waitFor(`window.__mainlineMock.discovery.enabled && document.querySelector('.discovery-progress').innerText.includes('补查失败')`);
  assert.equal(await evaluate(`window.__mainlineMock.discovery.processed`),3,'Continuing a finished failed scan does not reset original processed rows');
  assert.equal(await evaluate(`document.querySelectorAll('.discovery-candidates button').length`),1,'Successful ranking remains visible while only failed sectors retry');
  assert.match(await evaluate(`document.querySelector('.discovery-progress').innerText`),/失败补查 0 [/] 1.*成功结果保留/s);
  await clickScan();
  await waitFor(`!window.__mainlineMock.discovery.enabled`);
  await clickScan();
  await waitFor(`window.__mainlineMock.discovery.enabled`);
  assert.equal(await evaluate(`window.__mainlineMock.discovery.retry_processed`),0,'A second pause/continue retains the failure cursor');
  const quietBox=await evaluate(`(()=>{const r=document.querySelector('.discovery-refresh-control').getBoundingClientRect();return {width:r.width,height:r.height}})()`);
  await evaluate(`window.__mainlineMock.holdDiscovery=true`);
  await waitFor(`window.__mainlineMock.discoveryPending`,12000);
  assert.equal(await evaluate(`document.querySelector('.discovery-refresh-control').classList.contains('n-button--loading')`),false,'Background progress polling does not toggle the visible loading spinner');
  const quietPendingBox=await evaluate(`(()=>{const r=document.querySelector('.discovery-refresh-control').getBoundingClientRect();return {width:r.width,height:r.height}})()`);
  assert.ok(Math.abs(quietBox.width-quietPendingBox.width)<=1 && Math.abs(quietBox.height-quietPendingBox.height)<=1,'Quiet progress read does not resize its button');
  await evaluate(`window.__mainlineMock.holdDiscovery=false;window.__mainlineMock.finishDiscovery()`);
  await waitFor(`!document.querySelector('.discovery-refresh-control').disabled`);
  await clickScan();await waitFor(`!window.__mainlineMock.discovery.enabled`);
  await evaluate(`window.__mainlineMock.fail=true;document.querySelectorAll('.discovery-actions button')[1].click()`);
  await waitFor(`document.querySelector('.discovery-error')?.innerText.includes('discovery read failure')`);
  assert.match(await evaluate(`document.querySelector('.discovery-error').innerText`),/保留上次读取状态/);
  await evaluate(`window.__mainlineMock.calls.length=0;window.__mainlineMock.fail=false;window.__mainlineView.mode='evidence'`);
  await waitFor(`document.querySelectorAll('.evidence tbody tr').length===15`);
  assert.deepEqual(await evaluate(`window.__mainlineMock.calls.map(c=>c.command)`),['get_research_evidence'],'Evidence read never calls summarization or recommendations');
  assert.match(await evaluate(`document.querySelector('.evidence tbody').innerText`),/失败模型完整保留/);
  assert.equal(await evaluate(`document.querySelectorAll('.evidence .negative').length`),15,'Negative training results stay visible');
  await evaluate(`document.querySelectorAll('.evidence tbody tr')[14].click()`);
  assert.match(await evaluate(`document.querySelector('.selected-detail').innerText`),/fit_failed/);
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('.evidence nav button')].map(button=>button.innerText)`),['策略比较','数据覆盖','实验与压力'],'Only useful evidence tabs remain; empty candidate page is removed');
  assert.equal(await evaluate(`window.__mainlineMock.calls.length`),1,'Failed/negative model views do not request buys');
  await evaluate(`document.querySelectorAll('.evidence nav button')[1].click()`);
  assert.equal(await evaluate(`document.querySelectorAll('.evidence img').length`),0,'Data source is escaped');
  assert.match(await evaluate(`document.querySelector('.evidence pre').textContent`),/<img src=x/);
  await evaluate(`document.querySelectorAll('.evidence nav button')[2].click()`);
  assert.equal(await evaluate(`window.__evidenceXss`),undefined,'Schema/JSON code is rendered as text');
  assert.match(await evaluate(`document.querySelector('.evidence pre').textContent`),/<script>/);
  await evaluate(`document.querySelectorAll('.evidence .head>div')[1].querySelectorAll('button')[1].click()`);
  await waitFor(`document.querySelector('.evidence .meta').innerText.includes('mock imported validated evidence')`);
  assert.deepEqual(await evaluate(`window.__mainlineMock.calls.find(c=>c.command==='import_research_evidence').args`),{path:'C:/research/frozen.json'});
  assert.match(await evaluate(`document.querySelector('.evidence').innerText`),/没有创建账户或自动准入策略/);
  await evaluate(`window.__mainlineMock.importPath=null;document.querySelectorAll('.evidence .head>div')[1].querySelectorAll('button')[1].click()`);
  await waitFor(`window.__mainlineMock.calls.filter(c=>c.command==='dialogOpen').length===2`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='import_research_evidence').length`),1,'Canceled picker does not import');
  await evaluate(`window.__mainlineMock.importPath='C:/research/bad.json';window.__mainlineMock.fail=true;document.querySelectorAll('.evidence .head>div')[1].querySelectorAll('button')[1].click()`);
  await waitFor(`document.querySelector('.evidence .error')?.innerText.includes('schema/hash rejected')`);
  assert.match(await evaluate(`document.querySelector('.evidence .meta').innerText`),/mock imported validated evidence/);
  assert.equal(await evaluate(`window.__mainlineMock.calls.some(c=>/summarize|analyze|recommend|buy/.test(c.command))`),false,'Evidence screen never calls AI or trading');

  await evaluate(`window.__mainlineMock.calls.length=0;window.__mainlineMock.fail=false;window.__mainlineMock.variant='complete';window.__mainlineView.mode='topbar';Object.assign(window.__mainlineMock.discovery,{enabled:false,busy:false,processed:3,total:3,failed:[],catalog_errors:[],finished:true,candidates:[{kind:'industry',code:'BK0001',name:'20日最强候选',as_of:'2026-09-30',entry_ready:true,observation_state:'ready',extension_atr:1.5,metrics:{r5:1,r20:9,r60:12,rs20_vs_hs300:8,rs60_vs_hs300:3}},{kind:'industry',code:'SW801150',name:'60日最强候选',as_of:'2026-09-30',entry_ready:false,observation_state:'waiting_pullback',extension_atr:3.5,metrics:{r5:-1,r20:7,r60:18,rs20_vs_hs300:4,rs60_vs_hs300:9}},{kind:'concept',code:'BK0003',name:'日期异常候选',as_of:'2026-09-29',metrics:{rs20_vs_hs300:99,rs60_vs_hs300:99}}]})`);
  await waitFor(`document.querySelector('button[aria-label="打开市场主线"]')`);
  await evaluate(`document.querySelector('button[aria-label="打开市场主线"]').click()`);
  await waitFor(`document.querySelector('.market-mainline-dialog .discovery-candidates button')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.some(c=>c.command==='run_mainline_discovery')`),false,'Topbar opens read-only status, not a new scan');
  assert.equal(await evaluate(`window.__mainlineMock.calls.some(c=>c.command==='get_sector_mainline')`),false,'Mainline detail scans only after explicitly opening a candidate');
  assert.equal(await evaluate(`window.__mainlineMock.calls.some(c=>c.command==='get_mainline_alert_history')`),false,'History is read only when its panel is opened');
  assert.match(await evaluate(`document.querySelector('.discovery-candidates button').innerText`),/20日最强候选/);
  assert.match(await evaluate(`document.querySelector('.discovery-candidates button .candidate-observation').innerText`),/趋势强 · 距离门槛通过/,'Ready candidate passes distance only, not final entry admission');
  assert.match(await evaluate(`document.querySelectorAll('.discovery-candidates button')[1].innerText`),/趋势强 · 等待回落/);
  assert.equal(await evaluate(`document.querySelectorAll('.discovery-candidates button')[1].disabled`),false,'A greater-than-2ATR industry remains visible and clickable for manual research');
  assert.match(await evaluate(`document.querySelector('.discovery-entry-note').innerText`),/强度排名不等于可追涨.*超过2倍ATR.*不能作为当前入场提示.*完整成分、日期和原有条件核验/);
  assert.doesNotMatch(await evaluate(`document.querySelector('.discovery-candidates').innerText`),/当前可入场|立即买入|可追涨/,'No candidate tag claims current entry or purchase eligibility');
  for(const extension of [2,2.01]){
    await evaluate(`(()=>{const row=window.__mainlineMock.discovery.candidates[1];delete row.entry_ready;delete row.observation_state;row.extension_atr=${extension};document.querySelector('.discovery-toolbar .discovery-actions button:last-child').click()})()`);
    await waitFor(`document.querySelectorAll('.candidate-observation')[1]?.innerText===${JSON.stringify(extension===2?'趋势强 · 距离门槛通过':'趋势强 · 等待回落')}`);
    assert.equal(await evaluate(`document.querySelectorAll('.discovery-candidates button')[1].disabled`),false,'Legacy extension-only candidates remain manually researchable across the 2ATR boundary');
  }
  await evaluate(`Object.assign(window.__mainlineMock.discovery.candidates[1],{entry_ready:false,observation_state:'waiting_pullback',extension_atr:3.5});document.querySelector('.discovery-toolbar .discovery-actions button:last-child').click()`);
  await waitFor(`document.querySelectorAll('.candidate-observation')[1]?.innerText==='趋势强 · 等待回落'`);
  assert.equal(await evaluate(`[...document.querySelectorAll('.discovery-candidates button')].at(-1).disabled`),true,'Stale candidate is excluded from current rankings');
  await h.click('近60日');
  assert.match(await evaluate(`document.querySelector('.discovery-candidates button').innerText`),/60日最强候选/);
  assert.match(await evaluate(`document.querySelector('.discovery-candidates button .candidate-observation').innerText`),/趋势强 · 等待回落/,'ATR extension never removes or demotes the relative-strength leader');
  await preview('mainline-waiting-pullback');
  assert.equal(await evaluate(`[...document.querySelectorAll('[aria-pressed="true"]')].some(button=>button.innerText==='近60日')`),true,'Selected period control matches the candidate sorting');
  assert.equal(await evaluate(`window.__mainlineMock.calls.some(c=>c.command==='run_mainline_discovery')`),false,'Period display sorting does not alter frozen rules or launch research');
  await preview('mainline-ranking');
  const catalogReadsBeforeDetail=await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_summaries').length`);
  await clickTarget('.discovery-candidates button');
  await waitFor(`document.querySelector('.mainline-research .mainline-leader')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_summaries').length`),catalogReadsBeforeDetail,'Independent SW detail does not depend on the unrelated BK realtime catalog');
  assert.match(await evaluate(`document.querySelector('.mainline-selected').innerText`),/60日最强候选.*SW801150/);
  assert.deepEqual(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_mainline').at(-1).args`),{kind:'industry',sectorCode:'SW801150',sectorName:'60日最强候选',fingerprint:null});
  await preview('mainline-detail');
  await h.click('返回主线候选排名');
  await waitFor(`!document.querySelector('.mainline-research') && document.querySelector('.discovery-candidates button').innerText.includes('60日最强候选')`);
  await evaluate(`document.querySelector('.mainline-history').open=true`);
  await waitFor(`document.querySelectorAll('.history-row').length===2`);
  assert.match(await evaluate(`document.querySelector('.history-note').innerText`),/当时已经保存.*不.*当前扫描替换.*不能证明当时全市场最强/);
  assert.match(await evaluate(`document.querySelector('.history-row').innerText`),/2026-09-29.*[\s\S]*留存领涨股/);
  assert.match(await evaluate(`document.querySelector('.history-row').innerText`),/与2026-09-28留存相比.*\+2.00个百分点.*非连续日排名变化/);
  assert.match(await evaluate(`document.querySelector('.history-error').innerText`),/1条.*无法回看/);
  await evaluate(`document.querySelector('.history-row button').click()`);
  await waitFor(`document.querySelector('.mainline-archived') && document.querySelector('.mainline-leader')?.innerText.includes('留存领涨股')`);
  assert.match(await evaluate(`document.querySelector('.mainline-status').innerText`),/2026-09-29/);
  assert.equal(await evaluate(`document.querySelector('.mainline-reference-toggle').open`),false,'Archived stock reference starts folded');
  await clickTarget('.mainline-reference-toggle>summary');
  assert.deepEqual(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_mainline').at(-1).args`),{kind:'industry',sectorCode:'BK0001',sectorName:'测试主线',fingerprint:'f'.repeat(64)});
  assert.equal(await evaluate(`document.querySelector('.mainline-reference').dataset.referenceState`),'historical');
  assert.match(await evaluate(`document.querySelector('.mainline-reference-state').textContent`),/历史快照.*仅回看当时价位/);
  assert.match(await evaluate(`document.querySelector('.mainline-price-basis').innerText`),/截止 2026-09-29/);
  assert.equal(await evaluate(`document.querySelector('.mainline-cost-input').value`),'','Opening an archived/other-sector view starts a separate local cost input');
  await preview('mainline-archived');
  await evaluate(`window.__mainlineMock.expired=true;document.querySelectorAll('.mainline-actions button')[1].click()`);
  await waitFor(`document.querySelector('.mainline-error')?.innerText.includes('未留存或已过期')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mainline-leader').length`),0,'Expired snapshot is not replaced by current leaders');
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_mainline').at(-1).args.fingerprint`),'f'.repeat(64),'Retry retains archive identity instead of current scan');
  await evaluate(`window.__mainlineMock.expired=false;window.__mainlineMock.historyFail=true;document.querySelector('.history-toolbar button').click()`);
  await waitFor(`[...document.querySelectorAll('.history-error')].some(e=>e.innerText.includes('archive unavailable'))`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_mainline').at(-1).args.fingerprint`),'f'.repeat(64),'History read errors do not start current research');
  const sectorCallStart=await evaluate(`window.__mainlineMock.calls.length`);
  await evaluate(`document.querySelector('button[aria-label="打开市场板块"]').click()`);
  await waitFor(`document.querySelector('.sector-dialog .n-data-table-tbody')?.innerText.includes('测试行业板块')`);
  assert.equal(await evaluate(`!!document.querySelector('.market-mainline-dialog')`),false,'Opening sector view closes mainline dialog');
  const assertSectorClean=async()=>{
    assert.doesNotMatch(await evaluate(`document.querySelector('.sector-dialog').innerText`),/主线发现|主线研究|寻找市场主线|打开市场主线|在独立市场主线中查看/,'Sector UI has no duplicate mainline tabs or jump cards');
    assert.equal(await evaluate(`document.querySelectorAll('.sector-dialog .mainline-entry,.sector-dialog .mainline-discovery,.sector-dialog .mainline-research').length`),0,'Sector UI embeds no mainline components');
  };
  const clickSectorTab=async text=>evaluate(`[...document.querySelectorAll('.sector-dialog .n-tabs-tab')].find(tab=>tab.innerText.trim()===${JSON.stringify(text)}).click()`);
  const clickSectorButton=async text=>evaluate(`[...document.querySelectorAll('.sector-dialog button')].find(button=>button.innerText.trim()===${JSON.stringify(text)}).click()`);
  await assertSectorClean();
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('.sector-dialog > .n-tabs .n-tabs-tab')].map(tab=>tab.innerText.trim()).filter(Boolean)`),['板块排行','资金流与轮动'],'Only ranking and rotation remain as root tabs');
  assert.match(await evaluate(`document.querySelector('.sector-dialog .n-data-table-thead').innerText`),/板块.*涨跌幅.*成交额.*主力净流入/s);
  await clickSectorTab('概念板块');
  await waitFor(`document.querySelector('.sector-dialog .n-data-table-tbody')?.innerText.includes('测试概念板块')`);
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_summaries').at(-1).args.kind`),'concept');
  await clickSectorTab('行业板块');
  await waitFor(`document.querySelector('.sector-dialog .n-data-table-tbody')?.innerText.includes('测试行业板块')`);
  await evaluate(`(()=>{const input=document.querySelector('.sector-dialog .search input');input.value='BK0001';input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await waitFor(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_summaries').at(-1).args.keyword==='BK0001'`);
  await clickSectorButton('刷新');
  await waitFor(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_summaries').at(-1).args.forceRefresh===true`);
  await evaluate(`(()=>{const input=document.querySelector('.sector-dialog .search input');input.value='';input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await waitFor(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_summaries').at(-1).args.keyword===''`);
  await preview('sector-ranking-clean');

  await clickSectorTab('资金流与轮动');
  await waitFor(`document.querySelector('.quadrant.strong-in')?.innerText.includes('测试行业板块')`);
  assert.match(await evaluate(`document.querySelector('.sector-dialog .meta-line').innerText`),/行业已返回.*概念已返回.*mock readonly rotation/s);
  await assertSectorClean();
  const rotationReadsBefore=await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_rotation').length`);
  await clickSectorButton('近 3 日');
  await waitFor(`document.querySelector('.quadrant.weak-in')?.innerText.includes('测试行业板块')`);
  await clickSectorButton('近 10 日');
  await waitFor(`document.querySelector('.quadrant.strong-in')?.innerText.includes('+4.50%')`);
  await clickSectorTab('概念');
  await waitFor(`document.querySelector('.quadrant.strong-out')?.innerText.includes('测试概念板块')`);
  assert.equal(await evaluate(`document.querySelector('.heat-list').parentElement.parentElement.innerText.includes('测试行业板块')`),false,'Rotation kind filter excludes industry rows from concept view');
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_rotation').length`),rotationReadsBefore,'Rotation period and kind controls reuse loaded data');
  await clickSectorButton('刷新');
  await waitFor(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_rotation').length===${rotationReadsBefore+1}`);
  await clickSectorTab('行业');
  await preview('sector-rotation-clean');

  await clickSectorTab('板块排行');
  await waitFor(`document.querySelector('.sector-dialog .n-data-table-tbody')?.innerText.includes('测试行业板块')`);
  await evaluate(`document.querySelector('.sector-dialog .n-data-table-tbody tr').click()`);
  await waitFor(`document.querySelector('.sector-dialog .n-data-table-tbody')?.innerText.includes('测试板块成分股')`);
  await assertSectorClean();
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('.sector-dialog > .n-tabs .n-tabs-tab')].map(tab=>tab.innerText.trim()).filter(Boolean)`),['成分股','日/周/月 K','资金流详情'],'Member, Kline and fund detail tabs remain');
  assert.match(await evaluate(`document.querySelector('.sector-dialog .summary-line').innerText`),/上涨 8.*下跌 2.*涨停 2 家/s);
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('.sector-dialog .action-cell button')].map(button=>button.innerText.trim())`),['加自选','详情','分析'],'Member actions remain available');
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_members').at(-1).args.sectorCode`),'BK0001');
  await clickSectorButton('刷新成分股');
  await waitFor(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_members').at(-1).args.forceRefresh===true`);
  await preview('sector-members-clean');

  await clickSectorTab('日/周/月 K');
  for(const [period,label] of [['daily','日 K'],['weekly','周 K'],['monthly','月 K']]){
    if(period!=='daily')await clickSectorTab(label);
    await waitFor(`document.querySelector('.sector-kline .chart-meta')?.innerText.includes('mock readonly ${period}') && !document.querySelector('.sector-kline .chart-state')`);
    assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_history').at(-1).args.period`),period);
    assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='get_sector_history').at(-1).args.sectorCode`),'BK0001');
    assert.ok(await evaluate(`document.querySelectorAll('.sector-kline canvas').length`),'Real Kline component creates chart canvases');
    await assertSectorClean();
  }
  await preview('sector-kline-clean');
  await clickSectorTab('资金流详情');
  await waitFor(`document.querySelector('.fund-detail-grid')`);
  assert.match(await evaluate(`document.querySelector('.fund-detail-grid').innerText`),/主力净流入.*\+2.00亿.*占比 \+6.50%.*超大单.*\+1.20亿.*大单.*\+8000.00万.*中单.*-5000.00万.*小单.*-3000.00万/s);
  await assertSectorClean();
  await preview('sector-fund-clean');
  await clickSectorButton('‹ 返回板块列表');
  await waitFor(`document.querySelector('.sector-dialog .search input')`);
  await assertSectorClean();
  const sectorCommands=await evaluate(`window.__mainlineMock.calls.slice(${sectorCallStart}).map(c=>c.command)`);
  assert.equal(sectorCommands.some(command=>/mainline/.test(command)),false,'Ranking, rotation, members, Kline and funds never load or invoke mainline features');
  for(const command of ['get_sector_summaries','get_sector_rotation','get_sector_members','get_sector_limit_up_stats','get_sector_history'])assert.ok(sectorCommands.includes(command),'Real sector UI exercised '+command);

  await evaluate(`document.querySelector('.top-bar-right').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:500,clientY:30}))`);
  await waitFor(`[...document.querySelectorAll('.n-dropdown-option-body')].some(option=>option.innerText.trim()==='市场主线')`);
  await evaluate(`[...document.querySelectorAll('.n-dropdown-option-body')].find(option=>option.innerText.trim()==='市场主线').click()`);
  await waitFor(`document.querySelector('.market-mainline-dialog .mainline-discovery')`);
  assert.equal(await evaluate(`!!document.querySelector('.sector-dialog')`),false,'TopBar quick menu still opens the independent mainline dialog after using sector views');
  await h.call('Emulation.setDeviceMetricsOverride',{width:610,height:800,deviceScaleFactor:1,mobile:false});
  await waitFor(`document.querySelector('.market-mainline-dialog') && innerWidth===610`);
  const overflow=await evaluate(`(()=>{const e=document.querySelector('.market-mainline-dialog');return {width:e.getBoundingClientRect().width,viewport:innerWidth,scroll:e.scrollWidth,client:e.clientWidth}})()`);
  assert.ok(overflow.width<=overflow.viewport && overflow.scroll<=overflow.client+1,'Mainline content stays inside the narrow viewport: '+JSON.stringify(overflow));
  await preview('mainline-narrow');
  const viewCommands=await evaluate(`window.__mainlineMock.calls.map(c=>c.command)`);
  assert.equal(viewCommands.some(c=>/simulation|research_follow|paper|order|buy|trade|analyze_mainline|run_mainline_discovery/.test(c)),false,'Viewing and sorting mainlines never creates/accounts/orders or starts AI/trading');
  await evaluate(`window.__mainlineMock.holdBackground=true`);
  await clickScan();
  await waitFor(`window.__mainlineMock.discovery.enabled && document.querySelector('.discovery-scan-control').innerText.includes('暂停')`);
  const startsBeforeClose=await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='run_mainline_discovery').length`);
  await clickTarget('.market-mainline-dialog .n-base-close');
  await waitFor(`!document.querySelector('.market-mainline-dialog')`);
  assert.equal(await evaluate(`window.__mainlineMock.discovery.enabled`),true,'Closing the actual dialog leaves background scanning enabled');
  await evaluate(`Object.assign(window.__mainlineMock.discovery,{as_of:'2026-10-08',total:4,processed:1,finished:false,busy:true,candidates:[]})`);
  await clickTarget('button[aria-label="打开市场主线"]');
  await waitFor(`document.querySelector('.market-mainline-dialog .discovery-progress')?.innerText.includes('2026-10-08')`);
  assert.match(await evaluate(`document.querySelector('.discovery-progress').innerText`),/已处理 1 \/ 4/,'Reopened dialog reads new-day backend progress');
  assert.equal(await evaluate(`window.__mainlineMock.calls.filter(c=>c.command==='run_mainline_discovery').length`),startsBeforeClose,'Reopening reads status instead of restarting work');
  await clickScan();
  await waitFor(`!window.__mainlineMock.discovery.enabled && document.querySelector('.discovery-scan-control').innerText.includes('继续')`);
  assert.equal(await evaluate(`window.__mainlineMock.discovery.busy`),false,'Pause cancels an already returned background scan');
  assert.equal(await evaluate(`window.__mainlineMock.discovery.processed`),1,'Background pause preserves the current transaction-day progress');
  assert.match(await evaluate(`document.querySelector('.mainline-steps').innerText`),/开始扫描.*点候选查看板块与个股.*按需开启主线提醒/s);
  await preview('mainline-paused');
  const commands=await evaluate(`window.__mainlineMock.calls.map(c=>c.command)`);
  assert.equal(commands.some(c=>/simulation|research_follow|paper|order|buy|trade|analyze_mainline/.test(c)),false,'Only explicitly clicked discovery commands are added; no accounts, trading or AI');
  assert.deepEqual(h.errors,[],'No unhandled browser errors');
  h.save({result:'passed',test_date_beijing:'2026-10-05',cases:['default-folded per-stock MA20/ATR/prior-high references; native click opens only one stock; close/reopen keeps personal cost; no duplicate raw plan table; mapped raw prices; own-cost vs closing-price demonstration; invalid/low cost; missing/misaligned/invalid input has no invented levels; stale/future/failed-refresh/weak/partial/historical contexts; no new IPC; narrow readable cards','existing same-day model evidence retained inside stock references and source regressions','manual watch add/remove and read failure lock','leader default10/select3/5/10/max10; saved preference and invalid fallback; display-only changes never invoke IPC; partial coverage85/86 and per-row date/strength/ATR/trend filters; accurate empty reasons','visible scan start/pause/resume; pause during pending start ignores late replies; actual dialog close/reopen keeps backend work; new-day progress and background pause','discovery partial/failure/catalog/date rules; finished-with-failures resume preserves rankings and cursor; quiet polling stable geometry; same-sector refresh retains readonly evidence','20/60-day display sort without research','greater-than-2ATR waiting-pullback trend remains ranked and opens actual independent detail without trades; distance-ready is not final admission','Chinese max holding20/review5-20 reference labels remain unvalidated; conditional real-trading-day alignment hint','real topbar independent entry/detail/back','real archived snapshot metrics/dates/leaders; no expired-data fallback','independent SW detail without BK realtime catalog dependency','clean sector root/detail UI with no mainline IPC','ranking kind/search/refresh; rotation kind/3/10-day/refresh; members/actions/refresh; daily/weekly/monthly chart canvases; fund detail/back','TopBar quick menu retains independent mainline entry','narrow viewport','no account/trade/AI IPC'],commands,limits:['IPC mocked; no native/remote quotes or notifications tested','current constituents are not historical point-in-time membership','archive accessible through last14-day notification records only, not full historical market ranking','no new model or historical regime algorithm']});
  console.log('Market mainline UI passed: default-folded per-stock raw-price MA20/ATR/prior-high references and personal-cost estimates with readonly context; independent real TopBar and quick menu; clean sector ranking, rotation, members, daily/weekly/monthly Kline and fund views; 20/60-day display ranking; leaders before folded evidence; default10/select3/5/10/max10; partial coverage with independent stock eligibility; visible pause during pending start and background work, late-response guard and actual close/reopen; manual reminders without accounts; real archive identity and expiry; strict coverage/date/model/news/source failures; evidence import/XSS regressions; narrow viewport; zero trading, account or automatic AI commands. '+h.output);
} finally { await h.close(); }
