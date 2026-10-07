import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';

// Render the real Vue components in an isolated headless browser; IPC is mocked.
const root = fileURLToPath(new URL('..', import.meta.url));
const target = path.join(root, 'src-tauri', 'target');
const profile = mkdtempSync(path.join(target, 'news-ui-check-'));
const edge = process.env.NEWS_TEST_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const body = '事实：利润预亏\n观点：盈利压力偏利空，实际影响仍需核实\n方向：利空，影响：中\n行业：制造业（推测）；股票：600001\n依据：利润预亏';
const entry = { id: 'news-check', signal_id: 'news-check', signal_kind: 'news', title: '★自选 测试自选', body, received_at: Date.now(), history_version: 0, agent_summary: true, news_mode: 'ai', news_source: '测试', original_title: '<img src=x onerror=alert(1)>利润预亏' };
const targetRow={kind:'industry',sector_code:'SW801080',sector_name:'电子',as_of:'2026-09-30',fingerprint:'f'.repeat(64),match_basis:'structured_symbol'};
const directEntry = { ...entry, id: 'news-manual', signal_id: 'news:manual', agent_summary: false, news_mode: 'hybrid', body: '原文', original_title: '公司利润预亏',related_mainlines:[targetRow],published_at:'2026-09-30T07:20:00Z',source_received_at:'2026-09-30T07:21:00Z' };
const researchEntry={id:'persisted-research',signal_id:'mainline:SW801080:2026-09-30',signal_kind:'research',title:'历史主线观察',body:'只保存通知当时的证据',received_at:'2026-09-30T07:25:00Z',history_version:0,...targetRow};
const modelEvent={kind:'model_research_observation',run_id:42,as_of:'2026-09-30',historical_catchup:true,signals:[{symbol:'<img src=x onerror=window.__modelNoticeXss=true>',score:.0123,threshold:0,as_of:'2026-09-30',close:10,cash_reference_quantity:100,cash_reference_eligible:true,reason:'冻结观察，下一开盘未知'}],message:'只保存通知时模型观察'};
const modelEntry={id:'persisted-model',signal_kind:'research',title:'模型观察更新',body:'没有自动交易',received_at:Date.now(),history_version:0,model_snapshot:modelEvent};
const conditionEvent={schema:'model-condition-event-v1',event_key:'condition-saved-1',event:'conditions_confirmed',symbol:'sz002672',model_name:'广度模型',model_id:'breadth22_h20',preset:'model_confirm',as_of:'2026-09-29',checked_at:'2026-09-30T02:10:00Z',production_admission:false,message:'<img src=x onerror=window.__conditionNoticeXss=true>模型和分钟新确认',evidence:{candidate:{score:.0123,threshold:0,close:10},model_group:{as_of:'2026-09-29',model_sha256:'a'.repeat(64)},intraday_snapshot:{state:'confirmed',checked_at:'2026-09-30T02:10:00Z'}}};
const conditionEntry={id:'persisted-condition',signal_kind:'research',title:'组合条件新成立',body:'查看触发时证据',received_at:Date.now(),history_version:0,condition_event:conditionEvent,condition_events:[conditionEvent]};
const intradayEntry={id:'persisted-intraday',signal_kind:'research',title:'分钟形态确认',body:'形态观察未盈利准入',received_at:Date.now(),history_version:0,intraday_snapshot:{schema:'ashare-intraday-observation-v1',symbol:'sz002672',frozen_as_of:'2026-09-29',as_of:'2026-09-30',checked_at:'2026-09-30T02:10:00Z',message:'<img src=x onerror=window.__intradayNoticeXss=true>确认仅研究',state:'confirmed',execution_plan:{initial:'40%研究仓位',reduce_weakness:'T+1不可当天卖'}}};
const frozenSnapshot={kind:'industry',sector_code:'SW801080',sector_name:'电子',as_of:'2026-09-30',member_as_of:'2026-09-30',sector_source:'官方发行人',member_source:'当前发布者名单',total_members:10,excluded_members:0,covered_members:9,missing_members:[{symbol:'sh600000',reason:'缺少行情'}],complete:false,strong:false,status:'留存不完整，不提醒',metrics:{},leaders:[],news:[],limitations:['主线只是辅助观察，不是准入'],fingerprint:targetRow.fingerprint,content_sha256:'a'.repeat(64),generated_at:'2026-09-30T07:25:00Z'};
const mock = `
const handlers = new Map();
const rows = ${JSON.stringify([entry, directEntry])};
const briefs = [{day:'2026-09-27',stage:'postclose',generated_at:'2026-09-27T07:17:00Z',window_start:'2026-09-27T01:10:00Z',window_end:'2026-09-27T07:17:00Z',news_count:1,alert_count:1,major:[],watchlist:[{signal_id:'news:manual',title:'公司利润预亏',excerpt:'原文',source:'测试',tag:'业绩',received_at:Date.now()}],other:[],alerts:[{title:'价格提醒',body:'价格触发',received_at:Date.now(),kind:'price'}]}];
let operations={enabled:true,version:0,entries:[{id:1,at:Date.now(),level:"正常",source:"自动模型",message:"开始检查原模型；复用已有账户",repeats:1},{id:2,at:Date.now(),level:"注意",source:"数据更新",message:"网络暂不可用，等待后台重试 <img src=x onerror=alert(1)>",repeats:2}]};
export const calls = [];
export async function invoke(command, args) {
  calls.push({command, args});
  if(command === 'get_automatic_operations_log') return structuredClone(operations);
  if(command === 'clear_automatic_operations_log') {operations.entries=[];operations.version++;return structuredClone(operations);}
  if(command === 'get_notification_history') return {version:0,entries:rows};
  if(command === 'get_news_archive') return rows;
  if(command === 'get_daily_briefs') return briefs;
  if(command === 'get_model_condition_event') return ${JSON.stringify(conditionEvent)};
  if(command === 'get_mainline_alert_history') return ${JSON.stringify([researchEntry,modelEntry,intradayEntry,conditionEntry])};
  if(command === 'get_sector_mainline')return ${JSON.stringify(frozenSnapshot)};
  if(command === 'get_mainline_watchlist')return [];
  if(command === 'analyze_archived_news') { const found=rows.find(row=>row.signal_id===args.signalId); found.agent_summary=true; found.body=${JSON.stringify(body)}; emit('news-analysis-updated',found); return found; }
  if(command === 'set_setting') {if(args.key==='automatic_operations_log_enabled'){operations.enabled=args.value==='1';operations.version++;}return;}
  if(command === 'dismiss_desktop_toast' || command === 'view_desktop_toast') return;
  if(command === 'desktop_toast_ready') { setTimeout(() => emit('desktop-toast-show', {id:'long-toast',title:'很长的资讯标题'.repeat(20),body:${JSON.stringify(body.repeat(12))}}),0); return; }
  throw Error('Unexpected IPC: '+command);
}
export async function listen(name, callback) { handlers.set(name, callback); return () => handlers.delete(name); }
export async function emit(name, payload) { handlers.get(name)?.({payload}); }
export async function enable() {} export async function disable() {} export async function isEnabled() { return false; }
export async function openUrl(url){calls.push({command:'openUrl',url});}
window.__newsMock={calls,emit,addOperation(message){if(operations.enabled){operations.version++;operations.entries.unshift({id:operations.version+10,at:Date.now(),source:"模拟交易",level:"正常",message,repeats:1});}}};
`;
const server = await createServer({
  configFile: false, root, cacheDir:path.join(profile,'vite-cache'),
  optimizeDeps:{entries:[],include:['vue','pinia','naive-ui'],exclude:['@tauri-apps/api/core','@tauri-apps/api/event','@tauri-apps/plugin-autostart']},
  resolve: { alias: { '@': path.join(root, 'src') },dedupe:['vue'] },
  plugins: [vue(), {
    name: 'news-ui-check',
    enforce:'pre',
    resolveId(id) { if(id.startsWith('@tauri-apps/')) return '\0news-test-ipc'; if(id === '/news-test-entry.js') return '\0news-test-entry'; },
    load(id) {
      if(id === '\0news-test-ipc') return mock;
      if(id === '\0news-test-entry') return `
        import {createApp,h} from 'vue'; import {createPinia} from 'pinia'; import {NMessageProvider} from 'naive-ui';
        import Notices from '/src/components/settings/AlertNotifications.vue'; import Toast from '/src/components/notifications/ToastWindow.vue';
        createApp({render:()=>location.search ? h(Toast) : h(NMessageProvider,null,{default:()=>h(Notices)})}).use(createPinia()).mount('#app');
      `;
    },
    configureServer(s) { s.middlewares.use((req,res,next) => { if(req.url?.split('?')[0] !== '/') return next(); res.setHeader('Content-Type','text/html'); res.end('<html><head><meta charset="utf-8"></head><body><div id="app"></div><script type="module" src="/news-test-entry.js"></script></body></html>'); }); },
  }],
  server: {host:'127.0.0.1', port:0, strictPort:false,watch:{ignored:['**/src-tauri/**']}},
});
let browser;
let ws;
const pending = new Map();
let nextId = 0;
try {
  await server.listen();
  const port = server.httpServer.address().port;
  browser = spawn(edge, ['--headless=new','--disable-gpu','--disable-extensions','--no-first-run','--no-default-browser-check','--remote-debugging-port=0',`--user-data-dir=${profile}`,`http://127.0.0.1:${port}/`], {windowsHide:true,stdio:'ignore'});
  let debugPort;
  for(let attempt=0; attempt<100; attempt++) {
    try { const {readFileSync}=await import('node:fs'); debugPort=Number(readFileSync(path.join(profile,'DevToolsActivePort'),'utf8').split('\n')[0]); break; } catch { await new Promise(resolve=>setTimeout(resolve,100)); }
  }
  assert.ok(debugPort, 'Headless browser did not start');
  const pages = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
  const page = pages.find(p=>p.type==='page'&&p.url.startsWith(`http://127.0.0.1:${port}/`));assert.ok(page,'Local test page must exist; exclude unrelated welcome tabs');
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
  ws.addEventListener('message',event=>{const result=JSON.parse(event.data);const callback=pending.get(result.id);if(callback){pending.delete(result.id);result.error?callback.reject(Error(JSON.stringify(result.error))):callback.resolve(result.result);}});
  const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++nextId;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async expression=>{const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
  ws.addEventListener('message',event=>{const result=JSON.parse(event.data);if(result.method==='Runtime.exceptionThrown')console.log(JSON.stringify(result.params.exceptionDetails));});
  await call('Runtime.enable');
  const waitFor=async expression=>{for(let attempt=0;attempt<100;attempt++){if(await evaluate(expression))return;await new Promise(resolve=>setTimeout(resolve,100));}throw Error('UI timeout: '+expression);};
  await waitFor(`!!document.querySelector('.alert-history-button') && !!window.__newsMock`);
  await evaluate(`document.querySelector('.alert-history-button').click()`);
  await waitFor(`!!document.querySelector('.news-intro') && !!document.querySelector('.notice-list li')`);
  assert.equal(await evaluate(`window.__newsMock.calls.some(c=>c.command==='get_sector_mainline'||c.command==='analyze_mainline')`),false,'Reading alerts does not scan or call Claude');
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='自动操作').click()`);
  await waitFor(`document.querySelectorAll('.operations-list li').length===2`);
  assert.equal(await evaluate(`document.querySelectorAll('.operations-list img').length`),0,'Automatic operation text stays escaped');
  assert.equal(await evaluate(`!!document.querySelector('.news-intro')`),false,'Log page has only its own switch');
  await evaluate(`document.querySelector('.operations-toggle [role="switch"]').click()`);
  await waitFor(`document.querySelector('.operations-toolbar')?.innerText.includes('记录已关闭')`);
  assert.deepEqual(await evaluate(`window.__newsMock.calls.filter(c=>c.command==='set_setting').map(c=>c.args)`),[{key:'automatic_operations_log_enabled',value:'0'}],'Log switch never changes notification or trading settings');
  await evaluate(`window.__newsMock.addOperation('记录关闭期间不补记')`);
  await evaluate(`document.querySelector('.operations-toggle [role="switch"]').click()`);
  await waitFor(`document.querySelector('.operations-toolbar')?.innerText.includes('记录已开启')`);
  assert.equal(await evaluate(`document.querySelectorAll('.operations-list li').length`),2,'Re-enabling never backfills disabled operations');
  await evaluate(`window.__newsMock.addOperation('新买入结果会出现在自动操作记录')`);
  await waitFor(`document.querySelectorAll('.operations-list li').length===3`);
  await evaluate(`document.querySelector('.operations-toolbar button').click()`);
  await waitFor(`!document.querySelector('.operations-list')`);
  assert.equal(await evaluate(`document.querySelector('.operations-toolbar')?.innerText.includes('记录已开启')`),true,'Clearing log preserves recording switch');
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='市场资讯').click()`);
  const previousLogReads=await evaluate(`window.__newsMock.calls.filter(c=>c.command==='get_automatic_operations_log').length`);
  await new Promise(resolve=>setTimeout(resolve,2200));
  assert.equal(await evaluate(`window.__newsMock.calls.filter(c=>c.command==='get_automatic_operations_log').length`),previousLogReads,'Leaving log page stops polling');

  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='研究归档').click()`);
  await waitFor(`document.querySelector('.notice-list')?.innerText.includes('历史主线观察')`);
  assert.match(await evaluate(`document.querySelector('.notice-list').innerText`),/通知快照/);
  assert.equal(await evaluate(`!!document.querySelector('.ai-filter')`),false,'Persisted research is independent of news AI filter');
  await evaluate(`window.__newsMock.emit('notification-open-history','persisted-research')`);
  await waitFor(`document.querySelector('.notice-list li.selected')?.dataset.noticeId==='persisted-research'`);
  assert.equal(await evaluate(`document.querySelector('.news-tabs button.active').innerText`),'研究归档','Old notification navigation reads persistent archive after restart');
  await evaluate(`document.querySelector('.notice-mainlines button').click()`);
  await waitFor(`document.querySelector('.mainline-archived')?.innerText.includes('历史快照')`);
  assert.deepEqual(await evaluate(`window.__newsMock.calls.find(c=>c.command==='get_sector_mainline').args`),{kind:'industry',sectorCode:'SW801080',sectorName:'电子',fingerprint:'f'.repeat(64)});
  assert.equal(await evaluate(`window.__newsMock.calls.some(c=>c.command==='analyze_mainline'||c.command==='analyze_archived_news')`),false,'Snapshot navigation never calls AI');
  await evaluate(`Array.from(document.querySelectorAll('.n-card-header__close')).at(-1).click()`);
  await waitFor(`!document.querySelector('.mainline-research')`);
  await evaluate(`document.querySelector('.notice-model-open').click()`);
  await waitFor(`document.querySelector('.notice-model-snapshot')?.innerText.includes('账户 #42')`);
  assert.match(await evaluate(`document.querySelector('.notice-model-snapshot').innerText`),/2026-09-30.*[\s\S]*历史补齐，不能称实时.*[\s\S]*0.0123.*[\s\S]*下一开盘未知/);
  assert.equal(await evaluate(`document.querySelectorAll('.notice-model-snapshot img').length`),0,'Model notification raw fields/JSON stay escaped');
  assert.equal(await evaluate(`window.__modelNoticeXss`),undefined);
  assert.equal(await evaluate(`window.__newsMock.calls.some(c=>/research_model_run|analyze_mainline|analyze_archived_news/.test(c.command))`),false,'Model notification opening never reruns account/Claude');
  await evaluate(`Array.from(document.querySelectorAll('.n-card-header__close')).at(-1).click()`);
  await waitFor(`!document.querySelector('.notice-model-snapshot')`);
  await evaluate(`document.querySelector('.notice-intraday-open').click()`);
  await waitFor(`document.querySelector('.notice-model-snapshot')?.innerText.includes('sz002672')`);
  const intradayText=await evaluate(`document.querySelector('.notice-model-snapshot').innerText`);for(const token of ['2026-09-29','T+1','40%研究仓位'])assert.ok(intradayText.includes(token));
  assert.equal(await evaluate(`document.querySelectorAll('.notice-model-snapshot img').length`),0);assert.equal(await evaluate(`window.__intradayNoticeXss`),undefined);
  assert.equal(await evaluate(`window.__newsMock.calls.some(c=>/research_model_run|analyze_mainline|analyze_archived_news/.test(c.command))`),false);
  await evaluate(`Array.from(document.querySelectorAll('.n-card-header__close')).at(-1).click()`);await waitFor(`!document.querySelector('.notice-model-snapshot')`);
  await evaluate(`document.querySelector('.notice-condition-open').click()`);await waitFor(`document.querySelector('.condition-evidence')?.innerText.includes('模型和分钟新确认')`);
  assert.match(await evaluate(`document.querySelector('.condition-evidence').innerText`),/条件新成立[\s\S]*1.2300%[\s\S]*触发时保存/);
  assert.equal(await evaluate(`document.querySelectorAll('.condition-evidence img').length`),0);assert.equal(await evaluate(`window.__conditionNoticeXss`),undefined);
  assert.equal(await evaluate(`window.__newsMock.calls.filter(c=>c.command==='get_model_condition_event').length`),1);
  await evaluate(`Array.from(document.querySelectorAll('.n-card-header__close')).at(-1).click()`);await waitFor(`!document.querySelector('.condition-evidence')`);

  assert.equal(await evaluate(`document.querySelector('.news-tabs').innerText.includes('盘前 / 盘后')`), false, 'Removed timeline tab must stay hidden');
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='每日简报').click()`);
  await waitFor(`document.querySelector('.brief-card')?.innerText.includes('公司利润预亏')`);
  assert.ok(await evaluate(`document.querySelector('.brief-card').innerText.includes('价格提醒')`));
  assert.ok(await evaluate(`!!document.querySelector('.brief-card .manual-ai-btn')`));
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='市场资讯').click()`);
  assert.equal(await evaluate(`!!document.querySelector('.ai-filter')`), false, 'Removed AI-only filter must not return');
  await waitFor(`document.querySelectorAll('.notice-list li').length===2`);
  await evaluate(`(()=>{const search=document.querySelector('.news-tabs input[placeholder]');search.value='不存在的关键词';search.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await waitFor(`document.querySelector('.empty')?.innerText.includes('没有匹配')`);
  await evaluate(`(()=>{const search=document.querySelector('.news-tabs input[placeholder]');search.value='制造业';search.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await waitFor(`document.querySelectorAll('.notice-list li').length===1`);
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='本次提醒').click()`);
  await waitFor(`document.querySelectorAll('.notice-list li').length===1`);
  await evaluate(`window.__newsMock.emit('notification-open-history','news-manual')`);
  await waitFor(`document.querySelector('.notice-list li.selected')?.dataset.noticeId==='news-manual'`);
  assert.equal(await evaluate(`!!document.querySelector('.ai-filter')`),false,'Notification navigation must not restore removed AI filter');
  assert.equal(await evaluate(`document.querySelectorAll('.notice-list li').length`),2);
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='市场资讯').click()`);
  assert.match(await evaluate(`document.querySelector('.notice-list').innerText`), /观点：.*[\s\S]*利空.*[\s\S]*制造业（推测）.*600001/);
  assert.equal(await evaluate(`document.querySelectorAll('.news-mode').length`),0,'Automatic/hybrid controls must be removed');
  assert.equal(await evaluate(`document.querySelector('.news-intro').innerText.includes('手动点击')`),true);
  assert.equal(await evaluate(`window.__newsMock.calls.some(c=>c.command==='get_news_ai_usage'||c.command==='analyze_archived_news')`),false,'Opening or filtering news must not call AI or automatic quota');
  await evaluate(`window.__newsMock.emit('notification-open-history','news-check')`);
  await waitFor(`!!document.querySelector('.notice-list li.selected')`);
  await evaluate(`document.querySelector('.notice-list summary').click()`);
  assert.equal(await evaluate(`document.querySelectorAll('.notice-list img').length`),0,'Source text must stay escaped');
  await evaluate(`document.querySelector('.manual-ai-btn').click()`);
  await waitFor(`window.__newsMock.calls.some(c=>c.command==='analyze_archived_news'&&c.args.signalId==='news:manual')`);
  await waitFor(`document.querySelectorAll('.notice-list li')[1].innerText.includes('观点：')`);
  assert.equal(await evaluate(`window.__newsMock.calls.filter(c=>c.command==='analyze_archived_news').length`),1,'Exactly one explicit click triggers the manual interpretation');
  await evaluate(`document.querySelector('[data-notice-id="news-manual"] .notice-mainlines button').click()`);
  await waitFor(`document.querySelector('.mainline-archived')?.innerText.includes('历史快照')`);
  assert.equal(await evaluate(`window.__newsMock.calls.filter(c=>c.command==='get_sector_mainline').length`),2,'Related original news opens its same stored evidence');
  assert.equal(await evaluate(`window.__newsMock.calls.some(c=>c.command==='analyze_mainline')`),false,'Related mainline navigation never invokes Claude');
  await call('Page.navigate',{url:`http://127.0.0.1:${port}/?toast`});
  await call('Emulation.setDeviceMetricsOverride',{width:360,height:142,deviceScaleFactor:1,mobile:false});
  await waitFor(`!!document.querySelector('.toast section button')`);
  const layout=await evaluate(`(()=>{const button=document.querySelector('.toast section button');const b=button.getBoundingClientRect();return {x:b.left,y:b.top,right:b.right,bottom:b.bottom,text:button.innerText,width:innerWidth,height:innerHeight};})()`);
  assert.equal(layout.text,'查看');
  assert.ok(layout.x>=0&&layout.y>=0&&layout.right<=layout.width&&layout.bottom<=layout.height,`View button cropped: ${JSON.stringify(layout)}`);
  await evaluate(`document.querySelector('.toast section button').click()`);
  await waitFor(`window.__newsMock.calls.some(c=>c.command==='view_desktop_toast'&&c.args.id==='long-toast')`);
  console.log('News UI check passed: independent automatic-log switch, clear, disabled-period exclusion, live refresh and polling cleanup; persisted research archive/restart navigation, exact sector+fingerprint frozen snapshot opening with no AI; related original-news links, ordinary news search/empty state/reset without AI-only filter, no automatic AI or hybrid controls, daily brief, one manual interpretation, escaped source and long-toast view button. IPC mocked; native Windows notification/network delivery not tested.');
} finally {
  ws?.close();
  if(browser?.exitCode===null) { const exited=new Promise(resolve=>browser.once('exit',resolve)); browser.kill(); await Promise.race([exited,new Promise(resolve=>setTimeout(resolve,2000))]); }
  await server.close();
  assert.equal(path.dirname(path.resolve(profile)), path.resolve(target));
  assert.ok(path.basename(profile).startsWith('news-ui-check-'));
  try { rmSync(profile,{recursive:true,force:true,maxRetries:10,retryDelay:100}); } catch { console.log(`Temporary browser profile remains in ${profile}`); }
}
