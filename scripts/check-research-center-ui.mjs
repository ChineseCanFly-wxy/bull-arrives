import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';
const mock = `
window.__centerTest={calls:[],enabled:false,profitFailure:false};
const copy=value=>JSON.parse(JSON.stringify(value));
const run={id:1,model_id:'mean_confirmed_by_median',model_name:'均值＋中位数确认',as_of:'2026-09-30',mode:'forward',comparison:'baseline',holding_days:20,ledger:{metrics:{net_return_pct:0,max_drawdown_pct:0,completed_holding_cycles:0},orders:[]}};
export async function invoke(command,args={}) {
 const state=window.__centerTest;state.calls.push({command,args:copy(args)});
 if(command==='research_model_runs')return {runs:[run]};
 if(command==='research_follow_accounts')return [];
 if(command==='research_follow_profit_history'){
  if(state.profitFailure)throw Error('隔离历史查询失败');
  const q=args.query;
  const rows=Array.from({length:51},(_,i)=>({order_id:i+1,account_id:i%2+1,model_name:i%2?'另一模型':'测试模型',symbol:i%2?'sz000002':'sh600000',name:i%2?'测试乙':'浦发样例',quantity:100,price_cny:10,cost_cny:1002.5,net_proceeds_cny:994.5,sell_fee_cny:5.5,profit_cny:-8,return_pct:-8/1002.5*100,filled_at:'2026-09-21T17:00:00Z',holding_sessions:2,reason:'原模型3ATR提前退出'})).filter(r=>(!q.account_id||r.account_id===q.account_id)&&(!q.keyword||(r.name+r.symbol).includes(q.keyword))&&(!q.start||q.start<='2026-09-22')&&(!q.end||q.end>='2026-09-22'));
  return {accounts:[{account_id:1,model_name:'测试模型'},{account_id:2,model_name:'另一模型'}],page:q.page,page_size:50,summary:{count:rows.length,profit_cny:rows.length*-8,cost_cny:rows.length*1002.5,net_proceeds_cny:rows.length*994.5,sell_fee_cny:rows.length*5.5,return_pct:rows.length?-8/1002.5*100:null,wins:0},records:rows.slice(q.page*50,q.page*50+50)};
 }
 if(command==='research_auto_status'){if(typeof args.enabled==='boolean')state.enabled=args.enabled;return {enabled:state.enabled,effective_enabled:state.enabled,state:state.enabled?'waiting_data':'paused',message:'隔离状态，仅验证界面',models:[],next_check_at:null,last_completed_day:null};}
 if(command==='set_setting'){if(args.key==='research_auto_trade_enabled')state.enabled=args.value==='1';return;}
 if(command==='get_settings')return {ai_enabled:'1',research_auto_trade_enabled:state.enabled?'1':'0'};
 if(command==='research_job_list')return [{id:7,kind:'scan',state:'complete',phase:'complete',message:'已核验',completed:6,total:6,request:{models:['mean_confirmed_by_median'],comparisons:['baseline']},results:[{key:'test',model_id:run.model_id,run_id:1,metrics:run.ledger.metrics}]}];
 if(command==='research_market_data_status')return {state:'no_active_accounts',expected_as_of:'2026-09-30',enabled_accounts:0,missing_accounts:0,accounts:[],limitation:'样例数据'};
 if(command==='get_model_candidates')return {fresh:false,groups:[],message:'等待数据',as_of:'2026-09-30'};
 if(command==='model_condition_watches')return {watches:[],presets:[],limit_per_tick:20};
 if(command==='get_stockdb_status')return {enabled:false,state:'disabled'};
 if(command==='get_portable_mode')return false;
 if(command==='list_datasources')return [];
 throw Error('Unexpected IPC '+command);
}
export async function emit(){} export async function listen(){return()=>{};}
export async function open(){throw Error('No file picker expected');}
export async function enable(){} export async function disable(){} export async function isEnabled(){return false;}
`;
const entry = `import {createApp} from 'vue';import {createPinia} from 'pinia';import Center from '/src/components/research/ResearchCenter.vue';import '/src/assets/styles/variables.css';document.documentElement.dataset.theme='light';createApp(Center,{show:true}).use(createPinia()).mount('#app');`;
const t=await uiHarness({name:'compact-model-center',entry,mock});
try {
 await t.wait('document.querySelector(".follow-automatic-switch")&&!document.querySelector(".follow-automatic-switch").disabled');
 assert.deepEqual(await t.evaluate('[...document.querySelectorAll(".research-nav button")].map(b=>b.innerText)'),['自动模型交易','手动条件提醒','运行记录','历史盈亏']);
 const writes=()=>t.evaluate('window.__centerTest.calls.filter(row=>/start|discover|setup|import|set_setting/.test(row.command)||typeof row.args.enabled==="boolean")');
 assert.deepEqual(await writes(),[],'opening the page cannot start research or trading');
 assert.equal(await t.evaluate('!!document.querySelector(".center-mode,.research-paths,.mode-grid")'),false);
 await t.click('开启自动运行');await t.wait('document.querySelector(".follow-automatic-switch").getAttribute("aria-checked")==="true"');
 assert.equal((await writes()).filter(row=>row.command==='research_auto_status'&&row.args.enabled===true).length,1);
 await t.click('运行记录');await t.wait('document.querySelector(".research-background-jobs")?.innerText.includes("6/6")');
 await t.evaluate('document.querySelectorAll(".research-background-jobs details").forEach(item=>item.open=true)');
 await t.click('查看账本');await t.wait('document.querySelector(".saved-runs")');
 await t.evaluate('document.querySelector(".approved-model-results").open=true');
 assert.match(await t.evaluate('document.querySelector(".approved-model-results").innerText'),/均值＋中位数确认[\s\S]*6%仓位/);
 assert.doesNotMatch(await t.evaluate('document.body.innerText'),/深入研究|运行真实多年回放|导入研究|配置研究路径|选择研究方向/);
 await t.click('历史盈亏');await t.wait('document.querySelectorAll(".profit-table tbody tr").length===50');
 assert.match(await t.evaluate('document.querySelector(".profit-summary").innerText'),/-408\.00[\s\S]*51 笔/);
 assert.match(await t.evaluate('document.querySelector(".profit-table").innerText'),/2026\/9\/22 01:00:00/);
 const profitCalls=()=>t.evaluate('window.__centerTest.calls.filter(c=>c.command==="research_follow_profit_history")');
 const before=(await profitCalls()).length;
 await t.evaluate('const input=document.querySelector("[data-profit-keyword]");input.value="600000";input.dispatchEvent(new Event("input",{bubbles:true}));');
 assert.equal((await profitCalls()).length,before,'editing filters cannot fetch');
 await t.click('下一页');await t.wait('document.querySelectorAll(".profit-table tbody tr").length===1');
 assert.equal((await profitCalls()).at(-1).args.query.keyword,'','pagination uses the applied query');
 await t.evaluate('const start=document.querySelector("[data-profit-start]"),end=document.querySelector("[data-profit-end]");for(const el of [start,end]){el.value="2026-09-22";el.dispatchEvent(new Event("input",{bubbles:true}));}');
 await t.click('查询盈亏');await t.wait('document.querySelectorAll(".profit-table tbody tr").length===26');
 assert.deepEqual((await profitCalls()).at(-1).args.query,{account_id:null,keyword:'600000',start:'2026-09-22',end:'2026-09-22',page:0});
 assert.match(await t.evaluate('document.querySelector(".profit-period").innerText'),/2026-09-22.*600000/);
 await t.evaluate('window.__centerTest.profitFailure=true');await t.click('查询盈亏');await t.wait('document.querySelector(".profit-history .error")?.innerText.includes("隔离历史查询失败")');
 assert.equal(await t.evaluate('document.querySelectorAll(".profit-table tbody tr").length'),26);
 await t.evaluate('window.__centerTest.profitFailure=false');
 await t.evaluate('(()=>{const select=document.querySelector("[data-profit-account]");select.value="2";select.dispatchEvent(new Event("change",{bubbles:true}));})()');
 await t.click('查询盈亏');await t.wait('document.querySelector(".profit-empty")');
 assert.match(await t.evaluate('document.querySelector(".profit-summary").innerText'),/0 笔/);
 await t.evaluate('(()=>{const select=document.querySelector("[data-profit-account]");select.value="1";select.dispatchEvent(new Event("change",{bubbles:true}));})()');
 await t.click('查询盈亏');await t.wait('document.querySelectorAll(".profit-table tbody tr").length===26&&!document.querySelector(".profit-history .error")');
 for(const width of [1280,600,390]){
  await t.call('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:false});
  assert.ok(await t.evaluate('document.querySelector(".research-scroll").scrollWidth<=document.querySelector(".research-scroll").clientWidth+2'));
  await t.screenshot('profit-history-'+width);
 }
 await t.click('返回自动模型交易');await t.wait('document.querySelector(".follow-automatic-switch")');
 await t.click('运行记录');await t.wait('document.querySelector(".research-background-jobs")');
 for(const width of [1280,600,390]) {
  await t.call('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:false});
  assert.ok(await t.evaluate('document.querySelector(".research-scroll").scrollWidth<=document.querySelector(".research-scroll").clientWidth+2'));
  await t.screenshot('records-'+width);
 }
 assert.equal(t.errors.length,0,JSON.stringify(t.errors));
 t.save({status:'passed',checks:['no automatic start on open','six-model automatic switch','manual research removed','compact approved evidence','old ledgers remain readable','historical PnL inner page and back navigation','date and stock filters fetch on query only','pagination preserves applied filters','failed query preserves results','1280/600/390 responsive layout'],scope:'Vue with mocked IPC; no native package validation in this check'});
 console.log('Compact model center UI passed: '+t.output);
}finally{await t.close();}
