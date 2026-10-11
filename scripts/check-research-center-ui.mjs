import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';
const mock = `
window.__centerTest={calls:[],enabled:false};
const copy=value=>JSON.parse(JSON.stringify(value));
const run={id:1,model_id:'mean_confirmed_by_median',model_name:'均值＋中位数确认',as_of:'2026-09-30',mode:'forward',comparison:'baseline',holding_days:20,ledger:{metrics:{net_return_pct:0,max_drawdown_pct:0,completed_holding_cycles:0},orders:[]}};
export async function invoke(command,args={}) {
 const state=window.__centerTest;state.calls.push({command,args:copy(args)});
 if(command==='research_model_runs')return {runs:[run]};
 if(command==='research_follow_accounts')return [];
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
 assert.deepEqual(await t.evaluate('[...document.querySelectorAll(".research-nav button")].map(b=>b.innerText)'),['自动模型交易','手动条件提醒','运行记录']);
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
 for(const width of [1280,600,390]) {
  await t.call('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:false});
  assert.ok(await t.evaluate('document.querySelector(".research-scroll").scrollWidth<=document.querySelector(".research-scroll").clientWidth+2'));
  await t.screenshot('records-'+width);
 }
 assert.equal(t.errors.length,0,JSON.stringify(t.errors));
 t.save({status:'passed',checks:['no automatic start on open','six-model automatic switch','manual research removed','compact approved evidence','old ledgers remain readable','1280/600/390 responsive layout'],scope:'Vue with mocked IPC; native package tested separately'});
 console.log('Compact model center UI passed: '+t.output);
}finally{await t.close();}
