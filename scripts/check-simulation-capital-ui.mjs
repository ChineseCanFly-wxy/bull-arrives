import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';
const mock="\nconst s=window.__capitalMock={calls:[],failAdjust:false,legacyDetail:true,accounts:[],adjustments:{},maxima:{1:2,4:5},preset:{initial_cash_cny:100000},failPreset:false,globalPaused:false,deferred:[],deferDetailId:null,failDetailId:null,executionExtra:{}};\nconst base=(id,kind)=>({id,name:kind==='model_follow'?'原模型'+id:kind==='research'?'冻结研究':'独立手动',managed_by:kind,initial_cash:'1000000000',current_cash:'1000000000',mode:'auto',auto_enabled:true,manual_source_enabled:kind==='manual',rule_source_enabled:kind!=='research',ai_source_enabled:false,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:0,slippage_bps:0});s.accounts=[base(1,'model_follow'),base(2,'manual'),base(3,'research'),base(4,'model_follow')];\nconst clone=x=>JSON.parse(JSON.stringify(x));\nconst positions=a=>[{symbol:a.id===4?'sz000002':'sh600000',name:a.id===4?'另一模型持仓':'固定持仓',quantity:a.id===4?300:100,available_quantity:100,cost_price:'100000'}];\nconst follow=a=>({account_id:a.id,source_run_id:100+a.id,model_id:a.id===4?'breadth22_rank20':'breadth22_h20',model_name:a.name,max_positions:s.maxima[a.id],enabled:a.auto_enabled,...(s.globalPaused?{effective_enabled:false}:{}),as_of:'20260930',initial_cash_cny:Number(a.initial_cash)/10000,cash_cny:Number(a.current_cash)/10000,equity_cny:Number(a.current_cash)/10000+1000,net_return_pct:1,state:a.auto_enabled?'waiting_new_data':'paused',message:'已复用原模型，等待新交易日',source_sha256:'a'.repeat(64),positions:positions(a).map(p=>({...p,cost_cny:10,mark_cny:10,mark_at:'2026-09-30T07:00:00Z',entry_date:'20260930',holding_sessions:1,exit_reason:null})),orders:[],candidates:[],execution:{position_pct:a.id===4?10:8,total_entry_pct:a.id===4&&s.maxima[a.id]===5?50:s.maxima[a.id]*8,holding_days:20,max_gap_pct:5,buy_window:'09:30–09:35',entry_note:'本模型入场条件',exit_note:'20日上限 / 3ATR',allocation_note:'后台已验证配置',fee_note:'过户费0，滑点0',...s.executionExtra},comparison_note:'历史研究不代表本账户',performance:{filled_buys:1,filled_sells:0,cancelled:0,expired:0,confirmation_delay_seconds:null}});\nexport async function invoke(command,args={}){s.calls.push({command,args:clone(args)});\n if(command==='simulation_auto_preset')return clone(s.preset);\n if(command==='simulation_save_auto_preset'){if(s.failPreset)throw Error('隔离预设保存失败');s.preset=clone(args.input);return clone(s.preset);}\n if(command==='research_follow_accounts')return clone(s.accounts.filter(a=>a.managed_by==='model_follow').map(follow));\n if(command==='research_follow_update'){const a=s.accounts.find(a=>a.id===args.accountId);if(a?.managed_by!=='model_follow')throw Error('自动专用绑定');if(args.maxPositions!==null){if(!Number.isInteger(args.maxPositions)||args.maxPositions<1||args.maxPositions>10)throw Error('持股上限1至10');s.maxima[a.id]=args.maxPositions;}if(args.enabled!==null)a.auto_enabled=args.enabled;return clone(follow(a));}\n if(command==='simulation_list_accounts')return clone([s.accounts[1],...s.accounts.filter(a=>a.id!==2)]);\n if(command==='simulation_live_status')return{engine:'daily_raw_open',message:'isolated fixture',updated_at:null,plans:[],executions:[]};\n if(command==='simulation_get_detail'){if(s.deferDetailId===args.accountId){s.deferDetailId=null;await new Promise(r=>s.deferred.push(r));}if(s.failDetailId===args.accountId){s.failDetailId=null;throw Error('隔离定时明细读取失败');}const a=clone(s.accounts.find(x=>x.id===args.accountId)),p=positions(a);if(s.legacyDetail)delete a.managed_by;return{account:a,targets:[],positions:p,orders:[{id:10+a.id,symbol:p[0].symbol,side:'buy',quantity:100,signal_date:'2026-09-30',status:'awaiting_confirmation'}],recent_runs:[],metrics:{total_return_bps:0},source_stats:[],capital_adjustments:clone(s.adjustments[a.id]||[]),performance_note:'收益按最新初始资金基准；资金增减不计作盈利'};}\n if(command==='simulation_adjust_capital'){if(s.failAdjust)throw Error('减少后现金不足以覆盖未成交买单');const a=s.accounts.find(x=>x.id===args.input.accountId),old=BigInt(a.initial_cash),next=BigInt(Math.round(args.input.initialCash*100))*100n,cash=BigInt(a.current_cash);a.initial_cash=String(next);a.current_cash=String(cash+next-old);(s.adjustments[a.id]??=[]).push({id:s.calls.length,old_initial_cash:String(old),new_initial_cash:String(next),delta:String(next-old),cash_before:String(cash),cash_after:a.current_cash,created_at:'2026-10-04T08:00:00Z'});return clone(a);}\n if(command==='simulation_save_account'){const old=s.accounts.find(x=>x.id===args.input.id);if(old?.managed_by!=='manual')throw Error('自动模型专用');const next=BigInt(args.input.initial_cash);old.current_cash=String(BigInt(old.current_cash)+next-BigInt(old.initial_cash));Object.assign(old,args.input);return clone(old);}\n throw Error('Unexpected IPC '+command);\n}\n";
const entry="\nimport {createApp,h,reactive} from 'vue';import {NConfigProvider,NMessageProvider,darkTheme,lightTheme} from 'naive-ui';\nimport Simulation from '/src/components/simulation/SimulationDialog.vue';import '/src/assets/styles/variables.css';import '/src/assets/workspace.css';\nconst app=window.__capitalApp=reactive({show:true,theme:'light'});\ncreateApp({render:()=>h(NConfigProvider,{theme:app.theme==='dark'?darkTheme:lightTheme},()=>h(NMessageProvider,null,()=>h(Simulation,{ref:c=>window.__capitalComponent=c,show:app.show,'onUpdate:show':v=>app.show=v})))}).mount('#app');\n";

const t=await uiHarness({name:'simulation-capital',entry,mock});const results=[];let refreshGeometry;const secondaryFontSizes=[];
const check=async(name,fn)=>{await fn();results.push(name);console.log('PASS '+name);};
const select=async id=>{await t.evaluate('window.__capitalComponent.$.setupState.selectedId='+id);await t.wait('window.__capitalComponent.$.setupState.detail?.account.id==='+id);};
try{
 await t.wait('document.querySelector(".metrics")&&window.__capitalComponent.$.setupState.detail?.account.id===1');

 await check('Timed refresh keeps button/modal geometry, current detail and last data on failure; explicit refresh uses a stable spinner',async()=>{
  await t.evaluate('document.fonts.ready.then(()=>new Promise(r=>setTimeout(r,400)))');
  await t.evaluate('(()=>{const c=window.__capitalComponent.$.setupState,a=window.__capitalRefreshAudit={samples:[],modal:[],problems:[],quiet:true,detail:c.detail,positions:JSON.stringify(c.detail.positions),node:document.querySelector(".table-wrap")};a.sample=()=>{const b=document.querySelector(".simulation-refresh"),r=b.getBoundingClientRect(),m=document.querySelector(".dialog").getBoundingClientRect();a.samples.push({width:r.width,height:r.height});if(a.quiet&&!c.error)a.modal.push({width:m.width,height:m.height});if(c.detail?.account.id!==1||JSON.stringify(c.detail.positions)!==a.positions||document.querySelector(".table-wrap")!==a.node)a.problems.push("current detail changed or disappeared");if(a.quiet&&(c.loading||c.explicitRefresh||b.classList.contains("n-button--loading")))a.problems.push("poll displayed loading");};a.sample();a.timer=setInterval(a.sample,25);})()');
  for(let cycle=0;cycle<3;cycle++){
   await t.evaluate('window.__capitalMock.deferDetailId=1');await t.wait('window.__capitalMock.deferred.length===1');
   assert.equal(await t.evaluate('window.__capitalComponent.$.setupState.detail===window.__capitalRefreshAudit.detail'),cycle===0);
   assert.equal(await t.evaluate('window.__capitalComponent.$.setupState.loading'),false);
   await t.evaluate('new Promise(r=>setTimeout(r,350))');
   await t.evaluate((cycle===2?'window.__capitalMock.failDetailId=1;':'')+'window.__capitalMock.deferred.shift()()');await t.wait('!window.__capitalComponent.$.setupState.busy');
  }
  assert.match(await t.evaluate('document.querySelector(".dialog").innerText'),/隔离定时明细读取失败/);
  assert.equal(await t.evaluate('window.__capitalComponent.$.setupState.detail.account.id'),1);
  assert.match(await t.evaluate('document.querySelector(".table-wrap").innerText'),/固定持仓/);
  await t.evaluate('window.__capitalRefreshAudit.quiet=false;window.__capitalMock.deferDetailId=1;document.querySelector(".simulation-refresh").click()');
  await t.wait('window.__capitalComponent.$.setupState.explicitRefresh&&window.__capitalMock.deferred.length===1');
  assert.equal(await t.evaluate('document.querySelector(".simulation-refresh").classList.contains("n-button--loading")'),true);
  await t.evaluate('new Promise(r=>setTimeout(r,350))');await t.evaluate('window.__capitalMock.deferred.shift()()');await t.wait('!window.__capitalComponent.$.setupState.busy&&!window.__capitalComponent.$.setupState.explicitRefresh');
  await t.evaluate('new Promise(r=>setTimeout(r,350))');
  const audit=await t.evaluate('(()=>{const a=window.__capitalRefreshAudit;clearInterval(a.timer);return{samples:a.samples,modal:a.modal,problems:a.problems,error:window.__capitalComponent.$.setupState.error}})()');
  assert.deepEqual(audit.problems,[]);assert.equal(audit.error,'');assert.ok(audit.samples.length>30);
  const spans=rows=>Object.fromEntries(['width','height'].map(key=>[key,Math.max(...rows.map(r=>r[key]))-Math.min(...rows.map(r=>r[key]))]));
  refreshGeometry={cycles:3,samples:audit.samples.length,button:spans(audit.samples),quietModal:spans(audit.modal)};
  for(const [scope,values]of Object.entries({button:refreshGeometry.button,quietModal:refreshGeometry.quietModal}))for(const [dimension,delta]of Object.entries(values))assert.ok(delta<=1,scope+' '+dimension+' changed '+delta+'px');
 });
 await check('Model ownership survives legacy detail reads; no manual trading controls',async()=>{
  assert.equal(await t.evaluate('!!document.querySelector("[data-account-ownership=model_follow]")'),true);
  assert.match(await t.evaluate('document.querySelector(".simulation-signal-schedule").innerText'),/完成日日线选股.*次日开盘实时盘口买入/);
  assert.equal(await t.evaluate('document.querySelector(".simulation-signal-schedule").closest("details")'),null);
  assert.equal(await t.evaluate('[...document.querySelectorAll("button")].some(b=>["立即运行","确认","保存账户"].includes(b.innerText))'),false);
  assert.equal(await t.evaluate('[...document.querySelectorAll(".n-tabs-tab")].some(b=>b.innerText.includes("手动指令"))'),false);
 });
 await check('Account picker groups manual, automatic-exclusive and frozen accounts',async()=>{
  await t.evaluate('document.querySelector(".toolbar .n-base-selection").click()');await t.wait('document.querySelector(".n-base-select-menu")?.innerText.includes("独立手动账户")');
  const text=await t.evaluate('document.querySelector(".n-base-select-menu").innerText');assert.match(text,/独立手动账户/);assert.match(text,/自动模型专用账户/);assert.match(text,/冻结研究/);
  await t.evaluate('[...document.querySelectorAll(".n-base-select-option")].find(b=>b.innerText.startsWith("#1 ")).click()');
 });
 await check('Automatic account is preferred even when the server lists a manual account first',async()=>{
  assert.equal(await t.evaluate('window.__capitalComponent.$.setupState.selectedId'),1);
 });
 await t.evaluate('window.__capitalComponent.$.setupState.activeTab="account"');await t.wait('document.querySelector("[data-capital-input] input")');assert.equal(await t.evaluate('document.querySelectorAll(".capital-editor").length'),1);

 await check('One global automatic capital preset affects future accounts only, not existing capitals/holdings',async()=>{
  await t.evaluate('document.querySelector(".automatic-capital-preset").open=true');
  const old=await t.evaluate('window.__capitalMock.accounts.map(a=>({id:a.id,initial_cash:a.initial_cash,current_cash:a.current_cash}))');
  await t.evaluate('(()=>{const input=document.querySelector("[data-auto-preset-input]");input.value="220000.25";input.dispatchEvent(new Event("input",{bubbles:true}))})()');await t.click('保存预设');await t.wait('window.__capitalMock.preset.initial_cash_cny===220000.25&&!window.__capitalComponent.$.setupState.busy');
  assert.deepEqual(await t.evaluate('window.__capitalMock.accounts.map(a=>({id:a.id,initial_cash:a.initial_cash,current_cash:a.current_cash}))'),old);
  assert.deepEqual(await t.evaluate('window.__capitalMock.calls.filter(c=>c.command==="simulation_save_auto_preset").at(-1).args'),{input:{initial_cash_cny:220000.25}});
  assert.equal(await t.evaluate('document.querySelectorAll("[data-auto-preset-input]").length'),1);
  await t.evaluate('window.__capitalMock.failPreset=true;(()=>{const input=document.querySelector("[data-auto-preset-input]");input.value="330000.25";input.dispatchEvent(new Event("input",{bubbles:true}))})()');await t.click('保存预设');await t.wait('document.body.innerText.includes("隔离预设保存失败")&&!window.__capitalComponent.$.setupState.busy');
  assert.equal(await t.evaluate('window.__capitalMock.preset.initial_cash_cny'),220000.25);assert.equal(await t.evaluate('document.querySelector("[data-auto-preset-input]").value'),'330000.25');await t.evaluate('window.__capitalMock.failPreset=false;document.querySelector(".automatic-capital-preset").open=false');
 });
 await check('Dedicated model capital editor invokes agreed CNY command without editing rules',async()=>{
  await t.evaluate('(()=>{let e=document.querySelector("[data-capital-input] input");e.value="120000.01";e.dispatchEvent(new Event("input",{bubbles:true}))})()');await t.click('调整资金');
  await t.wait('window.__capitalMock.accounts[0].initial_cash==="1200000100"&&!window.__capitalComponent.$.setupState.busy');
  const call=await t.evaluate('window.__capitalMock.calls.find(x=>x.command==="simulation_adjust_capital")');assert.deepEqual(call.args,{input:{accountId:1,initialCash:120000.01}});
  assert.equal(await t.evaluate('window.__capitalMock.calls.some(x=>x.command==="simulation_save_account")'),false);
 });
 await check('Failed withdrawal preserves account and the editable draft',async()=>{
  await t.evaluate('window.__capitalMock.failAdjust=true;(()=>{let e=document.querySelector("[data-capital-input] input");e.value="1000";e.dispatchEvent(new Event("input",{bubbles:true}))})()');await t.click('调整资金');
  await t.wait('document.body.innerText.includes("减少后现金不足")&&!window.__capitalComponent.$.setupState.busy');
  assert.equal(await t.evaluate('window.__capitalMock.accounts[0].initial_cash'), '1200000100');assert.equal(await t.evaluate('document.querySelector("[data-capital-input] input").value'), '1000');
  await t.evaluate('window.__capitalMock.failAdjust=false');
 });

 await check('Per-model configuration moved here and uses backend matching for quantities up to ten',async()=>{
  await t.evaluate('document.querySelector(".model-maximum-settings").open=true;window.__capitalComponent.$.setupState.maximum=10;window.__capitalComponent.$.setupState.maximumDirty=true');await t.click('保存数量');await t.wait('window.__capitalMock.maxima[1]===10&&!window.__capitalComponent.$.setupState.busy');
  const call=await t.evaluate('window.__capitalMock.calls.filter(c=>c.command==="research_follow_update").at(-1)');assert.deepEqual(call.args,{accountId:1,maxPositions:10,enabled:null});assert.equal(await t.evaluate('window.__capitalMock.maxima[4]'),5);
  assert.match(await t.evaluate('document.querySelector(".model-account-config").innerText'),/本模型入场条件/);
 });
 await check('Only the selected model can be paused or resumed; manual source controls remain unavailable',async()=>{
  await t.click('暂停此账户买卖');await t.wait('!window.__capitalMock.accounts[0].auto_enabled&&!window.__capitalComponent.$.setupState.busy');
  const call=await t.evaluate('window.__capitalMock.calls.filter(c=>c.command==="research_follow_update").at(-1)');assert.deepEqual(call.args,{accountId:1,maxPositions:null,enabled:false});assert.equal(await t.evaluate('window.__capitalMock.accounts[3].auto_enabled'),true);
  assert.equal(await t.evaluate('!!document.querySelector("[data-manual-capital-input]")'),false);
 });
 await check('Quick switching replaces holdings and ignores a late detail response from another account',async()=>{
  await t.evaluate('window.__capitalComponent.$.setupState.activeTab="overview";window.__capitalMock.deferDetailId=1;void window.__capitalComponent.$.setupState.loadDetail()');await t.wait('window.__capitalMock.deferred.length===1');
  await t.evaluate('[...document.querySelectorAll(".simulation-account-cards button")].find(b=>b.dataset.accountId==="4").click()');await t.wait('window.__capitalComponent.$.setupState.detail?.account.id===4');
  await t.evaluate('window.__capitalMock.deferred.shift()()');await t.wait('window.__capitalComponent.$.setupState.detail?.account.id===4');
  assert.match(await t.evaluate('document.querySelector(".table-wrap").innerText'),/另一模型持仓/);assert.doesNotMatch(await t.evaluate('document.querySelector(".table-wrap").innerText'),/固定持仓/);
  await select(1);await t.evaluate('window.__capitalMock.deferDetailId=1;void window.__capitalComponent.$.setupState.act(()=>window.__capitalComponent.$.setupState.loadDetail(),true)');await t.wait('window.__capitalMock.deferred.length===1');
  await t.evaluate('window.__capitalComponent.$.setupState.selectedId=4');await t.wait('document.querySelector("[data-simulation-detail-loading]")?.innerText.includes("#4")');
  assert.equal(await t.evaluate('window.__capitalComponent.$.setupState.detail'),null);
  await t.evaluate('window.__capitalMock.failDetailId=1;window.__capitalMock.deferred.shift()()');await t.wait('window.__capitalComponent.$.setupState.detail?.account.id===4&&!window.__capitalComponent.$.setupState.busy');
  assert.equal(await t.evaluate('window.__capitalComponent.$.setupState.error'),'');

  await select(1);await t.evaluate('window.__capitalComponent.$.setupState.activeTab="account"');await t.wait('document.querySelector(".model-account-config")');
 });

 await check('Global pause is not advertised as running by account cards, overview or next-check text',async()=>{
  await t.evaluate('window.__capitalMock.globalPaused=true;window.__capitalComponent.$.setupState.activeTab="overview"');await select(4);
  assert.match(await t.evaluate('[...document.querySelectorAll(".simulation-account-cards button")].find(b=>b.dataset.accountId==="4").innerText'),/全局已暂停/);
  assert.match(await t.evaluate('document.querySelector(".model-overview").innerText'),/全局已暂停/);
  assert.match(await t.evaluate('window.__capitalComponent.$.setupState.nextCheck'),/全局已暂停/);
  await t.evaluate('window.__capitalMock.globalPaused=false;void window.__capitalComponent.$.setupState.loadDetail()');await t.wait('window.__capitalComponent.$.setupState.follow?.effective_enabled===undefined');
  await select(1);await t.evaluate('window.__capitalComponent.$.setupState.activeTab="account"');await t.wait('document.querySelector(".model-account-config")');
 });
 await check('Model capital form fits light/dark themes and a narrow window',async()=>{
  for(const width of [1280,430])for(const theme of ['light','dark']){await t.call('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:false});await t.evaluate('window.__capitalApp.theme='+JSON.stringify(theme)+';document.documentElement.dataset.theme='+JSON.stringify(theme));
   assert.equal(await t.evaluate('(()=>{const e=document.querySelector(".capital-editor");return e.scrollWidth<=e.clientWidth+1})()'),true);await t.evaluate('new Promise(r=>setTimeout(r,500))');const sizes=await t.evaluate('[...document.querySelectorAll(".simulation-account-cards small,.automatic-capital-preset summary span")].map(e=>parseFloat(getComputedStyle(e).fontSize))');assert.deepEqual([...new Set(sizes)],[12]);secondaryFontSizes.push({width,theme,fontSizePx:[...new Set(sizes)]});await t.screenshot('capital-'+theme+'-'+width);}
 });
 await t.call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});await select(2);await t.wait('document.querySelector("[data-manual-capital-input] input")');
 await check('Existing manual account initial cash is editable in normal account form',async()=>{
  assert.equal(await t.evaluate('document.querySelector("[data-manual-capital-input] input").disabled'),false);
  await t.evaluate('window.__capitalComponent.$.setupState.form.initialCash="115000.05"');await t.click('保存账户');await t.wait('window.__capitalMock.accounts[1].initial_cash==="1150000500"&&!window.__capitalComponent.$.setupState.busy');
  assert.equal(await t.evaluate('window.__capitalMock.accounts[1].current_cash'), '1150000500');
  assert.equal(await t.evaluate('[...document.querySelectorAll(".n-tabs-tab")].some(b=>b.innerText.includes("独立手动指令"))'),true);
 });
 await select(3);await t.wait('document.body.innerText.includes("冻结研究对照")');
 await check('Frozen research account exposes neither funds editing nor manual order controls',async()=>{
  assert.equal(await t.evaluate('!!document.querySelector(".capital-editor")'),false);
  assert.equal(await t.evaluate('!!document.querySelector("[data-manual-capital-input]")'),false);
  assert.equal(await t.evaluate('[...document.querySelectorAll("button")].some(b=>["立即运行","确认","保存账户","调整资金"].includes(b.innerText))'),false);
 });
 await select(1);await t.evaluate('window.__capitalComponent.$.setupState.activeTab="overview"');await t.wait('document.querySelector(".capital-history")');
 await check('Capital audit and return baseline are visible alongside unchanged holdings',async()=>{
  assert.match(await t.evaluate('document.body.innerText'),/收益按最新初始资金基准/);assert.match(await t.evaluate('document.querySelector(".capital-history").innerText'),/资金调整记录/);
  assert.equal(await t.evaluate('window.__capitalComponent.$.setupState.detail.positions[0].quantity'),100);
 });
 assert.equal(t.errors.length,0,JSON.stringify(t.errors));t.save({schema:'simulation-capital-ui-v1',passed:results.length,checks:results,refreshGeometry,secondaryFontSizes,limitation:'Real Vue components with isolated synthetic IPC; no real accounts or orders changed'});console.log(JSON.stringify({passed:results.length,output:t.output}));
}finally{await t.close();}
