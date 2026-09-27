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
const directEntry = { ...entry, id: 'news-manual', signal_id: 'news:manual', agent_summary: false, news_mode: 'direct', body: '原文', original_title: '公司利润预亏' };
const mock = `
const handlers = new Map();
const rows = ${JSON.stringify([entry, directEntry])};
const briefs = [{day:'2026-09-27',stage:'postclose',generated_at:'2026-09-27T07:17:00Z',window_start:'2026-09-27T01:10:00Z',window_end:'2026-09-27T07:17:00Z',news_count:1,alert_count:1,major:[],watchlist:[{signal_id:'news:manual',title:'公司利润预亏',excerpt:'原文',source:'测试',tag:'业绩',received_at:Date.now()}],other:[],alerts:[{title:'价格提醒',body:'价格触发',received_at:Date.now(),kind:'price'}]}];
export const calls = [];
export async function invoke(command, args) {
  calls.push({command, args});
  if(command === 'get_notification_history') return {version:0,entries:rows};
  if(command === 'get_news_archive') return rows;
  if(command === 'get_daily_briefs') return briefs;
  if(command === 'get_news_ai_usage') return {day:'2026-09-27',used:1,limit:3};
  if(command === 'analyze_archived_news') { const found=rows.find(row=>row.signal_id===args.signalId); found.agent_summary=true; found.body=${JSON.stringify(body)}; emit('news-analysis-updated',found); return found; }
  if(command === 'set_setting' || command === 'dismiss_desktop_toast' || command === 'view_desktop_toast') return;
  if(command === 'desktop_toast_ready') { setTimeout(() => emit('desktop-toast-show', {id:'long-toast',title:'很长的资讯标题'.repeat(20),body:${JSON.stringify(body.repeat(12))}}),0); return; }
  throw Error('Unexpected IPC: '+command);
}
export async function listen(name, callback) { handlers.set(name, callback); return () => handlers.delete(name); }
export async function emit(name, payload) { handlers.get(name)?.({payload}); }
export async function enable() {} export async function disable() {} export async function isEnabled() { return false; }
window.__newsMock={calls,emit};
`;
const server = await createServer({
  configFile: false, root, cacheDir:path.join(profile,'vite-cache'),
  optimizeDeps:{exclude:['@tauri-apps/api/core','@tauri-apps/api/event','@tauri-apps/plugin-autostart']},
  resolve: { alias: { '@': path.join(root, 'src') } },
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
  const page = pages.find(p=>p.type==='page');
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
  await waitFor(`!!document.querySelector('.news-mode') && !!document.querySelector('.notice-list li')`);
  assert.equal(await evaluate(`document.querySelector('.news-tabs').innerText.includes('盘前 / 盘后')`), false, 'Removed timeline tab must stay hidden');
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='每日简报').click()`);
  await waitFor(`document.querySelector('.brief-card')?.innerText.includes('公司利润预亏')`);
  assert.ok(await evaluate(`document.querySelector('.brief-card').innerText.includes('价格提醒')`));
  assert.ok(await evaluate(`!!document.querySelector('.brief-card .manual-ai-btn')`));
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='市场资讯').click()`);
  assert.match(await evaluate(`document.querySelector('.notice-list').innerText`), /观点：.*[\s\S]*利空.*[\s\S]*制造业（推测）.*600001/);
  assert.equal(await evaluate(`document.querySelector('.news-mode button').getAttribute('aria-pressed')`),'true');
  await evaluate(`document.querySelectorAll('.news-mode button')[1].click()`);
  await waitFor(`document.querySelectorAll('.news-mode button')[1].getAttribute('aria-pressed')==='true'`);
  assert.ok(await evaluate(`window.__newsMock.calls.some(c=>c.command==='set_setting'&&c.args.key==='news_notification_mode'&&c.args.value==='hybrid')`));
  await evaluate(`document.querySelectorAll('.news-mode button')[0].click()`);
  await waitFor(`document.querySelectorAll('.news-mode button')[0].getAttribute('aria-pressed')==='true'`);
  await evaluate(`window.__newsMock.emit('notification-open-history','news-check')`);
  await waitFor(`!!document.querySelector('.notice-list li.selected')`);
  await evaluate(`document.querySelector('.notice-list summary').click()`);
  assert.equal(await evaluate(`document.querySelectorAll('.notice-list img').length`),0,'Source text must stay escaped');
  await evaluate(`document.querySelector('.manual-ai-btn').click()`);
  await waitFor(`window.__newsMock.calls.some(c=>c.command==='analyze_archived_news'&&c.args.signalId==='news:manual')`);
  await waitFor(`document.querySelectorAll('.notice-list li')[1].innerText.includes('观点：')`);
  await call('Page.navigate',{url:`http://127.0.0.1:${port}/?toast`});
  await call('Emulation.setDeviceMetricsOverride',{width:360,height:142,deviceScaleFactor:1,mobile:false});
  await waitFor(`!!document.querySelector('.toast section button')`);
  const layout=await evaluate(`(()=>{const button=document.querySelector('.toast section button');const b=button.getBoundingClientRect();return {x:b.left,y:b.top,right:b.right,bottom:b.bottom,text:button.innerText,width:innerWidth,height:innerHeight};})()`);
  assert.equal(layout.text,'查看');
  assert.ok(layout.x>=0&&layout.y>=0&&layout.right<=layout.width&&layout.bottom<=layout.height,`View button cropped: ${JSON.stringify(layout)}`);
  await evaluate(`document.querySelector('.toast section button').click()`);
  await waitFor(`window.__newsMock.calls.some(c=>c.command==='view_desktop_toast'&&c.args.id==='long-toast')`);
  console.log('News UI check passed: mode persistence, complete AI text, history navigation, escaped source, long-toast view button. IPC mocked; native Windows notification not tested.');
} finally {
  ws?.close();
  if(browser?.exitCode===null) { const exited=new Promise(resolve=>browser.once('exit',resolve)); browser.kill(); await Promise.race([exited,new Promise(resolve=>setTimeout(resolve,2000))]); }
  await server.close();
  assert.equal(path.dirname(path.resolve(profile)), path.resolve(target));
  assert.ok(path.basename(profile).startsWith('news-ui-check-'));
  try { rmSync(profile,{recursive:true,force:true,maxRetries:10,retryDelay:100}); } catch { console.log(`Temporary browser profile remains in ${profile}`); }
}
