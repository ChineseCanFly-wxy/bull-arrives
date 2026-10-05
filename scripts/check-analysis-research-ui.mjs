import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';

// Actual AnalysisDialog + actual Pinia analysis store + actual workbench. Read IPC mocked.
const root=fileURLToPath(new URL('..',import.meta.url));
const target=path.join(root,'src-tauri','target');
const profile=mkdtempSync(path.join(target,'analysis-research-ui-check-'));
const edge=process.env.NEWS_TEST_BROWSER||'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const analysis={total_score:47,verdict:'中性',factors:[{name:'形态描述',score:47,weight:1,note:'不是模型预测'}],close:10,ma20:9.9,ma60:9.5,macd_dif:null,rsi12:null,boll_upper:null,boll_lower:null,momentum20:.02,momentum60:.1,volume_ratio:1,
  trade_plan:null,backtest:null,levels:[],rule_match:null,history:{source:'local_stockdb',source_label:'冻结测试StockDB',start_date:'2026-01-01',end_date:'2026-09-30',bars:180,adjustment:'qfq',stale:false,warning:null},agent_context_fingerprint:'a'.repeat(64),
  research_context:{model_research:{schema:'stock-research-context-v1',frozen_as_of:'2026-09-30',requested_as_of:'2026-09-30',date_matches:true,exploration:true,production_admission:false,evidence_sha256:'b'.repeat(64),
    models:[{id:'breadth22_h20',name:'真实22特征模型',holding_days:20,signal_status:'nonpositive_record',score:-.01661923,performance:{train:{net_return_pct:-5.51},validation:{net_return_pct:12.58},test:{net_return_pct:55.88,max_drawdown_pct:20.97},double_cost_return_pct:49.41,unique_stocks:100,closed_cycles:187}},
      {id:'index26_h20',name:'真实26特征模型',holding_days:20,signal_status:'nonpositive_record',score:-.047863,performance:{train:{net_return_pct:-8},validation:{net_return_pct:7.6},test:{net_return_pct:11,max_drawdown_pct:25},double_cost_return_pct:8,unique_stocks:80,closed_cycles:140}}],limitations:['来源版本已冻结，收益标签不是胜率']},
    recent_news:[{id:'news:1',title:'<img src=x onerror=window.__analysisXss=true>真实测试公告',body:'<script>window.__analysisXss=true</script>原文摘录',source:'发行方',received_at:'2026-10-01T01:00:00Z',published_at:null,published_date:'2026-10-01',publication_precision:'date',published_after_market_asof:true,source_index_only:true,url:'javascript:alert(1)'},
      {id:'news:2',title:'合法来源',body:'另一原文',source:'官方公告',received_at:1790769600000,published_at:'2026-09-30T07:10:00Z',published_after_market_asof:true,source_index_only:false,url:'https://example.com/report'}],
    financial_research:{available:true,message:'保守公告版本；完整历史修订链未认证',fields:{report_date:20260630,first_notice_date:20260825,revision_date:20260825,available_signal_date:20260826,revenue_yoy_pct:5,profit_yoy_pct:-12,roe_pct:4,cash_per_share_cny:.8,debt_assets_pct:30},limitations:['不能把财务较好转成买入许可']},
    mainlines:[{sector_name:'电子',as_of:'2026-09-30'}],news_scope:'只解释本机最近七日原文，覆盖不完整',legacy_rule_note:'旧规则不是模型准入'}};
analysis.research_context.model_research.latest_model_scan={as_of:'2026-09-30',message:'实际筛选',models:[{model_id:'breadth22_rank20',name:'排序模型',score_semantic:'排序标签，非胜率',score:.59,threshold:.6,signal_status:'nonpositive_record',job_id:42,score_source:'同日真实筛选'},{model_id:'breadth22_excess_csi20',name:'超额模型',score_semantic:'相对指数超额，可能绝对亏损',score:.0123,threshold:0,signal_status:'positive_record',job_id:42,score_source:'同日真实筛选'}]};
const mock=`
window.__analysisMock={calls:[],variant:'normal',pending:{},unexpected:[],rows:${JSON.stringify(analysis)}};
export async function invoke(command,args){
 const state=window.__analysisMock;state.calls.push({command,args:args?JSON.parse(JSON.stringify(args)):args});
 if(command==='analyze_stock'){
  const row=structuredClone(state.rows);row.research_context.legacy_rule_note='个股 '+args.symbol+' 的研究证据';
  if(state.variant==='network-error')throw Error('local history unavailable');
  if(state.variant==='stale'){row.research_context.model_research.date_matches=false;row.research_context.model_research.models.forEach(m=>{m.signal_status='date_mismatch';m.score=null;});}
  if(state.variant==='unknown'){row.research_context.model_research.models.forEach(m=>{m.signal_status='not_in_scored_export';m.score=null;});}
  if(state.variant==='fallback'){row.research_context.model_research={models:[],error:'冻结研究指纹不符，评分不可用',limitations:['不伪造模型分值']};row.research_context.news_error='相关原文读取失败';row.research_context.mainline_error='主线快照无效';}
  if(state.variant==='delayed')return new Promise(resolve=>{state.pending[args.symbol]=()=>resolve(row);});
  return row;
 }
 if(command==='run_stock_research')return {bars:180,horizon_days:5,causal_audit_passed:true,factors:[{id:'momentum20',label:'20日动量',samples:100,rank_ic:.07,quantiles:[{quantile:1,samples:20,avg_return_pct:-.1,positive_probability:.45},{quantile:5,samples:20,avg_return_pct:1.2,positive_probability:.6}]}],patterns:[{id:'volume_breakout20',label:'20日放量突破',samples:12,positive_probability:.583,avg_return_pct:.9,max_drawdown_pct:8.2}],stratification_status:'unavailable',stratification_note:'缺历史行业与市值，不能按当前标签回填',intraday_status:'unavailable',intraday_note:'缺真实分钟历史，不生成盘中统计',runtime:'隔离测试'};
 if(command==='get_agent_status')return {installed:true,state:'ready',path:null,message:'模拟可用状态',guidance:'测试不会运行Claude',run_dir:null};
 if(command==='analyze_stock_agent')return {status:'ready',provider:'claude_code',cached:false,conclusion:'cautious',summary:'当前模型偏谨慎，基本面改善与现金流仍有冲突，先核对已有模型依据。',claims:[{kind:'risk',text:'模型意见偏谨慎，不能把技术评分转成买入依据。',evidence:[{field:'total_score',value:'47',source:'冻结测试StockDB',as_of:'2026-09-30'}]},{kind:'watch',text:'等待形态确认后再核对原文与现金流。',evidence:[{field:'close',value:'10',source:'冻结测试StockDB',as_of:'2026-09-30'}]}],evidence:[],confidence:50,invalidation_conditions:[],error:null,guidance:null,generated_at:'2026-10-03T08:00:00+08:00',context_fingerprint:args.contextFingerprint};
 if(command==='list_interactive_analyses')return [];
 if(command==='get_agent_activity')return {current:[],runs:[]};
 if(command==='inspect_agent_task')return {prompts:[],input_json:'{"frozen":true}',schema_json:'{}',missing_data:['无PIT财务'],timeout_seconds:120,budget_usd:'10',executable:null,history_summary:'冻结研究测试'};
 state.unexpected.push(command);throw Error('Unexpected IPC '+command);
}
export async function listen(){return ()=>{};}
export async function emit(){}
export async function enable(){} export async function disable(){} export async function isEnabled(){return false;}
`;
const server=await createServer({configFile:false,root,cacheDir:path.join(profile,'vite-cache'),
 optimizeDeps:{entries:[],include:['vue','pinia','naive-ui'],exclude:['@tauri-apps/api/core','@tauri-apps/api/event']},resolve:{alias:{'@':path.join(root,'src')},dedupe:['vue']},
 plugins:[vue(),{name:'analysis-research-ui-check',enforce:'pre',
  resolveId(id){if(id.startsWith('@tauri-apps/'))return '\0analysis-test-ipc';if(id==='/analysis-test-entry.js')return '\0analysis-test-entry';},
  load(id){if(id==='\0analysis-test-ipc')return mock;if(id==='\0analysis-test-entry')return `
    import {createApp,h,reactive} from 'vue';import {createPinia} from 'pinia';import Dialog from '/src/components/analysis/AnalysisDialog.vue';
    import {NConfigProvider} from 'naive-ui';import '/src/assets/styles/variables.css';import '/src/assets/workspace.css';
    import {useAnalysisStore} from '/src/stores/analysis.ts';import {useSettingsStore} from '/src/stores/settings.ts';
    const selected=reactive({show:true,symbol:'sz000001',name:'测试平安'});window.__analysisSelection=selected;
    document.documentElement.dataset.theme='light';const pinia=createPinia();const app=createApp({render:()=>h(NConfigProvider,null,()=>h(Dialog,{...selected,'onUpdate:show':value=>selected.show=value}))});app.use(pinia);
    window.__analysisStore=useAnalysisStore(pinia);useSettingsStore(pinia).settings.agent_auto_analyze='0';app.mount('#app');`;
  },configureServer(s){s.middlewares.use((req,res,next)=>{if(req.url?.split('?')[0]!=='/')return next();res.setHeader('Content-Type','text/html');res.end('<html><head><meta charset="utf-8"></head><body><div id="app"></div><script type="module" src="/analysis-test-entry.js"></script></body></html>');});}}],
 server:{host:'127.0.0.1',port:0,strictPort:false,watch:{ignored:['**/src-tauri/**']}}});
let browser,ws;const pending=new Map();let nextId=0;
try{
 await server.listen();const port=server.httpServer.address().port;
 browser=spawn(edge,['--headless=new','--disable-gpu','--disable-extensions','--no-first-run','--no-default-browser-check','--remote-debugging-port=0',`--user-data-dir=${profile}`,`http://127.0.0.1:${port}/`],{windowsHide:true,stdio:'ignore'});
 let debugPort;for(let attempt=0;attempt<100;attempt++){try{debugPort=Number(readFileSync(path.join(profile,'DevToolsActivePort'),'utf8').split('\n')[0]);break;}catch{await new Promise(r=>setTimeout(r,100));}}
 assert.ok(debugPort,'Headless browser did not start');const pages=await(await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();ws=new WebSocket(pages.find(p=>p.type==='page'&&p.url.startsWith(`http://127.0.0.1:${port}/`)).webSocketDebuggerUrl);
 await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
 ws.addEventListener('message',event=>{const result=JSON.parse(event.data);const callback=pending.get(result.id);if(callback){pending.delete(result.id);result.error?callback.reject(Error(JSON.stringify(result.error))):callback.resolve(result.result);}});
 ws.addEventListener('message',event=>{const result=JSON.parse(event.data);if(result.method==='Runtime.exceptionThrown')console.log(JSON.stringify(result.params.exceptionDetails));});
 const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++nextId;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
 const evaluate=async expression=>{const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
 const waitFor=async expression=>{for(let i=0;i<100;i++){if(await evaluate(expression))return;await new Promise(r=>setTimeout(r,100));}console.log(await evaluate(`({body:document.body.innerText,calls:window.__analysisMock?.calls})`));throw Error('UI timeout '+expression);};
 await call('Runtime.enable');await call('Emulation.setDeviceMetricsOverride',{width:1100,height:940,deviceScaleFactor:1,mobile:false});await waitFor(`document.querySelector('.model-overview')&&window.__analysisStore?.analysis`);
 assert.equal(await evaluate(`document.querySelector('.evidence-disclosure').open`),false);
 assert.equal(await evaluate(`document.querySelector('.ai-disclosure').open`),false);
 assert.equal(await evaluate(`document.querySelector('.technical-disclosure').open`),false);
 assert.equal(await evaluate(`document.querySelector('.workbench')===null`),true,'Collapsed AI does not inspect or render the workbench');
 assert.match(await evaluate(`document.querySelector('.model-overview').innerText`),/零\/负分，偏谨慎/);
 const preview=path.join(target,'ui-previews');mkdirSync(preview,{recursive:true});
 await waitFor(`!window.__analysisStore.loading`);await new Promise(r=>setTimeout(r,350));
 const shot=await call('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(preview,'stock-analysis-collapsed.png'),Buffer.from(shot.data,'base64'));
 const helpPosition=await evaluate(`(()=>{const r=document.querySelector('[aria-label="模型研究使用说明"]').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2,next:document.querySelector('.model-overview').getBoundingClientRect().top}})()`);
 await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:helpPosition.x,y:helpPosition.y});
 await waitFor(`!!document.querySelector('.help-tooltip-content[role="tooltip"]')&&document.querySelector('[aria-label="模型研究使用说明"]').hasAttribute('aria-describedby')`);
 assert.equal(await evaluate(`document.querySelector('.model-overview').getBoundingClientRect().top`),helpPosition.next,'Tooltip does not expand text into the page');
 const tipPosition=await evaluate(`(()=>{const r=document.querySelector('.help-tooltip-content').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
 await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:tipPosition.x,y:tipPosition.y});await new Promise(r=>setTimeout(r,300));assert.equal(await evaluate(`document.querySelector('[aria-label="模型研究使用说明"]').hasAttribute('aria-describedby')`),true,'Moving onto tooltip keeps it readable');
 await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
 await waitFor(`!document.querySelector('[aria-label="模型研究使用说明"]').hasAttribute('aria-describedby')`);assert.equal(await evaluate(`window.__analysisSelection.show`),true,'Escape dismisses tooltip without closing analysis');
 await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:5,y:5});await evaluate(`document.querySelector('[aria-label="模型研究使用说明"]').focus()`);
 await waitFor(`document.querySelector('[aria-label="模型研究使用说明"]').hasAttribute('aria-describedby')`);await evaluate(`document.querySelector('[aria-label="模型研究使用说明"]').blur()`);
 await waitFor(`!document.querySelector('[aria-label="模型研究使用说明"]').hasAttribute('aria-describedby')`);
 await evaluate(`document.querySelector('.evidence-disclosure > summary').click()`);await waitFor(`document.querySelector('.evidence-disclosure').open`);
 assert.match(await evaluate(`document.querySelector('.research-card').innerText`),/真实标签预测为零或负.*[\s\S]*-1.6619%（非胜率）/);
 assert.match(await evaluate(`document.querySelector('.research-card').innerText`),/前段 -5.51%.*[\s\S]*后段回撤 20.97%.*[\s\S]*双成本 \+49.41%/);
 await evaluate(`document.querySelectorAll('.research-card details summary').forEach(el=>el.click())`);
 assert.match(await evaluate(`document.querySelector('.evidence-disclosure').innerText`),/本次筛选模型证据[\s\S]*排序模型 · 未过阈值[\s\S]*0.59[\s\S]*超额模型 · 满足条件[\s\S]*任务 #42/);

 for(const text of ['报告期 20260630','利润累计同比 -12%','完整历史修订链未认证'])assert.ok(await evaluate(`document.querySelector('.research-card').innerText.includes(${JSON.stringify(text)})`));
 assert.match(await evaluate(`document.querySelector('.research-card').innerText`),/2026-10-01（仅日期，具体公开时点未知）.*[\s\S]*行情截止后的新信息.*[\s\S]*标题索引；原文正文尚未采集/);
 assert.equal(await evaluate(`document.querySelectorAll('.research-card article a').length`),1,'Only safe http(s) sources have actionable links');
 assert.equal(await evaluate(`document.querySelector('.research-card article a').href`),'https://example.com/report');
 assert.equal(await evaluate(`document.querySelectorAll('.research-card img,.research-card script').length`),0,'Source HTML rendered as text');
 assert.equal(await evaluate(`window.__analysisXss`),undefined);
 assert.equal(await evaluate(`window.__analysisMock.calls.some(c=>/^(analyze_stock_agent|analyze_stock_team|start_live_analysis|start_interactive_analysis|run_stock_research)$/.test(c.command))`),false,'Mounting stock evidence never runs Claude or a strategy study');
 assert.equal(await evaluate(`document.querySelector('.research-card').textContent.includes('分钟形态确认')`),false,'Removed minute feature is not presented as current stock evidence');
 const change=async(variant,symbol)=>{await evaluate(`window.__analysisMock.variant=${JSON.stringify(variant)};window.__analysisSelection.symbol=${JSON.stringify(symbol)}`);if(!['network-error','delayed'].includes(variant)){await waitFor(`!window.__analysisStore.loading&&window.__analysisStore.symbol===${JSON.stringify(symbol)}`);assert.equal(await evaluate(`document.querySelector('.ai-disclosure').open`),false,'Changing the stock closes AI');await evaluate(`document.querySelector('.evidence-disclosure > summary').click()`);await waitFor(`document.querySelector('.evidence-disclosure').open`);}};
 await change('stale','sh600000');await waitFor(`document.querySelector('.research-card')?.innerText.includes('日期不符，当前评分未知')`);
 assert.equal(await evaluate(`document.querySelector('.research-card').innerText.includes('标签预测 ')`),false,'Across-date null score is not reused');
 await change('unknown','sh600001');await waitFor(`document.querySelector('.research-card')?.innerText.includes('无对应评分记录，当前方向未知')`);
 assert.equal(await evaluate(`document.querySelector('.research-card').innerText.includes('标签预测 ')`),false,'Unknown score is not invented');
 await change('fallback','sh600002');await waitFor(`document.querySelector('.research-card')?.innerText.includes('冻结研究指纹不符')`);
 assert.match(await evaluate(`document.querySelector('.research-card').innerText`),/主线关联读取失败.*[\s\S]*近期资讯读取失败/);
 assert.equal(await evaluate(`document.querySelectorAll('.research-card>div.research-row').length`),0,'Failed evidence never preserves previous model scores');
 await change('network-error','sh600003');await waitFor(`document.querySelector('.analysis>.n-spin-container .error-line')?.innerText.includes('local history unavailable')||document.querySelector('.analysis .error-line')?.innerText.includes('local history unavailable')`);
 assert.equal(await evaluate(`document.querySelectorAll('.research-card').length`),0,'Failed stock request clears old research card');
 await change('delayed','sh600004');await waitFor(`!!window.__analysisMock.pending.sh600004`);await change('delayed','sh600005');await waitFor(`!!window.__analysisMock.pending.sh600005`);
 await evaluate(`window.__analysisMock.pending.sh600005()`);await waitFor(`document.querySelector('.research-card')?.textContent.includes('个股 sh600005 的研究证据')`);
 await evaluate(`(async()=>{window.__analysisMock.pending.sh600004();await new Promise(r=>setTimeout(r,100));})()`);
 assert.equal(await evaluate(`window.__analysisStore.symbol`),'sh600005');
 assert.equal(await evaluate(`window.__analysisStore.analysis.research_context.legacy_rule_note`),'个股 sh600005 的研究证据','Late result for old symbol does not poison actual Pinia store');
 assert.equal(await evaluate(`window.__analysisMock.calls.some(c=>/^(analyze_stock_agent|analyze_stock_team|start_live_analysis|start_interactive_analysis|run_stock_research)$/.test(c.command))`),false,'Variants and symbol changes remain read-only');
 assert.deepEqual(await evaluate(`window.__analysisMock.unexpected`),[],'Mock allowlist covers all real component reads');
 await evaluate(`document.querySelector('.ai-disclosure > summary').click()`);await waitFor(`document.querySelector('.agent-actions')`);
 assert.equal(await evaluate(`window.__analysisMock.calls.filter(c=>c.command==='analyze_stock_agent').length`),0,'Expanding AI does not generate an answer');
 await evaluate(`document.querySelector('.agent-actions .agent-btn').click()`);await waitFor(`document.querySelector('.agent-conclusion')?.innerText==='谨慎'`);
 assert.equal(await evaluate(`window.__analysisMock.calls.filter(c=>c.command==='analyze_stock_agent').length`),1,'Manual generation runs once');
 assert.equal(await evaluate(`window.__analysisMock.calls.find(c=>c.command==='analyze_stock_agent').args.contextFingerprint`),'a'.repeat(64));
 assert.match(await evaluate(`document.querySelector('.agent-claim').innerText`),/综合评分 = 47.*冻结测试StockDB.*2026-09-30/);
 await evaluate(`document.querySelector('.ai-disclosure > summary').click()`);await waitFor(`document.querySelector('.ai-disclosure').open===false`);await evaluate(`document.querySelector('.ai-disclosure > summary').click()`);await waitFor(`document.querySelector('.agent-conclusion')`);
 assert.equal(await evaluate(`window.__analysisMock.calls.filter(c=>c.command==='analyze_stock_agent').length`),1,'Reopening a ready answer does not call Claude again');
 assert.equal(await evaluate(`document.querySelector('.statistics-disclosure').open`),false,'Single-stock statistics are optional and collapsed');
 assert.equal(await evaluate(`window.__analysisMock.calls.filter(c=>c.command==='run_stock_research').length`),0,'No passive statistics run');
 await evaluate(`document.querySelector('.statistics-disclosure > summary').click()`);
 await waitFor(`document.querySelector('.statistics-disclosure').open`);
 assert.match(await evaluate(`document.querySelector('.statistics-description').innerText`),/未做滚动样本外检验[\s\S]*未计交易成本/);
 assert.equal(await evaluate(`window.__analysisMock.calls.filter(c=>c.command==='run_stock_research').length`),0,'Expanding statistics does not run or trade');
 await evaluate(`document.querySelector('.statistics-disclosure .agent-btn').click()`);
 await waitFor(`document.querySelector('.stock-statistics-card')`);
 assert.equal(await evaluate(`window.__analysisMock.calls.filter(c=>c.command==='run_stock_research').length`),1,'Explicit statistics request runs once');
 assert.equal(await evaluate(`window.__analysisMock.calls.find(c=>c.command==='run_stock_research').args.symbol`),'sh600005');
 assert.match(await evaluate(`document.querySelector('.stock-statistics-card').innerText`),/历史上涨占比[\s\S]*信号后最低价最大跌幅[\s\S]*不是交易账户回撤/);
 assert.equal(await evaluate(`window.__analysisMock.calls.some(c=>/(?:follow_create|simulation.*(?:save|order|run)|research_job_start)/.test(c.command))`),false,'Single-stock statistics never creates a model or account or orders');
 await change('normal','sh600006');await waitFor(`window.__analysisStore.symbol==='sh600006'&&!window.__analysisStore.loading`);
 assert.equal(await evaluate(`document.querySelector('.statistics-disclosure').open`),false,'Statistics collapse on symbol change');
 assert.equal(await evaluate(`document.querySelector('.stock-statistics-card')===null`),true,'Previous stock statistics are cleared');
 assert.deepEqual(await evaluate(`window.__analysisMock.unexpected`),[]);
 console.log('Analysis research UI passed: real Vue dialog/store/workbench; same-day scan job/rank/excess evidence plus true negative label and multi-year return/DD/cost, stale/unknown null score, integrity fallback and request error, date-only/post-market/title-index source flags, safe URL and escaped HTML, stale asynchronous symbol result rejected; default-collapsed evidence/AI/rules, hover/focus/Escape tooltip without layout shift, manual fingerprint-bound AI trigger and Chinese conclusion; honest optional single-stock 5-day descriptive statistics, explicit read-only calculation, historical rise fraction and signal-to-low drop, no model/account/order creation, symbol change clears statistics. IPC mocked; no actual Claude/study process, network or native GUI.');
}finally{
 ws?.close();if(browser?.exitCode===null){const exited=new Promise(r=>browser.once('exit',r));browser.kill();await Promise.race([exited,new Promise(r=>setTimeout(r,2000))]);}await server.close();
 assert.equal(path.dirname(path.resolve(profile)),path.resolve(target));assert.ok(path.basename(profile).startsWith('analysis-research-ui-check-'));try{rmSync(profile,{recursive:true,force:true,maxRetries:10,retryDelay:100});}catch{console.log(`Temporary browser profile remains in ${profile}`);}
}
