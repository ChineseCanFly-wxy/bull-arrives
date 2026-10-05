import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdtempSync,rmSync,readFileSync,mkdirSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createServer} from 'vite';
import vue from '@vitejs/plugin-vue';
// Actual Vue rendering, mocked IPC. This does not claim native/network execution.
const root=fileURLToPath(new URL('..',import.meta.url));
const target=path.join(root,'src-tauri/target');const profile=mkdtempSync(path.join(target,'research-center-ui-'));
const externalEvidence=JSON.parse(readFileSync(path.join(root,'src-tauri/external-strategy-evidence.json'),'utf8'));
const mock=`
import {reactive} from 'vue';
window.__lab={externalSettings:[],calls:[],runs:[],tasks:[],watches:[{id:702,enabled:false,config:{symbol:'sz301068',model_id:'index26_h20',preset:'model_confirm'},observation:{state:'watching',last_scan_day:'2026-09-30',message:'已暂停的旧提醒'}},{id:701,enabled:true,config:{symbol:'sh600101',model_id:'breadth22_h20',preset:'model_hit'},observation:{state:'watching',last_scan_day:'2026-09-30',message:'隔离股票提醒，不参与买卖'}}],config:{research_root:'E:/research',python:'python',snapshot:'E:/snapshot',index:'E:/csi.ndjson'}};
const config={auto_research:false,observation_days:28,min_samples:20,max_drawdown_bps:1500,min_return_bps:0,initial_cash:'1000000000',max_active:3,stock_count:5,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:1,slippage_bps:5};
export async function invoke(command,args){const s=window.__lab;s.calls.push({command,args});
 if(command==='get_model_candidates')return{schema:'research-model-candidates-v1',current_as_of:'2026-09-30',as_of:'2026-09-30',fresh:true,groups:[{model_id:'breadth22_h20',name:'技术＋市场广度',as_of:'2026-09-30',holding_days:20,score_semantic:'隔离分数，不是胜率',scored_stocks:2500,hit_count:1,threshold:0,candidates:[{symbol:'sh600101',as_of:'2026-09-30',score:0.04,close:10,signal_eligible:true,reason:'隔离候选条件'}],model_sha256:'a'.repeat(64),runner_sha256:'b'.repeat(64),source_fingerprint:'c'.repeat(64),job_id:88,production_admission:false}],job_id:88,message:'隔离候选，不是市场研究',production_admission:false};
 if(command==='get_model_candidate_labels')return Object.fromEntries(args.symbols.map(symbol=>[symbol,{name:'提醒样例',excluded:false}]));
 if(command==='research_job_get')return JSON.parse(JSON.stringify(s.jobs.find(j=>j.id===args.id)));
 if(command==='research_job_list')return JSON.parse(JSON.stringify(s.jobs??[]));
 if(command==='model_condition_watches'){const value=JSON.parse(JSON.stringify({presets:[{id:'model_hit',name:'原模型推荐'}],watches:s.watches,limit_per_tick:20}));if(s.deferList)return new Promise(resolve=>{s.listResolvers??=[];s.listResolvers.push(()=>resolve(value));});return value;}
 if(command==='model_condition_delete'){if(s.deleteFails)throw Error('隔离删除失败');const remove=()=>{s.watches=s.watches.filter(w=>w.id!==args.id);};if(s.deferDelete)return new Promise(resolve=>s.deleteResolve=()=>{remove();resolve();});remove();return;}
 if(command==='model_condition_enable'){s.watches.find(w=>w.id===args.id).enabled=args.enabled;return;}
 if(command==='model_condition_watch'){const row=s.watches.find(w=>w.config.symbol===args.request.symbol&&w.config.model_id===args.request.model_id);row.config.preset=args.request.preset;row.enabled=true;return row.id;}
 if(command==='model_condition_auto_update')return{job_id:null,message:'仅自动更新条件提醒的信号来源，不创建交易账户'};
 if(command==='research_job_start'){
   s.jobs??=[];const request=args.request;if(request.kind==='explore'){const job={created_at:'2026-10-04T08:00:00Z',updated_at:'2026-10-04T08:00:00Z',id:s.jobs.length+1,kind:'explore',request,state:'running',phase:'calculating',message:'研究年份',completed:0,total:5,results:[]};s.jobs.unshift(job);setTimeout(()=>{job.results=[2022,2023,2024,2025,2026].map(year=>({key:'nested:'+year,model_id:'technical-nested-v1',comparison:'nested',year,bank_size:12,accounts:12,evaluations:['balanced','return','defensive'].map(policy=>({policy,selected_id:null,status:'cash',metrics:{net_return_pct:0,max_drawdown_pct:0,completed_holding_cycles:0}}))}));job.completed=5;job.state='complete';},80);return JSON.parse(JSON.stringify(job));}const job={id:s.jobs.length+1,kind:request.kind,request,state:'running',phase:'calculating',message:'后台计算',completed:0,total:request.comparisons.length,results:[]};s.jobs.unshift(job);
   setTimeout(async()=>{for(const comparison of request.comparisons){const id=await invoke('research_model_run',{modelId:request.models[0],holdingDays:comparison==='holding15'?15:20,comparison,mode:request.kind==='forward'?'forward':'replay',continuation:request.continuation});job.results.push({key:comparison,model_id:request.models[0],comparison,holding_days:comparison==='holding15'?15:20,run_id:id,metrics:{net_return_pct:0,max_drawdown_pct:0,completed_holding_cycles:0}});if(request.enable_observation)s.runs.find(r=>r.id===id).enabled=true;job.completed++;}job.state='complete';job.message='已核验账本';},80);return JSON.parse(JSON.stringify(job));
 }
 if(command==='research_dashboard')return{config,experiments:[],local_ready:true,agent_installed:true,last_auto_message:'',busy:false};
 if(command==='research_model_runs')return JSON.parse(JSON.stringify({runs:s.runs,tasks:s.tasks,config:s.config,last_error:null}));
 if(command==='research_model_config'){s.config=args.config;return s.config;}
 if(command==='get_agent_status')return{state:'ready',installed:true,path:'claude',message:'fixture'};
 if(command==='get_settings')return{};
 if(command==='research_market_data_status')return{state:'no_active_accounts',expected_as_of:'2026-09-30',enabled_accounts:0,missing_accounts:0,accounts:[],last_refresh:{mode:'saved_snapshot'},policy:'StockDB优先，近期缺口尝试行情源',limitation:'跨多日缺口需历史数据'};
 if(command==='research_follow_accounts')return[];
 if(command==='save_research_config'){Object.assign(config,args.config);return;}
 if(command==='research_discover'){if(!s.tasks.length)s.tasks.push({id:5,name:'<img src=x onerror=window.__labXss=true>任务',hypothesis:'真实去重任务',model_id:'breadth22_h20',holding_days:15,comparison:'holding15',invalidation:['成本反证'],missing_data:[]});return 5;}
 if(command==='research_model_run'){await new Promise(r=>setTimeout(r,80));const v={id:args.continuation??s.runs.length+1,model_id:args.modelId,model_name:'实际模型',score_semantic:'预测非胜率',mode:args.mode,as_of:'2026-09-30',holding_days:args.holdingDays,comparison:args.comparison,state:args.mode==='forward'?'waiting_new_data':'historical_replay',enabled:false,forward_start:20260930,content_sha256:'a'.repeat(64),model_sha256:'b'.repeat(64),data_sha256:{snapshot:'c'.repeat(64)},signal_watch:[],limitations:['未准入'],ledger:{metrics:{net_return_pct:0,max_drawdown_pct:0,completed_holding_cycles:0,win_rate_pct:null,open_positions:0,max_overdue_sessions:0},curve:[{date:20260930,cash:100000,equity:100000,positions:0,dividend_receivable:0}],orders:[],unclosed:[]}};if(!args.continuation)s.runs.push(v);return v.id;}
 if(command==='research_model_observe'){s.runs.find(r=>r.id===args.id).enabled=args.enabled;return;}
 if(command==='research_extension_report')return{studies:[...${JSON.stringify(externalEvidence.studies)},{title:'财务37探索研究',status:'执行稳健性未通过',message:'不能据分段漂亮收益直接采纳',limitations:['未知不填0']} ]};
 return null;
}
export async function open(){return null;}export async function openUrl(){}
const settingsStore=reactive({settings:{research_notifications_enabled:'1'},researchNotificationsEnabled:true,localHistoryEnginePath:'stockdb.exe',localHistoryEnabled:false,localHistoryAutoUpdateEnabled:true,localHistoryAutoUpdateTime:'09:00',localHistoryUrl:'http://127.0.0.1:7899',stockDbStatus:null,async fetchStockDbStatus(){},async scanStockDb(){},async setSetting(key,value){window.__lab.calls.push({command:'set_setting',args:{key,value}});this.settings[key]=value;if(key==='research_notifications_enabled')this.researchNotificationsEnabled=value!=='0';return true;}});export function useSettingsStore(){return settingsStore;}
`;
const server=await createServer({root,configFile:false,cacheDir:path.join(profile,'vite-cache'),optimizeDeps:{noDiscovery:true,entries:[],include:['vue','pinia','naive-ui','klinecharts'],exclude:['@tauri-apps/api/core','@tauri-apps/plugin-dialog','@tauri-apps/plugin-opener']},plugins:[vue(),{
 name:'research-center-fixture',enforce:'pre',resolveId(id){if(['@tauri-apps/api/core','@tauri-apps/plugin-dialog','@tauri-apps/plugin-opener','lab-store','@/stores/settings'].includes(id))return'\0lab-mock';if(id==='/lab-entry.js')return'\0lab-entry';},
 load(id){if(id==='\0lab-mock')return mock;if(id==='\0lab-entry')return`import{createApp}from'vue';import{createPinia}from'pinia';import '/src/assets/styles/variables.css';import '/src/assets/workspace.css';document.documentElement.dataset.theme='light';document.documentElement.dataset.style='modern';import Center from '/src/components/research/ResearchCenter.vue';createApp(Center,{show:true,onOpenSettings:value=>window.__lab.externalSettings.push(value)}).use(createPinia()).mount('#app');`;},
 configureServer(s){s.middlewares.use((req,res,next)=>{if(req.url!=='/')return next();res.setHeader('Content-Type','text/html');res.end('<html><meta charset="utf-8"><div id="app"></div><script type="module" src="/lab-entry.js"></script></html>');});}
}],resolve:{dedupe:['vue'],alias:[{find:'@/stores/settings',replacement:'lab-store'},{find:'@',replacement:path.join(root,'src')}]},server:{host:'127.0.0.1',port:0,watch:{ignored:['**/src-tauri/**']}}});
let browser,ws;const runtimeErrors=[];const pending=new Map();let counter=0;
try{
 await server.listen();const port=server.httpServer.address().port;
 browser=spawn(process.env.NEWS_TEST_BROWSER||'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--disable-gpu','--disable-extensions','--no-first-run','--remote-debugging-port=0',`--user-data-dir=${profile}`,`http://127.0.0.1:${port}/`],{windowsHide:true,stdio:'ignore'});
 let debug;for(let i=0;i<100;i++){try{debug=Number(readFileSync(path.join(profile,'DevToolsActivePort'),'utf8').split('\n')[0]);break;}catch{await new Promise(r=>setTimeout(r,100));}}assert.ok(debug);
 const pages=await(await fetch(`http://127.0.0.1:${debug}/json/list`)).json();const page=pages.find(v=>v.type==='page'&&v.url.startsWith(`http://127.0.0.1:${port}/`));assert.ok(page,'The local test page must exist; unrelated browser welcome tabs are excluded');ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 ws.addEventListener('message',e=>{const v=JSON.parse(e.data);if(v.method==='Runtime.exceptionThrown')runtimeErrors.push(v.params.exceptionDetails);if(v.method==='Runtime.consoleAPICalled'&&['error','warning'].includes(v.params.type))runtimeErrors.push(v.params.args.map(a=>a.description||a.value));const cb=pending.get(v.id);if(cb){pending.delete(v.id);v.error?cb.reject(Error(JSON.stringify(v.error))):cb.resolve(v.result);}});
 const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++counter;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
 const evaluate=async expression=>{const v=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!v.exceptionDetails,JSON.stringify(v.exceptionDetails));return v.result.value;};
 await call('Runtime.enable');
 const wait=async expression=>{for(let i=0;i<100;i++){if(await evaluate(expression))return;await new Promise(r=>setTimeout(r,100));}console.log(await evaluate(`({body:document.body.innerText,html:document.body.innerHTML.slice(0,1000),nestedState:(()=>{const s=document.querySelector('.nested-research')?.__vueParentComponent?.setupState;return s?{keys:Object.keys(s),jobs:JSON.parse(JSON.stringify(s.jobs)),selected:s.selected,report:JSON.parse(JSON.stringify(s.report)),records:JSON.parse(JSON.stringify(s.records)),error:s.error}:null})(),calls:window.__lab?.calls})`));console.log('Runtime errors',JSON.stringify(runtimeErrors.slice(0,8).map(e=>Array.isArray(e)?e.map(v=>String(v).slice(0,600)):({text:e.text,exception:e.exception?.description?.slice(0,1200)}))));throw Error('Timeout '+expression);};
 const click=async text=>evaluate(`[...document.querySelectorAll('button')].find(v=>v.textContent.includes(${JSON.stringify(text)})).click()`);
 await wait(`document.querySelector('.follow-setup')&&!document.querySelector('.follow-setup-start').disabled`);
 assert.equal(await evaluate(`document.querySelector('.follow-advanced-creation').open`),false,'Default simple page is automatic tracking with professional controls collapsed');
 assert.equal(await evaluate(`window.__lab.calls.filter(c=>c.command==='research_follow_setup').length`),0,'Opening research never starts trading');
 assert.equal(await evaluate(`document.querySelector('.research-notification-details').open`),false,'Notification tests are optional');
 await click('手动条件提醒');await wait(`document.querySelector('.research-model-screen')`);
 assert.equal(await evaluate(`!!document.querySelector('.model-follow')||!!document.querySelector('.follow-account-picker')`),false,'Manual observation exposes no automatic account controls');
 assert.match(await evaluate(`document.querySelector('.manual-reminder-intro').innerText`),/无需交易账户[\s\S]*不会自动买入或卖出[\s\S]*专属自动账户/);
 assert.equal(await evaluate(`document.body.innerText.includes('研究提醒设置 / 测试')`),false,'Duplicate broken notification button is removed');
 assert.equal(await evaluate('document.querySelectorAll(".research-notification-switch").length'),1,'Research has one global reminder switch');
 assert.equal(await evaluate('[...document.querySelectorAll("button")].filter(b=>/^(暂停提醒|恢复提醒)$/.test(b.textContent.trim())).length'),1,'No per-account or per-watch pause-reminder buttons');
 assert.equal(await evaluate('document.querySelectorAll(".condition-manager .delete-watch").length'),2);
 const watchBefore=await evaluate('window.__lab.calls.length');
 await evaluate('window.__lab.deleteFails=true;document.querySelector(".condition-manager .delete-watch").click()');
 await wait('document.querySelector(".condition-manager .error")?.textContent.includes("隔离删除失败")');
 assert.equal(await evaluate('document.querySelectorAll(".condition-manager article").length'),2,'A failed deletion keeps both rows');
 await evaluate('window.__lab.deleteFails=false;window.__lab.deferList=true;void document.querySelector(".condition-manager").__vueParentComponent.setupState.refresh()');
 await wait('window.__lab.listResolvers?.length===1');
 await evaluate('window.__lab.deferList=false;window.__lab.deferDelete=true;document.querySelector(".condition-manager .delete-watch").click()');
 await wait('!!window.__lab.deleteResolve');
 assert.equal(await evaluate('[...document.querySelectorAll(".condition-manager .delete-watch")].every(b=>b.disabled)'),true,'Concurrent row actions are disabled while removal is pending');
 await evaluate('window.__lab.deleteResolve();window.__lab.deferDelete=false');
 await wait('document.querySelectorAll(".condition-manager article").length===1&&!document.querySelector(".condition-manager .delete-watch").disabled');
 await evaluate('window.__lab.listResolvers[0]()');await evaluate('new Promise(r=>setTimeout(r,50))');
 assert.equal(await evaluate('document.querySelectorAll(".condition-manager article").length'),1,'An older list response cannot resurrect the deleted reminder');
 assert.deepEqual(await evaluate('window.__lab.calls.filter(c=>c.command==="model_condition_delete").at(-1).args'),{id:702});
 assert.equal(await evaluate('window.__lab.watches.some(w=>w.id===702)'),false,'Paused reminders can also be deleted');
 assert.equal(await evaluate('window.__lab.calls.slice('+watchBefore+').some(c=>/research_follow_(?:setup|start|update|refresh|order|delete)|simulation_/.test(c.command))'),false,'Manual deletion never calls trade or account commands');
 const observeBefore=await evaluate('window.__lab.calls.length');
 await wait(`document.querySelector('.model-table-wrap tbody tr')`);
 await evaluate(`[...document.querySelectorAll('.model-table-wrap tbody tr button')].find(button=>button.textContent.trim()==='观察').click()`);
 await wait(`document.querySelector('.condition-choice')`);
 assert.equal(await evaluate(`document.querySelector('.observation-options').open`),false,'Manual observation uses model defaults without requiring advanced options');
 assert.equal(await evaluate(`document.querySelector('.condition-choice .follow-account-picker')!==null`),false,'Manual observation has no automatic-account picker');
 await evaluate(`[...document.querySelectorAll('.condition-choice button')].find(button=>button.textContent.includes('保存并开始观察')).click()`);
 await wait(`window.__lab.calls.some(call=>call.command==='model_condition_auto_update')&&!document.querySelector('.condition-choice')`);
 assert.deepEqual(await evaluate(`window.__lab.calls.find(call=>call.command==='model_condition_watch').args`),{request:{job_id:88,model_id:'breadth22_h20',symbol:'sh600101',preset:'model_hit'}});
 assert.equal(await evaluate(`window.__lab.calls.slice(${observeBefore}).some(call=>/research_follow_|simulation_/.test(call.command))`),false,'Saving manual observation and its signal refresh calls no trading or account commands');
 assert.deepEqual(await evaluate(`window.__lab.runs`),[],'Manual reminder does not create a trading account or alter research ledgers in this fixture');
 assert.equal(await evaluate(`window.__lab.calls.some(v=>v.command==='research_job_start'||v.command==='research_discover')`),false,'Opening simple mode never starts research or AI');
 assert.equal(await evaluate(`document.querySelector('.research-nav').querySelectorAll('button').length`),2);
 await click('自动模型交易');await wait(`document.querySelector('.follow-setup')`);assert.match(await evaluate(`document.body.innerText`),/按推荐一键启动自动模拟/);assert.match(await evaluate(`document.querySelector('.follow-create').textContent`),/尚无合格模型来源账本/);
 await click('深入研究');await wait(`document.querySelector('.wizard-progress')`);
 assert.match(await evaluate(`document.querySelector('.wizard-progress').innerText`),/检查行情[\s\S]*选择模型[\s\S]*历史回放[\s\S]*查看结果/);
 assert.equal(await evaluate(`document.querySelector('.research-nav').querySelectorAll('button').length`),4);
 assert.equal(await evaluate(`document.body.innerText.includes('旧模拟记录')`),false);
 await click('模型模拟');await wait(`document.querySelectorAll('.mode-grid .mode-card').length===5`);
 await evaluate(`document.querySelector('.research-notification-details').open=false`);
 const focusBefore=await evaluate('window.__lab.calls.length');await evaluate('void document.querySelector(".research-notifications").__vueParentComponent.setupState.focus()');
 await wait(`document.querySelector('.research-notification-details').open&&document.activeElement===document.querySelector('.research-notification-switch')`);
 assert.deepEqual(await evaluate('window.__lab.externalSettings'),[],'Collapsed local research notifications expand and focus without a settings-page jump');
 assert.equal(await evaluate(`window.__lab.calls.slice(${focusBefore}).some(c=>/set_setting|test_notification|research_follow_|simulation_/.test(c.command))`),false,'Focusing notification details starts no trade, setting write or test notification');
 await evaluate(`document.querySelector('.research-notification-details').open=false`);
 assert.equal(await evaluate(`document.querySelector('.toolbar select').options.length`),5);
 assert.equal(await evaluate(`window.__lab.calls.some(v=>v.command==='research_model_run'||v.command==='research_discover')`),false);
 await click('运行真实多年回放');await wait(`window.__lab.runs.length===1&&document.body.innerText.includes('历史回放')`);
 assert.deepEqual(await evaluate(`window.__lab.calls.find(v=>v.command==='research_job_start').args.request`),{kind:'replay',models:['breadth22_h20'],comparisons:['baseline'],continuation:null});
 await wait(`document.querySelectorAll('.research-jobs article[data-job-state=complete]').length===1`);await evaluate(`const s=document.querySelector('.toolbar select');s.value='holding15';s.dispatchEvent(new Event('change',{bubbles:true}));`);await click('从最新完成日开启纸上账户');await wait(`window.__lab.runs.length===2&&document.body.innerText.includes('等待新增完成行情')`);
 assert.equal(await evaluate(`window.__lab.calls.filter(v=>v.command==='research_job_start').at(-1).args.request.comparisons[0]`),'holding15');
 assert.match(await evaluate(`document.body.innerText`),/100,000/);
 await wait(`document.querySelectorAll('.research-jobs article[data-job-state=complete]').length===2`);await click('开启盘后更新');await wait(`window.__lab.runs[1].enabled===true`);await click('暂停盘后更新');await wait(`window.__lab.runs[1].enabled===false`);
 await click('比较本模型');await wait(`document.querySelectorAll('.research-jobs article[data-job-state=complete]').length===3`);assert.deepEqual(await evaluate(`window.__lab.calls.filter(v=>v.command==='research_job_start').at(-1).args.request.comparisons`),['baseline','holding15','cost_double']);
 await click('更多');await click('AI 研究');await click('AI 提出新研究假说');await wait(`window.__lab.tasks.length===1`);await click('更多');await click('AI 研究');await click('AI 提出新研究假说');await wait(`window.__lab.calls.filter(v=>v.command==='research_discover').length===2`);assert.equal(await evaluate(`window.__lab.tasks.length`),1);assert.equal(await evaluate(`!!window.__labXss||document.querySelectorAll('.research-center img').length>0`),false);
 await click('更多');await click('扩展研究');await wait(`document.querySelector('.extension-study')&&document.body.innerText.includes('执行稳健性未通过')`);
 assert.equal(await evaluate(`window.__lab.calls.some(v=>v.command==='research_intraday_status')`),false);
 assert.equal(await evaluate(`document.querySelectorAll('.study-comparison').length`),2);
 assert.equal(await evaluate(`[...document.querySelectorAll('.study-comparison')].every(v=>!v.open)`),true,'Experimental metrics start collapsed');
 await evaluate(`document.querySelectorAll('.study-comparison')[1].querySelector('summary').click()`);
 assert.equal(await evaluate(`document.querySelectorAll('.study-comparison')[1].querySelectorAll('tbody tr').length`),10);
 assert.match(await evaluate(`document.querySelectorAll('.study-comparison')[1].innerText`),/56\.123%[\s\S]*-0\.626%/);
 assert.match(await evaluate(`document.querySelectorAll('.study-meta')[1].innerText`),/192[\s\S]*新增可用模型 0/);
 await evaluate(`document.querySelectorAll('.study-comparison')[1].querySelector('summary').click()`);
 await evaluate(`document.querySelectorAll('.extension-study .help-tooltip-trigger')[3].focus()`);
 await wait(`document.querySelector('[role="tooltip"]')&&document.body.innerText.includes('151个真实信号')`);
 await evaluate(`window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))`);
 await wait(`!document.querySelector('.extension-study .help-tooltip-trigger[aria-describedby]')`);
 assert.equal(await evaluate(`document.querySelector('.extension-study').innerText.includes('分钟走势确认')`),false);
 await click('运行财务37真实历史回放');await wait(`document.querySelector('.extension-study').innerText.includes('已保存账本 #')`);
 assert.deepEqual(await evaluate(`window.__lab.calls.filter(v=>v.command==='research_model_run').at(-1).args`),{modelId:'fundamental37_h20',holdingDays:20,comparison:'baseline',mode:'replay',continuation:null});
 await click('更多');await click('自动研究设置');await wait(`document.querySelector('.auto-toggle input')`);assert.equal(await evaluate(`document.querySelector('.auto-toggle input').checked`),false);await evaluate(`document.querySelector('.auto-toggle input').click()`);await click('保存自动研究设置');await wait(`window.__lab.calls.some(v=>v.command==='save_research_config'&&v.args.config.auto_research===true)`);
 // Source preparation in the simple tracking view waits for its own completed job,
 // selects the result automatically, and reuses it without a duplicate run.
 await click('简化版');await click('自动模型交易');await wait(`document.querySelector('.follow-prepare-source')`);await evaluate(`document.querySelector('.follow-advanced-creation').open=true`);
 assert.equal(await evaluate(`document.querySelector('.follow-source select').disabled`),true,'Replay and 15-day comparison are not mistaken for a baseline source');
 await click('准备 / 复用模型来源');await wait(`document.querySelector('.follow-source select').options.length>1&&!document.querySelector('.follow-source select').disabled`);
 const sourceId=await evaluate(`Number(document.querySelector('.follow-source select').value)`);
 assert.ok(sourceId>0,'Newly completed source is immediately selected');
 const jobCount=await evaluate(`window.__lab.calls.filter(v=>v.command==='research_job_start').length`);
 await click('准备 / 复用模型来源');await wait(`document.querySelector('.daily-notice')?.textContent.includes('已复用该模型来源账户')`);
 assert.equal(await evaluate(`window.__lab.calls.filter(v=>v.command==='research_job_start').length`),jobCount,'Existing baseline source is reused');
 assert.equal(await evaluate(`Number(document.querySelector('.follow-source select').value)`),sourceId);
 await click('深入研究');
 // Validate real layout at laptop, narrow-window and small-screen sizes.
 await click('模型模拟');
 assert.equal(await evaluate(`document.body.innerText.includes('StockDB 网站的 AI 开发资料')`), false);
 for (const [width,height] of [[1280,900],[900,650],[600,480],[390,640]]) {
   await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
   await wait(`window.innerWidth===${width}&&document.querySelector('.research-scroll').clientHeight>0`);
   const measure = () => evaluate(`(()=>{const rect=e=>{const r=e.getBoundingClientRect();return{top:r.top,bottom:r.bottom,left:r.left,right:r.right}};const s=document.querySelector('.research-scroll');return{nav:rect(document.querySelector('.research-nav')),close:rect(document.querySelector('.research-modal .n-card-header__close')),modal:rect(document.querySelector('.research-modal')),scrollTop:s.scrollTop,scrollHeight:s.scrollHeight,clientHeight:s.clientHeight,clientWidth:s.clientWidth,scrollWidth:s.scrollWidth};})()`);
   await evaluate(`document.querySelector('.research-scroll').scrollTop=0`);
   const before=await measure();
   assert.ok(before.modal.left>=-1&&before.modal.right<=width+1,'Modal must fit viewport '+width);
   assert.ok(before.modal.top>=-1&&before.modal.bottom<=height+1,'Modal height must fit viewport '+height);
   assert.ok(before.scrollWidth<=before.clientWidth+2,'Long fields must not overflow content '+width);
   await evaluate(`document.querySelector('.research-scroll').scrollTop=100000`);
   const after=await measure();
   assert.ok(after.scrollHeight>after.clientHeight&&after.scrollTop>0,'Content must actually scroll '+width);
   assert.ok(Math.abs(before.nav.top-after.nav.top)<1,'Tabs must remain stationary '+width);
   assert.ok(Math.abs(before.close.top-after.close.top)<1&&after.close.bottom<=height,'Close button remains visible '+width);
   await click('更多');await wait(`document.querySelector('.more-tabs')`);await click('自动研究设置');
   assert.equal(await evaluate(`document.querySelector('.research-scroll').scrollTop`),0,'New page starts at the top');
   assert.equal(await evaluate(`document.querySelectorAll('.reading input[type=number]').length`),0,'Legacy inputs must not masquerade as model settings');
   await click('更多');await click('扩展研究');await wait(`document.querySelectorAll('.study-comparison').length===2`);
   await evaluate(`document.querySelectorAll('.study-comparison')[1].querySelector('summary').click()`);
   const expanded=await measure();
   assert.ok(expanded.scrollWidth<=expanded.clientWidth+2,'Expanded research table must scroll inside its own container '+width);
   await evaluate(`document.querySelector('.research-scroll').scrollTop=100000`);
   const expandedBottom=await measure();
   assert.ok(Math.abs(expanded.nav.top-expandedBottom.nav.top)<1,'Research tabs remain visible on expanded result page '+width);
   assert.ok(Math.abs(expanded.close.top-expandedBottom.close.top)<1,'Close remains visible on expanded result page '+width);
   await click('模型模拟');
   if(process.env.UI_TEST_SCREENSHOT_DIR){
     mkdirSync(process.env.UI_TEST_SCREENSHOT_DIR,{recursive:true});
     const image=await call('Page.captureScreenshot',{format:'png'});
     writeFileSync(path.join(process.env.UI_TEST_SCREENSHOT_DIR,'research-'+width+'.png'),Buffer.from(image.data,'base64'));
   }
 }
 await click('更多');await click('技术组合研究');await wait(`document.querySelector('.nested-research')`);
 assert.equal(await evaluate(`window.__lab.calls.filter(c=>c.command==='research_job_start'&&c.args.request.kind==='explore').length`),0);
 await click('运行五年技术组合研究');await wait(`window.__lab.jobs.find(j=>j.kind==='explore')?.state==='complete'`);await click('刷新研究结果');await wait(`document.querySelectorAll('.nested-table tbody tr').length===15`);
 assert.deepEqual(await evaluate(`window.__lab.calls.find(c=>c.command==='research_job_start'&&c.args.request.kind==='explore').args.request`),{kind:'explore',models:['technical-nested-v1'],comparisons:['baseline'],continuation:null});
 await click('简化版');await wait(`document.querySelector('.follow-setup')`);await click('自动模型交易');await wait(`document.querySelector('.follow-advanced-creation')`);await evaluate(`document.querySelector('.follow-advanced-creation').open=true`);
 await wait(`!document.querySelector('.follow-prepare-source button').disabled`);
 const beforePaper=await evaluate(`window.__lab.runs.length`);const paperStarts=await evaluate(`window.__lab.calls.filter(c=>c.command==='research_job_start').length`);await click('准备 / 复用模型来源');await wait(`document.body.innerText.includes('已复用该模型来源账户')`);assert.equal(await evaluate(`window.__lab.calls.filter(c=>c.command==='research_job_start').length`),paperStarts,'Reuses source created earlier without a duplicate job');assert.equal(await evaluate(`window.__lab.runs.length`),beforePaper);
 assert.match(await evaluate(`document.body.innerText`),/无需把股票逐只加入观察/);
 assert.equal(await evaluate(`document.querySelector('.follow-account-picker')===null`),true,'Preparing a research source alone does not create a trading account');
 const verifiedIpc=await evaluate('window.__lab.calls');
 await call('Page.reload');await wait(`document.querySelector('.follow-setup')`);assert.equal(await evaluate(`localStorage.getItem('research-center-mode-v1')`),'simple');
 assert.equal(runtimeErrors.filter(e=>!Array.isArray(e)).length,0,'No unhandled runtime exception');
 const evidenceDir=mkdtempSync(path.join(target,'research-center-evidence-'));
 const testedFiles=['src/components/research/ResearchCenter.vue','src/components/research/DailyResearch.vue','src/components/research/ModelFollowTrading.vue','src/components/research/ModelConditionWatches.vue','src/components/research/ResearchNotifications.vue','src/components/research/ResearchMarketData.vue','scripts/check-research-center-ui.mjs'];
 const result={schema:'research-center-isolation-ui-v1',passed:true,at:new Date().toISOString(),evidenceDir,checks:['Default automatic page and optional professional controls','Manual reminders need no account and expose no automatic trading controls','Saving manual observation and source update invoke no trading command','Manual reminder deletion supports failures, disabled records and stale list requests without trade IPC','A single visible reminder control and optional local notification help avoid duplicate settings entries','Source creation/reuse does not create a user trading account','Five frozen identities and independent research preserved','Persisted navigation and 1280/900/600/390px layout'],sourceFiles:Object.fromEntries(testedFiles.map(file=>[file,createHash('sha256').update(readFileSync(path.join(root,file))).digest('hex')])),ipc:verifiedIpc,afterReloadIpc:await evaluate('window.__lab.calls'),runtimeErrors,limitation:'Actual Vue and isolated synthetic IPC only. No native application, user DB, StockDB service, real notifications, updater or market data were used.'};
 writeFileSync(path.join(evidenceDir,'result.json'),JSON.stringify(result,null,2)+'\n');
 console.log(JSON.stringify({passed:true,evidence:path.join(evidenceDir,'result.json'),isolationChecks:result.checks.length},null,2));
 console.log('ResearchCenter Vue+mockIPC passed: separate automatic trading and manual reminder pages, default professional collapse, save-manual-watch and notification-pause isolation, local collapsed-notification expand/focus without external settings, no trading on open, persisted mode switch, research source create/update/reuse and automatic selection of completed account, fixed-bank five-year research explicit start/15 result rows/no runtime exceptions; four-step guide, three main entries, empty archive hidden, five registered identities, five finite modes, no auto-run on open, persistent background replay/15-forward/three-way comparison args, waiting cash/zero cycles, independent opt-in, task dedup, escaped text, minute opt-in/status and financial37 replay-only call, real external-study evidence default-collapse, positive-result counterevidence and accessible help. Expanded result tables and layout at 1280/900/600/390 widths, fixed tabs/close after scrolling, no horizontal content overflow and no retired parameter inputs passed. Native Windows/network/subprocess not exercised by this check.');
}finally{
 ws?.close();if(browser?.exitCode===null){const ended=new Promise(r=>browser.once('exit',r));browser.kill();await Promise.race([ended,new Promise(r=>setTimeout(r,2000))]);}await server.close();assert.equal(path.dirname(path.resolve(profile)),path.resolve(target));try{rmSync(profile,{recursive:true,force:true,maxRetries:10,retryDelay:100});}catch{console.log('Temporary test profile retained',profile);}
}
