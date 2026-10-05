import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { uiHarness } from './ui-check-harness.mjs';

// Real components and stores. All IPC remains synthetic and local to the browser.
const mock = `
const s = window.__themeMock = { calls:[], broadcasts:[], rejected:[], failSave:false, db:JSON.parse(localStorage.getItem('elegant-ui-settings')||'null')||{theme:'light',visual_style:'classic',local_history_enabled:'0'} };
const handlers=new Map(), clone=value=>JSON.parse(JSON.stringify(value));
const stocks=[['600519','sh','贵州茅台',1456.78,1.26],['000001','sz','平安银行',12.68,-.84],['600036','sh','招商银行',38.22,.36],['300750','sz','宁德时代',218.50,-1.32],['600900','sh','长江电力',29.46,.48],['601899','sh','紫金矿业',18.71,2.18],['002415','sz','海康威视',31.20,-.52],['600000','sh','浦发银行',10.38,.18]];
const items=stocks.map(([code,market,name],i)=>({id:i+1,code,market,name,sort_order:i,added_at:'2026-10-05'}));
const quotes=stocks.map(([code,market,name,price,change_pct])=>({code,market,name,price,change_pct,change:price*change_pct/100,prev_close:price/(1+change_pct/100),open:price,high:price*1.01,low:price*.99,volume:3487000,turnover:43865000,turnover_rate:1.82,timestamp:1791180000}));
const indices=[['上证指数','sh000001',3268.52,.72],['深证成指','sz399001',10563.18,-.48],['创业板指','sz399006',2198.56,.36],['沪深300','sh000300',3822.64,.51],['科创50','sh000688',1026.42,-.63]].map(([name,code,price,change_pct])=>({name,code,price,change_pct,change:price*change_pct/100,volume:128500000,turnover:28653000000}));
const stockdb={enabled:false,platformSupported:true,state:'disabled',phase:null,enginePath:'E:/isolated/stockdb.exe',updaterPath:null,updaterAvailable:false,owned:false,busy:false,message:'隔离外观检查 · 不连接历史服务',lastError:null,candidates:[]};
const config={auto_research:false,observation_days:28,min_samples:20,max_drawdown_bps:1500,min_return_bps:0,initial_cash:'1000000000',max_active:3,stock_count:5,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:1,slippage_bps:5};
const account={id:1,name:'广度绝对收益22 · 示例账户',managed_by:'model_follow',initial_cash:'1000000000',current_cash:'873462500',mode:'auto',auto_enabled:false,manual_source_enabled:false,rule_source_enabled:true,ai_source_enabled:false,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:0,slippage_bps:0};
const positions=[{symbol:'sh600036',name:'招商银行',quantity:300,available_quantity:300,cost_price:'371800',cost_cny:37.18,mark_cny:38.22,mark_at:'2026-10-05T07:00:00Z',entry_date:'20260930',holding_sessions:2,exit_reason:null}];
const follow={account_id:1,source_run_id:101,model_id:'breadth22_h20',model_name:account.name,max_positions:5,enabled:false,as_of:'20261005',initial_cash_cny:100000,cash_cny:87346.25,equity_cny:98812.25,net_return_pct:-1.19,state:'paused',message:'示例账本 · 隔离外观检查',source_sha256:'a'.repeat(64),positions,orders:[],candidates:[],execution:{position_pct:8,total_entry_pct:40,holding_days:20,max_gap_pct:5,buy_window:'09:30–09:35',entry_note:'原模型条件',exit_note:'固定退出条件',allocation_note:'独立账户',fee_note:'费用示例'},comparison_note:'模拟数据',performance:{filled_buys:1,filled_sells:0,cancelled:0,expired:0,confirmation_delay_seconds:null}};
const model={id:101,model_id:'breadth22_h20',model_name:'广度绝对收益22',score_semantic:'示例研究账本 · 不代表实际收益',mode:'replay',as_of:'2026-10-05',holding_days:20,comparison:'baseline',state:'complete',enabled:false,forward_start:null,content_sha256:'a'.repeat(64),model_sha256:'b'.repeat(64),data_sha256:{},signal_watch:[],limitations:['仅用于外观检查'],ledger:{metrics:{net_return_pct:7.386,max_drawdown_pct:4.21,completed_holding_cycles:18,win_rate_pct:55.56,open_positions:0,max_overdue_sessions:0},curve:[{date:20261005,cash:107386,equity:107386,positions:0,dividend_receivable:0}],orders:[{date:20260930,code:'600036',side:'buy',qty:300,price:37.18,fee:5,reason:'模拟入场记录'},{date:20261005,code:'600036',side:'sell',qty:300,price:38.22,fee:6.8,reason:'模拟退出记录'}],unclosed:[]}};
export async function listen(name,fn){const list=handlers.get(name)||new Set();list.add(fn);handlers.set(name,list);return()=>list.delete(fn);}
window.__remote=(key,value)=>{for(const fn of handlers.get('setting-changed')||[])fn({payload:{key,value}});};
window.__toast=payload=>{for(const fn of handlers.get('desktop-toast-show')||[])fn({payload});};
export async function emit(name,payload){s.broadcasts.push({name,payload});for(const fn of handlers.get(name)||[])fn({payload});}
export async function invoke(command,args={}){
 s.calls.push({command,args});
 if(command==='get_settings')return clone(s.db);
 if(command==='set_setting'){if(s.failSave)throw Error('隔离保存失败');s.db[args.key]=args.value;localStorage.setItem('elegant-ui-settings',JSON.stringify(s.db));return;}
 if(command==='get_stockdb_status')return clone(stockdb);
 if(command==='list_datasources')return [['tencent','腾讯行情']];
 if(command==='get_portable_mode')return true;
 if(command==='get_market_session')return {session:'休市',interval_secs:30,is_trading:false,calendar:'隔离示例 · 不读取行情服务'};
 if(command==='get_group_snapshot')return {active_group_id:0,groups:[{id:0,name:'全部自选',sort_order:0,item_count:items.length}],items:clone(items)};
 if(command==='get_quotes')return clone(quotes);
 if(command==='get_indices')return clone(indices);
 if(command==='get_notification_history')return {version:0,entries:[]};
 if(['get_holdings','list_monitors','get_monitors','list_monitor_rules','get_monitor_rules','research_job_list'].includes(command))return [];
 if(command==='take_pending_navigation')return null;
 if(['desktop_toast_ready','resize_ticker_window','log_frontend'].includes(command))return;
 if(command==='model_condition_watches')return {watches:[],presets:[],limit_per_tick:20};
 if(command==='research_dashboard')return {config,experiments:[],local_ready:false,agent_installed:false,last_auto_message:'',busy:false};
 if(command==='research_model_runs')return {runs:[model],tasks:[],config:{research_root:'E:/isolated',python:'python',snapshot:'mock',index:'mock'},last_error:null};
 if(command==='get_agent_status')return {installed:false,state:'unavailable',path:null,message:'外观检查不调用 Claude',guidance:'隔离示例',run_dir:null};
 if(command==='get_research_notifications')return {model_enabled:false,mainline_enabled:false,condition_enabled:false};
 if(command==='get_mainline_discovery_status')return {enabled:false,busy:false,as_of:'20261005',total:120,processed:120,failed:[],candidates:[{kind:'industry',code:'SW801080',name:'电子',as_of:'20261005',entry_ready:true,observation_state:'ready',metrics:{rs20_vs_hs300:.087,rs60_vs_hs300:.125,r5:.02,r20:.09,r60:.15}},{kind:'industry',code:'SW801030',name:'基础化工',as_of:'20261005',entry_ready:false,observation_state:'waiting_pullback',metrics:{rs20_vs_hs300:.061,rs60_vs_hs300:.102,r5:.012,r20:.07,r60:.11}}],finished:true,catalog_total:120,catalog_errors:[]};
 if(command==='simulation_list_accounts')return [clone(account)];
 if(command==='research_follow_accounts')return [clone(follow)];
 if(command==='simulation_auto_preset')return {initial_cash_cny:100000};
 if(command==='simulation_get_detail')return {account:clone(account),targets:[],positions:clone(positions),orders:[],recent_runs:[],metrics:{total_return_bps:-119},source_stats:[],capital_adjustments:[],performance_note:'示例账本'};
 if(command==='simulation_live_status')return {engine:'daily_raw_open',message:'隔离数据',updated_at:null,plans:[],executions:[]};
 s.rejected.push(command);throw Error('Unexpected isolated IPC '+command);
}
export async function isEnabled(){return false;}
export async function enable(){}
export async function disable(){}
export async function getVersion(){return '2.2.6';}
export async function open(){throw Error('Dialog prohibited in appearance test');}
export async function openUrl(){throw Error('Network prohibited in appearance test');}
export class Menu{static async new(){return {popup:async()=>{},close:async()=>{}};}}
export function getCurrentWindow(){return {onFocusChanged:async()=>()=>{},setSkipTaskbar:async()=>{}};}
`;
const entry=`
import {createApp} from 'vue';import {createPinia} from 'pinia';
import {useSettingsStore} from '/src/stores/settings.ts';
import '/src/assets/styles/variables.css';import '/src/assets/styles/dark.css';import '/src/assets/workspace.css';
localStorage.setItem('research-center-mode-v1','advanced');
const mode=window.__rootMode||'main';
const file=mode==='ticker'?'/src/components/ticker/TickerBar.vue':mode==='quick'?'/src/components/ticker/TickerQuickAdd.vue':mode==='toast'?'/src/components/notifications/ToastWindow.vue':'/src/App.vue';
const Component=(await import(/* @vite-ignore */file)).default;
const pinia=createPinia();window.__app=createApp(Component).use(pinia).mount('#app');window.__settings=useSettingsStore(pinia);
window.__style=selector=>{const el=document.querySelector(selector);if(!el)throw Error('Missing style target '+selector);const s=getComputedStyle(el);return {font:s.fontFamily,size:s.fontSize,line:s.lineHeight,color:s.color,bg:s.backgroundColor,radius:s.borderRadius,height:el.getBoundingClientRect().height,nums:s.fontVariantNumeric};};
`;
const checks=[];let ticker;
const t=await uiHarness({name:'elegant-theme',entry,mock});
const check=async(name,fn)=>{await fn();checks.push(name);console.log('PASS '+name);};
const idle=async()=>{await t.evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');await t.evaluate('Promise.all(document.getAnimations().filter(a=>Number.isFinite(a.effect.getComputedTiming().iterations)).map(a=>a.finished.catch(()=>{})))');};
const style=selector=>t.evaluate('window.__style('+JSON.stringify(selector)+')');
const navigate=async destination=>{await t.evaluate('document.querySelector(".top-bar").__vueParentComponent.setupState.navigate('+JSON.stringify(destination)+')');await idle();};
const select=async label=>{await t.click(label);await t.wait('!document.querySelector(".style-grid button:disabled")');await idle();};
const remote=async(key,value)=>{await t.evaluate('window.__remote('+JSON.stringify(key)+','+JSON.stringify(value)+')');await idle();};
const contrast=(fg,bg)=>{const luminance=color=>{const channels=color.match(/[\d.]+/g).slice(0,3).map(Number).map(v=>{v/=255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;});return channels[0]*.2126+channels[1]*.7152+channels[2]*.0722;};const a=luminance(fg),b=luminance(bg);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05);};
try{
 await t.call('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false});
 await t.wait('document.querySelectorAll(".index-card").length===5&&document.querySelectorAll(".name-text").length===8&&window.__settings?.visualStyle==="classic"');
 await check('Existing classic is preserved without a settings write',async()=>{assert.equal(await t.evaluate('window.__themeMock.calls.filter(c=>c.command==="set_setting").length'),0);assert.equal(await t.evaluate('document.documentElement.dataset.appearance??null'),null);});
 await navigate('settings:appearance');await t.wait('document.querySelector(".style-grid")');
 await check('Three selectable appearances and a Chinese/numeric preview',async()=>{assert.equal(await t.evaluate('document.querySelectorAll(".style-grid button").length'),3);assert.match(await t.evaluate('document.querySelector(".elegant-sheet").innerText'),/3,268.52/);});
 await select('中文雅致');
 await check('Elegant saves its own value and broadcasts once',async()=>{assert.equal(await t.evaluate('window.__themeMock.db.visual_style'),'elegant');assert.deepEqual(await t.evaluate('({style:document.documentElement.dataset.style,appearance:document.documentElement.dataset.appearance})'),{style:'modern',appearance:'elegant'});assert.equal(await t.evaluate('window.__themeMock.broadcasts.filter(e=>e.payload.key==="visual_style").length'),1);assert.equal(await t.evaluate('document.querySelector(".elegant-preview").getAttribute("aria-pressed")'),'true');});
 await idle();await t.screenshot('elegant-settings-light');await t.click('完成');await t.wait('!document.querySelector(".settings-modal")');await idle();
 await check('14px Chinese, Arial tabular prices, 44px rows and distinct headings',async()=>{assert.match((await style('body')).font,/Microsoft YaHei UI/);assert.equal((await style('body')).size,'14px');assert.match((await style('.index-price')).font,/Arial/);assert.match((await style('.pct-col')).nums,/tabular-nums/);assert.match((await style('.index-price')).nums,/tabular-nums/);assert.equal((await style('.n-data-table-td')).size,'14px');assert.ok((await style('.n-data-table-tr')).height>=44);assert.equal((await style('.section-title')).size,'17px');});
 await t.screenshot('elegant-workbench-light');
 await check('Reload restores elegant without rewriting the saved setting',async()=>{await t.evaluate('location.reload()');await t.wait('document.documentElement.dataset.appearance==="elegant"&&document.querySelectorAll(".name-text").length===8');assert.equal(await t.evaluate('window.__settings.visualStyle'),'elegant');assert.equal(await t.evaluate('window.__themeMock.calls.filter(c=>c.command==="set_setting").length'),0);});
 ticker=await uiHarness({name:'elegant-ticker',entry:'window.__rootMode="ticker";'+entry,mock});await ticker.wait('document.querySelector(".ticker-price")&&window.__settings');
 await check('Separate ticker receives remote appearance/brightness without rebroadcast',async()=>{await ticker.evaluate('window.__remote("visual_style","elegant");window.__remote("theme","light")');await ticker.wait('document.documentElement.dataset.appearance==="elegant"');assert.equal(await ticker.evaluate('window.__settings.visualStyle'),'elegant');assert.match(await ticker.evaluate('getComputedStyle(document.querySelector(".ticker-price")).fontFamily'),/Arial/);assert.equal(await ticker.evaluate('window.__themeMock.broadcasts.length'),0);});
 await ticker.call('Emulation.setDeviceMetricsOverride',{width:460,height:88,deviceScaleFactor:1,mobile:false});await ticker.screenshot('elegant-ticker-light');
 for(const theme of ['light','dark']){
  await remote('theme',theme);
  await check(theme+' text and red/green contrast is readable',async()=>{const body=await style('body');assert.ok(contrast(body.color,body.bg)>=4.5);const tokens=await t.evaluate('(()=>{const s=getComputedStyle(document.documentElement);return Object.fromEntries(["text-secondary","text-tertiary","up","down","surface-1"].map(k=>[k,s.getPropertyValue("--color-"+k).trim()]))})()');for(const name of ['text-secondary','text-tertiary','up','down']){const rgb=await t.evaluate('(()=>{const el=document.createElement("span");el.style.color='+JSON.stringify(tokens[name])+';el.style.background='+JSON.stringify(tokens['surface-1'])+';document.body.append(el);const s=getComputedStyle(el);const v={color:s.color,bg:s.backgroundColor};el.remove();return v})()');assert.ok(contrast(rgb.color,rgb.bg)>=4.5,name+' contrast');}assert.notEqual(tokens.up,tokens.down);});
  await check(theme+' Naive table colors and reactive overrides follow brightness',async()=>{
   assert.equal(await t.evaluate('window.__app.$.setupState.themeOverrides.common.primaryColor'),theme==='light'?'#246c62':'#83c6ba');
   assert.equal((await style('.n-data-table-td')).bg,theme==='light'?'rgb(255, 255, 255)':'rgb(30, 41, 39)');
   assert.equal((await style('.n-data-table-th')).bg,theme==='light'?'rgb(238, 239, 234)':'rgb(37, 51, 47)');
  });
  await t.screenshot('elegant-workbench-'+theme);
  await navigate('research');await t.wait('document.querySelector(".research-modal .detail .metrics b")');
  await check(theme+' research cards and ledger inherit hierarchy',async()=>{assert.match((await style('.research-modal')).font,/Microsoft YaHei/);assert.match((await style('.research-modal .metrics b')).font,/Arial/);assert.equal((await style('.research-modal .metrics b')).size,'22px');assert.equal((await style('.research-modal table')).size,'14px');assert.ok((await style('.research-modal tbody tr')).height>=44);});await idle();await t.screenshot('elegant-research-cards-'+theme);await t.evaluate('document.querySelector(".research-modal .detail").scrollIntoView({block:"start"})');await idle();await t.screenshot('elegant-research-'+theme);
  await navigate('mainline');await t.wait('document.querySelectorAll(".discovery-candidates button").length===2');assert.match((await style('.market-mainline-dialog')).font,/Microsoft YaHei/);await t.screenshot('elegant-mainline-'+theme);
  await navigate('simulation');await t.wait('document.querySelector(".model-overview.metrics b")');
  await check(theme+' Naive select/input colors follow brightness',async()=>{
   const control=await style('.dialog .n-base-selection-label');
   assert.equal(control.bg,theme==='light'?'rgb(255, 255, 255)':'rgb(30, 41, 39)');
   assert.equal(control.color,theme==='light'?'rgb(38, 59, 52)':'rgb(232, 238, 234)');
   assert.match(control.font,/Microsoft YaHei/);
  });
  await check(theme+' account cash, holdings and dialog title inherit hierarchy',async()=>{assert.match((await style('.dialog .metrics b')).font,/Arial/);assert.equal((await style('.dialog .metrics b')).size,'22px');assert.equal((await style('.dialog .n-card-header__main')).size,'20px');assert.ok((await style('.dialog tbody tr')).height>=44);});await t.screenshot('elegant-account-'+theme);
  await navigate('settings:appearance');await t.wait('document.querySelector(".style-grid")');await t.screenshot('elegant-settings-'+theme);await t.click('完成');await t.wait('!document.querySelector(".settings-modal")');
 }
 await remote('theme','light');await navigate('settings:appearance');await t.wait('document.querySelector(".style-grid")');
 await check('Modern/classic clear elegant fonts, tokens and Naive overrides',async()=>{for(const [label,value,radius] of [['清晰现代','modern','11px'],['原版','classic','6px']]){await select(label);assert.equal(await t.evaluate('document.documentElement.dataset.appearance??null'),null);assert.equal(await t.evaluate('window.__settings.visualStyle'),value);assert.doesNotMatch((await style('body')).font,/Microsoft YaHei UI/);assert.equal(await t.evaluate('window.__app.$.setupState.themeOverrides.common.borderRadius'),radius);await ticker.evaluate('window.__remote("visual_style",'+JSON.stringify(value)+')');assert.equal(await ticker.evaluate('document.documentElement.dataset.appearance??null'),null);}});
 await check('Unknown saved/remote values fall back; legacy trading maps to modern',async()=>{await remote('visual_style','unknown-future-theme');assert.equal(await t.evaluate('window.__settings.visualStyle'),'classic');await remote('visual_style','trading');assert.equal(await t.evaluate('window.__settings.visualStyle'),'modern');assert.equal(await t.evaluate('document.documentElement.dataset.appearance??null'),null);await t.evaluate('(async()=>{window.__themeMock.db.visual_style="unknown-stored";await window.__settings.fetchSettings()})()');assert.equal(await t.evaluate('window.__settings.visualStyle'),'classic');});
 await check('Failed save preserves appearance and reports a visible error',async()=>{await t.evaluate('window.__themeMock.failSave=true');await t.click('中文雅致');await t.wait('document.querySelector(".settings-error")');assert.equal(await t.evaluate('window.__settings.visualStyle'),'classic');assert.equal(await t.evaluate('document.documentElement.dataset.appearance??null'),null);await t.evaluate('window.__themeMock.failSave=false');await select('中文雅致');});
 await t.call('DOM.enable');await t.call('CSS.enable');const doc=await t.call('DOM.getDocument');const platformFonts={};
 for(const [name,selector] of [['Chinese','.name-text'],['numeric','.index-price']]){const {nodeId}=await t.call('DOM.querySelector',{nodeId:doc.root.nodeId,selector});platformFonts[name]=(await t.call('CSS.getPlatformFontsForNode',{nodeId})).fonts;}
 assert.ok(platformFonts.Chinese.some(f=>/YaHei|PingFang/i.test(f.familyName)));assert.ok(platformFonts.numeric.some(f=>/Arial/i.test(f.familyName)));checks.push('Rendered local fonts are Microsoft YaHei UI and Arial');
 for(const mode of ['quick','toast']){
  const extra=await uiHarness({name:'elegant-'+mode,entry:'window.__rootMode='+JSON.stringify(mode)+';'+entry,mock});
  try{await extra.wait('window.__settings?.settings?.theme');await extra.evaluate('window.__remote("visual_style","elegant");window.__remote("theme","light")');if(mode==='toast'){await extra.evaluate('window.__toast({id:"fixture",title:"中文雅致 · 示例提醒",body:"行情数字 12.68，研究与账户共用清晰中文字体。"})');await extra.wait('document.querySelector(".toast")');}const selector=mode==='quick'?'.quick-input':'.toast';assert.match(await extra.evaluate('getComputedStyle(document.querySelector('+JSON.stringify(selector)+')).fontFamily'),/Microsoft YaHei/);await extra.call('Emulation.setDeviceMetricsOverride',{width:420,height:mode==='quick'?420:156,deviceScaleFactor:1,mobile:false});await extra.screenshot('elegant-'+mode+'-light');for(const value of ['modern','classic']){await extra.evaluate('window.__remote("visual_style",'+JSON.stringify(value)+')');assert.equal(await extra.evaluate('document.documentElement.dataset.appearance??null'),null);}assert.equal(await extra.evaluate('window.__themeMock.broadcasts.length'),0);assert.deepEqual(extra.errors,[]);checks.push(mode+' root inherits and clears remote appearance');}finally{await extra.close();}
 }
 const source=readFileSync(new URL('../src/assets/styles/variables.css',import.meta.url),'utf8');assert.doesNotMatch(source.slice(source.indexOf('/* Chinese financial desk')),/font-size[^;]*!important|@font-face|https?:/);
 const commands=await t.evaluate('window.__themeMock.calls.map(c=>c.command)');assert.ok(!commands.some(c=>/^(run_stockdb_update|run_mainline_discovery|set_mainline_discovery_enabled|simulation_(run|submit|confirm)|research_job_start|analyze_|test_agent_connection)/.test(c)));assert.deepEqual(await t.evaluate('window.__themeMock.rejected'),[]);assert.deepEqual(t.errors,[]);assert.deepEqual(ticker.errors,[]);
 t.save({result:'passed',checks,platformFonts,commands,limitation:'Real Vue SFCs in isolated Edge WebViews with synthetic IPC/events. No native app, quote/trade/update service or user data accessed.'});console.log('Elegant theme UI passed: '+checks.length+' checks; '+t.output);
}catch(error){await t.screenshot('failure').catch(()=>{});t.save({result:'failed',checks,error:String(error),calls:await t.evaluate('window.__themeMock?.calls').catch(()=>[])});throw error;}finally{await ticker?.close();await t.close();}