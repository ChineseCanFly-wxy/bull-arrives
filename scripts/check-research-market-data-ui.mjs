import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';
const mock=`
const state=window.__controlsMock={settings:{theme:'light',local_history_enabled:'1',local_history_auto_update_enabled:'1'},calls:[],health:null,healthDeferred:false,healthResolve:null};
const clone=value=>JSON.parse(JSON.stringify(value));
export async function invoke(command,args={}){state.calls.push({command,args:clone(args)});
 if(command==='get_settings')return clone(state.settings);
 if(command==='get_stockdb_status')return {enabled:true,platformSupported:true,state:'error',busy:false,enginePath:null,updaterPath:null};
 if(command==='list_datasources')return [['tencent','隔离行情']];if(command==='get_portable_mode')return false;
 if(command==='research_market_data_status'){if(state.healthDeferred)return new Promise(resolve=>state.healthResolve=value=>resolve(clone(value)));return clone(state.health);}
 throw Error('Unexpected IPC '+command);
}
export async function emit(){}export async function listen(){return()=>{};}
export async function enable(){}export async function disable(){}export async function isEnabled(){return false;}
`;
const entry=`
import {createApp,h,reactive} from 'vue';import {createPinia} from 'pinia';import {NConfigProvider,NMessageProvider,darkTheme,lightTheme} from 'naive-ui';
import MarketData from '/src/components/research/ResearchMarketData.vue';import {useSettingsStore} from '/src/stores/settings.ts';import '/src/assets/styles/variables.css';
const state=window.__controlsApp=reactive({mode:'idle',theme:'light',healthKey:0});const pinia=createPinia();const store=useSettingsStore(pinia);await store.fetchSettings();
store.stockDbStatus={enabled:true,platformSupported:true,state:'error',busy:false,enginePath:null,updaterPath:null,autoUpdate:{enabled:true,time:'09:00',timezone:'Asia/Shanghai',state:'failed',message:'今天已执行一次，失败后不自动重试',failures:1,lastError:'隔离更新失败原因'}};
createApp({render:()=>h(NConfigProvider,{theme:state.theme==='dark'?darkTheme:lightTheme},()=>h(NMessageProvider,null,()=>state.mode==='data'?h(MarketData,{key:state.healthKey}):h('div')))}).use(pinia).mount('#app');
`;
const t=await uiHarness({name:'research-market-data-ui',entry,mock});const checks=[];
const check=async(name,fn)=>{await fn();checks.push(name);console.log('PASS '+name);};
try{
 await t.wait('!!window.__controlsApp');
 await check('Market data panel shows a validated fallback and fitting layouts in both themes',async()=>{
  await t.evaluate('window.__controlsMock.health={state:"ready",expected_as_of:"2026-09-30",enabled_accounts:1,missing_accounts:0,accounts:[{account_id:3,model_name:"模型广度",as_of:"2026-09-30",ready:true}],last_refresh:{mode:"provider_fallback",as_of:"2026-09-30",coverage:4400,required_coverage:4300,providers:{sina:4300,tencent:100}},limitation:"备用只补相邻完成日，跨多日缺口需历史数据"};window.__controlsApp.mode="data"');
  await t.wait('document.querySelector(".research-market-data")?.innerText.includes("已齐备")');await t.evaluate('document.querySelector(".research-market-data details").open=true');
  const dataText=await t.evaluate('document.querySelector(".research-market-data").innerText');
  assert.match(dataText,/近期行情备用[\s\S]*4400/);assert.match(dataText,/09:00[\s\S]*启动[\s\S]*只自动执行一次[\s\S]*不自动重试[\s\S]*重启也不重复/);assert.doesNotMatch(dataText,/每1分钟|连续5次/);
  assert.doesNotMatch(dataText,/16:30与次日交易日|按关键时点提醒/);assert.match(dataText,/最近更新失败原因：隔离更新失败原因/);
  for(const theme of ['light','dark'])for(const [width,height]of[[1280,900],[390,640]]){await t.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await t.evaluate('window.__controlsApp.theme='+JSON.stringify(theme)+';document.documentElement.dataset.theme='+JSON.stringify(theme));await t.evaluate('new Promise(r=>setTimeout(r,120))');assert.equal(await t.evaluate('(()=>{const s=document.querySelector(".research-market-data");return s.scrollWidth<=s.clientWidth+1;})()'),true);await t.screenshot('data-'+theme+'-'+width);}
 });
 await check('Missing data shows required date and actual failure without submitting any order',async()=>{
  const before=await t.evaluate('window.__controlsMock.calls.length');await t.evaluate('window.__controlsMock.health={...window.__controlsMock.health,state:"waiting_data",missing_accounts:1,accounts:[{account_id:3,model_name:"模型广度",as_of:"2026-09-29",ready:false,error:"StockDB未更新；两路备用日期不符"}]};window.__controlsApp.healthKey++');
  await t.wait('document.querySelector(".research-market-data.waiting")?.innerText.includes("正在等待完整数据")');await t.evaluate('document.querySelector(".research-market-data details").open=true');
  assert.match(await t.evaluate('document.querySelector(".research-market-data").innerText'),/应到 2026-09-30[\s\S]*StockDB未更新/);
  assert.deepEqual(await t.evaluate('window.__controlsMock.calls.slice('+before+').map(c=>c.command)'),['research_market_data_status']);
 });
 await check('Closing the data panel while a status request is in flight cannot reopen it or affect trading',async()=>{
  await t.evaluate('window.__controlsMock.healthDeferred=true;window.__controlsApp.healthKey++');await t.wait('!!window.__controlsMock.healthResolve');await t.evaluate('window.__controlsApp.mode="notifications"');await t.wait('!document.querySelector(".research-market-data")');await t.evaluate('window.__controlsMock.healthResolve(window.__controlsMock.health);window.__controlsMock.healthDeferred=false');await t.evaluate('new Promise(r=>setTimeout(r,100))');assert.equal(await t.evaluate('!!document.querySelector(".research-market-data")'),false);
 });
 assert.deepEqual(t.errors,[]);assert.ok((await t.evaluate('window.__controlsMock.calls')).every(call=>['get_settings','get_stockdb_status','list_datasources','get_portable_mode','research_market_data_status'].includes(call.command)),'Displaying data status never launches an update or submits orders');const report={schema:'research-controls-ui-v1',checks,runtimeErrors:t.errors,ipc:await t.evaluate('window.__controlsMock.calls'),note:'Real Vue components + real Pinia settings store; isolated synthetic IPC, no actual notifications/trades/native application data.'};t.save(report);console.log(JSON.stringify({output:t.output,passed:checks.length},null,2));
}finally{await t.close();}
