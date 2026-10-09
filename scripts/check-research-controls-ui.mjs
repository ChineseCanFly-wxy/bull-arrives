import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';
const mock=`
const state=window.__controlsMock={settings:{alerts_enabled:'0',research_notifications_enabled:'1',theme:'light'},calls:[],broadcasts:[],pending:{},defer:false,saveFails:false,accounts:[],health:null,healthDeferred:false,healthResolve:null};
const account=(id,name,managed='manual')=>({id,name,managed_by:managed,initial_cash:'1000000000',current_cash:String((100000-id*1000)*10000),mode:'auto',auto_enabled:true,manual_source_enabled:true,rule_source_enabled:true,ai_source_enabled:false,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:0,slippage_bps:0});state.accounts=[account(1,'普通甲'),account(2,'普通乙'),account(3,'模型广度','model_follow')];
const clone=v=>JSON.parse(JSON.stringify(v));
const detail=id=>({account:clone(state.accounts.find(a=>a.id===id)),targets:[],positions:[],orders:[],recent_runs:[],metrics:{total_return_bps:id*100},source_stats:[]});
export async function invoke(command,args={}){state.calls.push({command,args:clone(args)});
 if(command==='research_market_data_status'){if(state.healthDeferred)return new Promise(resolve=>state.healthResolve=value=>resolve(clone(value)));return clone(state.health);}
 if(command==='get_settings')return clone(state.settings);
 if(command==='get_stockdb_status')return{enabled:false,enginePath:null,state:'disabled'};
 if(command==='list_datasources')return[['tencent','测试行情']];if(command==='get_portable_mode')return false;
 if(command==='set_setting'){if(state.saveFails)throw Error('fixture save failed');state.settings[args.key]=args.value;return;}
 if(command==='test_notification')return{native:'accepted',desktop:'queued'};
 if(command==='get_notification_identity_status')return{supported:true,registered:false,detail:'测试身份未登记'};
 if(command==='register_notification_identity')return{supported:true,registered:true,detail:'测试身份已登记'};
 if(command==='simulation_list_accounts')return clone(state.accounts);
 if(command==='simulation_get_detail'){if(state.defer)return new Promise(r=>state.pending[args.accountId]=()=>r(detail(args.accountId)));return detail(args.accountId);}
 if(command==='simulation_live_status')return{engine:'realtime_a_share',message:'隔离测试',updated_at:null,plans:[],executions:[]};
 if(command==='simulation_save_account'){const id=args.input.id??state.accounts.length+1;const row={...args.input,id,managed_by:'manual',current_cash:args.input.initial_cash};const index=state.accounts.findIndex(a=>a.id===id);if(index===-1)state.accounts.push(row);else state.accounts[index]=row;return clone(row);}
 throw Error('Unexpected IPC '+command);
}
export async function emit(name,payload){state.broadcasts.push({name,payload});}export async function listen(){return()=>{};}
export async function enable(){}export async function disable(){}export async function isEnabled(){return false;}
`;
const entry=`
import {createApp,h,reactive} from 'vue';import {createPinia} from 'pinia';import {NConfigProvider,NMessageProvider,NModal,darkTheme,lightTheme} from 'naive-ui';
import MarketData from '/src/components/research/ResearchMarketData.vue';import Notifications from '/src/components/research/ResearchNotifications.vue';import Simulation from '/src/components/simulation/SimulationDialog.vue';import {useSettingsStore} from '/src/stores/settings.ts';import '/src/assets/styles/variables.css';
const state=window.__controlsApp=reactive({mode:'notifications',show:true,theme:'light',healthKey:0});const pinia=createPinia();const store=window.__controlsStore=useSettingsStore(pinia);await store.fetchSettings();
createApp({render:()=>h(NConfigProvider,{theme:state.theme==='dark'?darkTheme:lightTheme},()=>h(NMessageProvider,null,()=>state.mode==='notifications'?h(NModal,{show:state.show,preset:'card',style:{width:'min(1170px,calc(100vw - 24px))',overflow:'hidden'}},()=>h(Notifications)):state.mode==='data'?h(MarketData,{key:state.healthKey}):h(Simulation,{ref:instance=>window.__controlsSimulation=instance,show:state.show,'onUpdate:show':v=>state.show=v})))}).use(pinia).mount('#app');
`;
const t=await uiHarness({name:'research-controls-ui',entry,mock});const checks=[];
const check=async(name,fn)=>{await fn();checks.push(name);console.log('PASS '+name);};
try{
 await t.wait('document.querySelector(".research-notification-details")');
 await check('One reminder switch is visible while notification tests stay optional',async()=>{
  assert.equal(await t.evaluate('document.querySelector(".research-notification-details").open'),false);
  assert.equal(await t.evaluate('window.__controlsMock.calls.some(c=>c.command==="test_notification"||c.command==="set_setting")'),false);
  await t.evaluate('document.querySelector(".research-notification-details").open=true');
 });
 await check('Independent research switch stays on while market alerts are off',async()=>{
  assert.equal(await t.evaluate('window.__controlsStore.alertsEnabled'),false);assert.equal(await t.evaluate('document.querySelector(".research-notifications [role=switch]").getAttribute("aria-checked")'),'true');
  await t.evaluate('document.querySelector(".research-notifications [role=switch]").click()');await t.wait('window.__controlsMock.settings.research_notifications_enabled==="0"');
  assert.equal(await t.evaluate('window.__controlsMock.settings.alerts_enabled'),'0');assert.deepEqual(await t.evaluate('window.__controlsMock.broadcasts.at(-1)'),{name:'setting-changed',payload:{key:'research_notifications_enabled',value:'0'}});
  await t.evaluate('window.__controlsStore.applyRemoteSetting("research_notifications_enabled","1")');await t.wait('document.querySelector(".research-notifications [role=switch]").getAttribute("aria-checked")==="true"');
 });
 await check('Research notification scope is explicit and no missing-market-data toggle remains',async()=>{
  const content=await t.evaluate('document.querySelector(".research-notifications").innerText');
  assert.match(content,/统一控制自动成交、手动条件和主线[\s\S]*暂停提醒后[\s\S]*自动买卖和条件检查继续运行/);
  assert.match(content,/信号表示发现候选[\s\S]*委托表示订单已提交[\s\S]*成交提醒[\s\S]*股数与成交价/);
  assert.equal(await t.evaluate('document.querySelectorAll(".research-notifications [role=switch]").length'),1);
  assert.equal(await t.evaluate('document.body.innerText.includes("数据异常提醒")'),false);
 });
 await check('Research test and Windows help work here without starting trades',async()=>{
  await t.click('测试研究提醒');await t.wait('document.querySelector(".research-notifications").innerText.includes("桌面提醒：已排队")');
  assert.deepEqual(await t.evaluate('window.__controlsMock.calls.find(c=>c.command==="test_notification").args'),{research:true});
  await t.click('通知帮助');await t.wait('document.body.innerText.includes("测试身份未登记")');await t.click('启用 Windows 通知');await t.wait('document.body.innerText.includes("测试身份已登记")');
  assert.equal(await t.evaluate('window.__controlsMock.calls.some(c=>/research_follow_|simulation_save_account/.test(c.command))'),false);
 });
 await check('Failed preference save retains the previous switch state and displays the error',async()=>{
  await t.evaluate('window.__controlsMock.saveFails=true;document.querySelector(".research-notifications [role=switch]").click()');await t.wait('document.querySelector(".research-notification-error")');assert.equal(await t.evaluate('document.querySelector(".research-notifications [role=switch]").getAttribute("aria-checked")'),'true');await t.evaluate('window.__controlsMock.saveFails=false');
 });
 await check('Research controls fit narrow and normal windows in both themes',async()=>{
  for(const theme of ['light','dark'])for(const [width,height]of[[1280,900],[390,640]]){await t.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await t.evaluate('window.__controlsApp.theme='+JSON.stringify(theme)+';document.documentElement.dataset.theme='+JSON.stringify(theme));await t.evaluate('new Promise(r=>setTimeout(r,180))');assert.equal(await t.evaluate('(()=>{const s=document.querySelector(".research-notifications");return s.scrollWidth<=s.clientWidth+1;})()'),true);await t.screenshot('notifications-'+theme+'-'+width);}
 });
 await t.call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});await t.evaluate('window.__controlsApp.mode="simulation";window.__controlsApp.theme="light";document.documentElement.dataset.theme="light"');await t.wait('document.querySelector(".metrics")&&window.__controlsMock.calls.some(c=>c.command==="simulation_get_detail")');
 const select=async id=>{await t.evaluate('document.querySelector(".toolbar .n-base-selection").click()');await t.wait('!!document.querySelector(".n-base-select-option")');await t.evaluate('[...document.querySelectorAll(".n-base-select-option")].find(e=>e.innerText.startsWith("#'+id+' " )).click()');};
 await check('Selecting another ordinary account while an older read is in flight resolves to the chosen account',async()=>{
  await t.evaluate('window.__controlsMock.defer=true');await select(2);await t.wait('!!window.__controlsMock.pending[2]');await select(1);await t.evaluate('window.__controlsMock.pending[2]()');await t.wait('!!window.__controlsMock.pending[1]');await t.evaluate('window.__controlsMock.pending[1]();window.__controlsMock.defer=false');await t.wait('window.__controlsSimulation.$.setupState.detail?.account.id===1');assert.match(await t.evaluate('document.querySelector(".metrics").innerText'),/99,?000|99000/);
 });
 await check('Account picker dropdown is readable outside the modal card',async()=>{
  await t.evaluate('document.querySelector(".toolbar .n-base-selection").click()');await t.wait('document.querySelector(".n-base-select-option")');assert.equal(await t.evaluate('(()=>{const e=document.querySelector(".n-base-select-menu");return !!e&&!e.closest(".n-modal")})()'),true);assert.match(await t.evaluate('document.querySelector(".n-base-select-menu").innerText'),/普通甲[\s\S]*普通乙[\s\S]*模型广度/);await t.evaluate('document.querySelectorAll(".n-base-select-option")[1].click()');await t.wait('window.__controlsSimulation.$.setupState.detail?.account.id===2');
 });
 await check('Mode/source explanation is visible and new ordinary accounts save zero transfer/slippage',async()=>{
  await t.click('新建账户');await t.wait('document.querySelector(".simulation-explanation")');assert.match(await t.evaluate('document.querySelector(".simulation-explanation").innerText'),/手动来源[\s\S]*仅记录[\s\S]*需确认[\s\S]*全自动[\s\S]*五个冻结模型/);assert.equal(await t.evaluate('document.querySelectorAll(".n-form-item").length>0&&!document.body.innerText.includes("过户费（基点）")&&!document.body.innerText.includes("滑点（基点）")'),true);await t.click('保存账户');await t.wait('window.__controlsMock.calls.some(c=>c.command==="simulation_save_account")');const input=await t.evaluate('window.__controlsMock.calls.find(c=>c.command==="simulation_save_account").args.input');assert.equal(input.transfer_fee_bps,0);assert.equal(input.slippage_bps,0);await t.wait('window.__controlsSimulation.$.setupState.detail?.account.id===4');
 });
 await check('Model-owned accounts are selectable but editing/manual execution directs users to research tracking',async()=>{
  await select(3);await t.wait('window.__controlsSimulation.$.setupState.detail?.account.id===3');await t.evaluate('window.__controlsSimulation.$.setupState.activeTab="account"');await t.wait('document.body.innerText.includes("此账户由原模型管理")');assert.equal(await t.evaluate('[...document.querySelectorAll("button")].some(b=>b.innerText==="保存账户")'),false);assert.equal(await t.evaluate('[...document.querySelectorAll("button")].find(b=>b.innerText.includes("立即运行"))?.disabled??true'),true);await t.screenshot('model-owned-account');
 });
 await check('Market data panel shows a validated fallback and fitting layouts in both themes',async()=>{
  await t.evaluate('window.__controlsMock.health={state:"ready",expected_as_of:"2026-09-30",enabled_accounts:1,missing_accounts:0,accounts:[{account_id:3,model_name:"模型广度",as_of:"2026-09-30",ready:true}],last_refresh:{mode:"provider_fallback",as_of:"2026-09-30",coverage:4400,required_coverage:4300,providers:{sina:4300,tencent:100}},limitation:"备用只补相邻完成日，跨多日缺口需历史数据"};window.__controlsApp.mode="data"');
  await t.wait('document.querySelector(".research-market-data")?.innerText.includes("已齐备")');await t.evaluate('document.querySelector(".research-market-data details").open=true');
  const dataText=await t.evaluate('document.querySelector(".research-market-data").innerText');
  assert.match(dataText,/近期行情备用[\s\S]*4400/);assert.match(dataText,/09:00[\s\S]*启动[\s\S]*只自动执行一次[\s\S]*不自动重试[\s\S]*重启也不重复/);assert.doesNotMatch(dataText,/每1分钟|连续5次/);
  assert.doesNotMatch(dataText,/16:30与次日交易日|按关键时点提醒/);
  for(const theme of ['light','dark'])for(const [width,height]of[[1280,900],[390,640]]){await t.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await t.evaluate('window.__controlsApp.theme='+JSON.stringify(theme)+';document.documentElement.dataset.theme='+JSON.stringify(theme));await t.evaluate('new Promise(r=>setTimeout(r,120))');assert.equal(await t.evaluate('(()=>{const s=document.querySelector(".research-market-data");return s.scrollWidth<=s.clientWidth+1;})()'),true);await t.screenshot('data-'+theme+'-'+width);}
 });
 await check('Missing data shows required date and actual failure without submitting any order',async()=>{
  const before=await t.evaluate('window.__controlsMock.calls.length');await t.evaluate('window.__controlsMock.health={...window.__controlsMock.health,state:"waiting_data",missing_accounts:1,accounts:[{account_id:3,model_name:"模型广度",as_of:"2026-09-29",ready:false,error:"StockDB未更新；两路备用日期不符"}]};window.__controlsApp.healthKey++');
  await t.wait('document.querySelector(".research-market-data.waiting")?.innerText.includes("正在等待完整数据")');await t.evaluate('document.querySelector(".research-market-data details").open=true');
  assert.match(await t.evaluate('document.querySelector(".research-market-data").innerText'),/应到 2026-09-30[\s\S]*StockDB未更新/);
  assert.deepEqual(await t.evaluate('window.__controlsMock.calls.slice('+before+').map(c=>c.command)'),['research_market_data_status']);
 });
 await check('Closing the data panel while a status request is in flight cannot reopen it or affect trading',async()=>{
  await t.evaluate('window.__controlsMock.healthDeferred=true;window.__controlsApp.healthKey++');await t.wait('!!window.__controlsMock.healthResolve');await t.evaluate('window.__controlsApp.mode="notifications"');await t.wait('!!document.querySelector(".research-notifications")');await t.evaluate('window.__controlsMock.healthResolve(window.__controlsMock.health);window.__controlsMock.healthDeferred=false');await t.evaluate('new Promise(r=>setTimeout(r,100))');assert.equal(await t.evaluate('!!document.querySelector(".research-market-data")'),false);
 });
 assert.deepEqual(t.errors,[]);const report={schema:'research-controls-ui-v1',checks,runtimeErrors:t.errors,ipc:await t.evaluate('window.__controlsMock.calls'),note:'Real Vue components + real Pinia settings store; isolated synthetic IPC, no actual notifications/trades/native application data.'};t.save(report);console.log(JSON.stringify({output:t.output,passed:checks.length},null,2));
}finally{await t.close();}
