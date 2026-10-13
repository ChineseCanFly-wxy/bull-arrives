import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdirSync,writeFileSync,realpathSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';
import path from 'node:path';
import { createServer } from 'node:net';
import { targetDir } from './build-env.mjs';
const directory=realpathSync(process.argv[2]);
assert.ok(directory.startsWith(targetDir+path.sep),'Native QA requires an isolated directory inside target');
writeFileSync(path.join(directory,'portable.dat'),'isolated native QA');
mkdirSync(path.join(directory,'data'),{recursive:true});
const db=new DatabaseSync(path.join(directory,'data','bull-arrives.db'));
db.exec('CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)');
for(const [key,value] of Object.entries({ai_enabled:'0',local_history_enabled:'0',news_notifications_enabled:'0',mainline_discovery_enabled:'0',research_auto_trade_enabled:'0',ticker_visible:'0',important_alerts_enabled:'1'}))db.prepare('INSERT OR REPLACE INTO settings VALUES(?,?)').run(key,value);
db.close();
const socket=createServer();await new Promise(r=>socket.listen(0,'127.0.0.1',r));const port=socket.address().port;await new Promise(r=>socket.close(r));
const processHandle=spawn(path.join(directory,'bull-arrives.exe'),[],{cwd:directory,windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_USER_DATA_FOLDER:path.join(directory,'isolated-webview'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+port}});
let ws;
try{
 let page;
 for(let count=0;count<160;count++){try{page=(await(await fetch('http://127.0.0.1:'+port+'/json/list')).json()).find(p=>p.url==='http://tauri.localhost/');}catch{}if(page)break;await new Promise(r=>setTimeout(r,250));}
 assert.ok(page,'Native application did not start');
 ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let id=0;const calls=new Map();ws.addEventListener('message',e=>{const message=JSON.parse(e.data);const pending=calls.get(message.id);if(pending){calls.delete(message.id);clearTimeout(pending.timer);message.error?pending.reject(Error(JSON.stringify(message.error))):pending.resolve(message.result);}});
 const call=(method,params)=>new Promise((resolve,reject)=>{const current=++id;const timer=setTimeout(()=>reject(Error('Native QA timeout')),20000);calls.set(current,{resolve,reject,timer});ws.send(JSON.stringify({id:current,method,params}));});
 const evaluate=async expression=>{const value=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!value.exceptionDetails,JSON.stringify(value.exceptionDetails));return value.result.value;};
 const invoke=(command,args={})=>evaluate('window.__TAURI_INTERNALS__.invoke('+JSON.stringify(command)+','+JSON.stringify(args)+')');
 for(let i=0;i<80;i++){try{await invoke('get_settings');break;}catch(e){if(i===79)throw e;await new Promise(r=>setTimeout(r,100));}}
 assert.equal(await invoke('research_follow_star',{enabled:null}),false);
 assert.equal(await invoke('research_follow_star',{enabled:true}),true);
 assert.equal(await invoke('research_follow_star',{enabled:false}),false);
 assert.equal((await invoke('research_follow_clear_star')).cleared_positions,0);
 const windowState=()=>evaluate('window.__TAURI_INTERNALS__.invoke("plugin:window|is_visible",{label:"important-alerts"})');
 for(let i=0;i<40&&!await windowState();i++)await new Promise(r=>setTimeout(r,100));
 assert.equal(await windowState(),true,'Empty reminder window must remain visible');
 await invoke('toggle_important_alerts');await new Promise(r=>setTimeout(r,150));assert.equal(await windowState(),false);
 await invoke('test_notification',{category:'floating'});await new Promise(r=>setTimeout(r,200));assert.equal(await windowState(),false,'New reminder must not reopen manually hidden window');
 await invoke('show_important_alerts');await new Promise(r=>setTimeout(r,150));assert.equal(await windowState(),true);
 const snapshot=await invoke('get_important_alerts');assert.ok(snapshot.entries.length);
 for(const entry of snapshot.entries)await invoke('dismiss_important_alert',{id:entry.id});
 await new Promise(r=>setTimeout(r,150));assert.equal(await windowState(),true,'Dismissing all messages must not close persistent window');
 await invoke('set_important_alerts_hotkey',{hotkey:'Ctrl+Alt+Shift+J'});
 const settings=await invoke('get_settings');assert.equal(settings.important_alerts_hotkey,'Ctrl+Alt+Shift+J');
 const conflict=await evaluate('window.__TAURI_INTERNALS__.invoke("set_important_alerts_hotkey",{hotkey:"Alt+Q"}).then(()=>false,()=>true)');assert.equal(conflict,true);
 assert.equal((await invoke('get_settings')).important_alerts_hotkey,'Ctrl+Alt+Shift+J');
 const result={passed:9,scope:'Actual Tauri commands and native Windows visibility in an isolated portable database; no real accounts, StockDB, research jobs or market orders used'};
 writeFileSync(path.join(directory,'native-check.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}finally{ws?.close();if(processHandle.exitCode===null){processHandle.kill();await Promise.race([new Promise(r=>processHandle.once('exit',r)),new Promise(r=>setTimeout(r,3000))]);}}
