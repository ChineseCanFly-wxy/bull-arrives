import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync, realpathSync, readFileSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';
import { createServer } from 'node:net';
import path from 'node:path';
import { targetDir } from './build-env.mjs';

// Actual installed Tauri commands, in a fresh portable test profile only.
const dir = realpathSync(process.argv[2]);
assert.ok(dir.startsWith(targetDir + path.sep), 'acceptance install must be inside the build target');
const exe = path.join(dir, 'bull-arrives.exe');
const resources = path.join(dir, 'research-runtime');
assert.equal(JSON.parse(readFileSync(path.join(resources,'runtime.json'))).schema,'bull-research-runtime-v1');
writeFileSync(path.join(dir,'portable.dat'),'');
mkdirSync(path.join(dir,'data'),{recursive:true});
const db = new DatabaseSync(path.join(dir,'data','bull-arrives.db'));
db.exec('CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)');
db.prepare('INSERT OR REPLACE INTO settings VALUES(?,?)').run('local_history_url','http://127.0.0.1:1');
for(const key of ['ai_enabled','local_history_enabled','news_notifications_enabled','mainline_discovery_enabled','research_notifications_enabled']) db.prepare('INSERT OR REPLACE INTO settings VALUES(?,?)').run(key,'0');
db.close();
const server=createServer(); await new Promise(r=>server.listen(0,'127.0.0.1',r)); const port=server.address().port; await new Promise(r=>server.close(r));
const home=path.join(dir,'empty-home');mkdirSync(home,{recursive:true});
const app=spawn(exe,[],{cwd:dir,windowsHide:true,env:{...process.env,PYTHONHOME:path.join(dir,'invalid-python'),PYTHONPATH:path.join(dir,'invalid-packages'),HOME:home,WEBVIEW2_USER_DATA_FOLDER:path.join(dir,'webview-profile'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+port},stdio:'ignore'});
let ws;
try {
  let page;
  for(let i=0;i<160;i++) {try {page=(await(await fetch('http://127.0.0.1:'+port+'/json/list')).json()).find(p=>p.url==='http://tauri.localhost/');} catch {} if(page)break; await new Promise(r=>setTimeout(r,250));}
  assert.ok(page,'installed native application did not start; exit='+app.exitCode);
  ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
  let id=0;const pending=new Map();ws.addEventListener('message',e=>{const data=JSON.parse(e.data),entry=pending.get(data.id);if(entry){pending.delete(data.id);clearTimeout(entry.timer);data.error?entry.reject(Error(JSON.stringify(data.error))):entry.resolve(data.result);}});
  const call=(method,params={})=>new Promise((resolve,reject)=>{const current=++id;const timer=setTimeout(()=>{pending.delete(current);reject(Error('native command timed out: '+method));},30000);pending.set(current,{resolve,reject,timer});ws.send(JSON.stringify({id:current,method,params}));});
  const evaluate=async expression=>{const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
  const invoke=(command,args={})=>evaluate('window.__TAURI_INTERNALS__.invoke('+JSON.stringify(command)+','+JSON.stringify(args)+')');
  for(let i=0;i<80;i++){if(await evaluate('!!window.__TAURI_INTERNALS__'))break;await new Promise(r=>setTimeout(r,100));}
  for(let i=0;;i++){try{await invoke('get_settings');break;}catch(error){if(i>=80||!error.message.includes('state not managed'))throw error;await new Promise(r=>setTimeout(r,100));}}
  const config=await invoke('research_model_config');assert.equal(path.toNamespacedPath(config.research_root),path.toNamespacedPath(resources));assert.ok(path.toNamespacedPath(config.python).startsWith(path.toNamespacedPath(resources)+path.sep));
  const before=await invoke('get_settings');assert.equal(before.local_history_enabled,'0');
  const job=await invoke('research_job_start',{request:{kind:'scan',models:['breadth22_h20','index26_h20','breadth22_excess_csi20','breadth22_rank20','breadth22_open_downside20'],comparisons:['baseline'],enable_observation:false}});
  let final;
  const started=Date.now();
  while(Date.now()-started<600000){final=await invoke('research_job_get',{id:job.id});if(['complete','failed','cancelled','interrupted'].includes(final.state))break;await new Promise(r=>setTimeout(r,1000));}
  assert.equal(final.state,'complete',JSON.stringify(final));assert.equal(final.completed,5);assert.equal(final.results.length,5);
  const result={state:'passed',installed_directory:dir,external_python:false,stockdb_enabled:false,job_id:job.id,completed:final.completed,models:final.results.map(r=>({model_id:r.model_id,as_of:r.group.as_of,candidates:r.group.candidates.length}))};
  await evaluate('Array.from(document.querySelectorAll("button")).find(b=>b.innerText.includes("研究中心"))?.click()');
  const screenshot=await call('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(dir,'research-install.png'),Buffer.from(screenshot.data,'base64'));
  writeFileSync(path.join(dir,'research-install-result.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result));
} finally {ws?.close();if(app.exitCode===null){const exited=new Promise(r=>app.once('exit',r));app.kill();await Promise.race([exited,new Promise(r=>setTimeout(r,3000))]);}}
