import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, copyFileSync, writeFileSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';
import { createServer } from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Real Tauri IPC in a portable copy; installed app and user data are untouched.
const root = fileURLToPath(new URL('..', import.meta.url));
const dir = mkdtempSync(path.join(root, 'src-tauri', 'target', 'brief-native-'));
const exe = path.join(dir, 'bull-arrives.exe');
copyFileSync(path.join(root, 'src-tauri', 'target', 'release', 'bull-arrives.exe'), exe);
writeFileSync(path.join(dir, 'portable.dat'), '');
const { mkdirSync } = await import('node:fs');
mkdirSync(path.join(dir, 'data'));
const db = new DatabaseSync(path.join(dir, 'data', 'bull-arrives.db'));
db.exec('CREATE TABLE news_archive(id TEXT PRIMARY KEY,payload TEXT NOT NULL,received_at TEXT NOT NULL); CREATE TABLE daily_briefs(day TEXT NOT NULL,stage TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(day,stage));');
const news = {id:'brief-news',signal_id:'news:brief-test',signal_kind:'news',title:'隔离测试：甲公司中标',body:'隔离测试原文',original_title:'隔离测试：甲公司中标',original_body:'甲公司发布中标公告',news_source:'隔离测试',received_at:Date.now(),agent_summary:false,news_mode:'direct',history_version:0};
db.prepare('INSERT INTO news_archive VALUES(?,?,?)').run(news.signal_id,JSON.stringify(news),new Date().toISOString());
const brief = {day:'2026-09-24',stage:'postclose',generated_at:'2026-09-24T07:15:00Z',window_start:'2026-09-24T01:10:00Z',window_end:'2026-09-24T07:15:00Z',news_count:1,alert_count:1,major:[],watchlist:[{signal_id:news.signal_id,title:news.original_title,excerpt:'甲公司发布中标公告',source:'隔离测试',tag:'公司事项',watchlist_match:true,received_at:news.received_at}],other:[],alerts:[{title:'隔离测试：甲公司价格提醒',body:'价格越过已设置阈值',kind:'price',received_at:news.received_at}]};
db.prepare('INSERT INTO daily_briefs VALUES(?,?,?)').run(brief.day,brief.stage,JSON.stringify(brief));
db.close();
const server = createServer();
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const port = server.address().port;
await new Promise(resolve=>server.close(resolve));
const app = spawn(exe,[],{cwd:dir,windowsHide:true,env:{...process.env,WEBVIEW2_USER_DATA_FOLDER:path.join(dir,'webview-profile'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`},stdio:'ignore'});
let ws;
try {
  let page;
  for(let i=0;i<120;i++) {
    try { page=(await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(item=>item.url==='http://tauri.localhost/'); } catch {}
    if(page)break;
    await new Promise(resolve=>setTimeout(resolve,250));
  }
  assert.ok(page,'Isolated native main WebView did not start');
  ws=new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
  let nextId=0;
  const pending=new Map();
  ws.addEventListener('message',event=>{const response=JSON.parse(event.data);const callback=pending.get(response.id);if(callback){pending.delete(response.id);response.error?callback.reject(Error(JSON.stringify(response.error))):callback.resolve(response.result);}});
  const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++nextId;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async expression=>{const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
  const until=async expression=>{for(let i=0;i<80;i++){if(await evaluate(expression))return;await new Promise(resolve=>setTimeout(resolve,100));}throw Error('UI timeout: '+expression);};
  await until(`!!document.querySelector('.alert-history-button')&&!!window.__TAURI_INTERNALS__`);
  const saved=await evaluate(`window.__TAURI_INTERNALS__.invoke('get_daily_briefs')`);
  assert.equal(saved[0].watchlist[0].title,news.original_title);
  assert.equal(saved[0].alerts[0].title,brief.alerts[0].title);
  await evaluate(`document.querySelector('.alert-history-button').click()`);
  await until(`Array.from(document.querySelectorAll('.news-tabs button')).some(button=>button.innerText==='每日简报')`);
  await evaluate(`Array.from(document.querySelectorAll('.news-tabs button')).find(button=>button.innerText==='每日简报').click()`);
  await until(`document.querySelector('.brief-card')?.innerText.includes('隔离测试：甲公司中标')`);
  assert.ok(await evaluate(`document.querySelector('.brief-card').innerText.includes('隔离测试：甲公司价格提醒')`));
  assert.ok(await evaluate(`!!document.querySelector('.brief-card .manual-ai-btn')`));
  const layout=await evaluate(`(()=>{const el=document.querySelector('.brief-list');const b=el.getBoundingClientRect();return {bottom:b.bottom,height:innerHeight,overflow:getComputedStyle(el).overflowY};})()`);
  assert.equal(layout.overflow,'auto');
  assert.ok(layout.bottom<=layout.height,JSON.stringify(layout));
  await new Promise(resolve=>setTimeout(resolve,300)); // Wait for the modal opening transition.
  const capture=await call('Page.captureScreenshot',{format:'png'});
  writeFileSync(path.join(dir,'daily-brief.png'),Buffer.from(capture.data,'base64'));
  console.log(`Native daily brief passed: actual Tauri IPC, saved news/alerts, manual AI entry, scrollable list. Fixture data only; no Claude call. Evidence: ${dir}`);
} finally {
  ws?.close();
  if(app.exitCode===null) { const exited=new Promise(resolve=>app.once('exit',resolve));app.kill();await Promise.race([exited,new Promise(resolve=>setTimeout(resolve,2000))]); }
}
