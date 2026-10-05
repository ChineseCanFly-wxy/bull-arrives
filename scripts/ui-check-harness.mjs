import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdirSync,mkdtempSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createServer} from 'vite';
import vue from '@vitejs/plugin-vue';
export async function uiHarness({name,entry,mock,styles=''}){
 const root=fileURLToPath(new URL('..',import.meta.url));const target=path.join(root,'src-tauri/target/ui-previews');mkdirSync(target,{recursive:true});
 const output=mkdtempSync(path.join(target,name+'-'));const profile=mkdtempSync(path.join(output,'browser-profile-'));const errors=[];let browser,ws,nextId=0;
 const pending=new Map();
 const server=await createServer({root,configFile:false,cacheDir:path.join(profile,'vite-cache'),optimizeDeps:{noDiscovery:true,entries:[],include:['vue','pinia','naive-ui'],exclude:['@tauri-apps/api/core','@tauri-apps/api/event','@tauri-apps/plugin-autostart']},resolve:{alias:{'@':path.join(root,'src')},dedupe:['vue']},plugins:[vue(),{name:'isolated-'+name,enforce:'pre',resolveId(id){if(id.startsWith('@tauri-apps/'))return'\0controls-mock';if(id==='/controls-entry.js')return'\0controls-entry';},load(id){if(id==='\0controls-mock')return mock;if(id==='\0controls-entry')return entry;},configureServer(s){s.middlewares.use((req,res,next)=>{if(req.url!=='/')return next();res.setHeader('Content-Type','text/html');res.end('<!doctype html><html><head><meta charset="utf-8"><style>body{margin:0}'+styles+'</style></head><body><div id="app"></div><script type="module" src="/controls-entry.js"></script></body></html>');});}}],server:{host:'127.0.0.1',port:0,watch:{ignored:['**/src-tauri/**']}}});
 const close=async()=>{ws?.close();if(browser?.exitCode===null){const exited=new Promise(r=>browser.once('exit',r));browser.kill();await Promise.race([exited,new Promise(r=>setTimeout(r,2000))]);}await server.close();assert.equal(path.dirname(path.resolve(profile)),path.resolve(output));assert.ok(path.basename(profile).startsWith('browser-profile-'));try{rmSync(profile,{recursive:true,force:true,maxRetries:10,retryDelay:100});}catch{console.log('Isolated profile retained: '+profile);}};
 try{
  await server.listen();const port=server.httpServer.address().port;
  browser=spawn(process.env.NEWS_TEST_BROWSER||'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--disable-gpu','--disable-extensions','--no-first-run','--no-default-browser-check','--remote-debugging-port=0','--user-data-dir='+profile,'http://127.0.0.1:'+port+'/'],{windowsHide:true,stdio:'ignore'});
  let debug;for(let i=0;i<100;i++){try{debug=Number(readFileSync(path.join(profile,'DevToolsActivePort'),'utf8').split('\n')[0]);break;}catch{await new Promise(r=>setTimeout(r,100));}}assert.ok(debug);
  const pages=await(await fetch('http://127.0.0.1:'+debug+'/json/list')).json();const page=pages.find(p=>p.type==='page'&&p.url.startsWith('http://127.0.0.1:'+port+'/'));assert.ok(page);ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
  ws.addEventListener('message',event=>{const data=JSON.parse(event.data);if(data.method==='Runtime.exceptionThrown')errors.push(data.params.exceptionDetails);const callback=pending.get(data.id);if(callback){pending.delete(data.id);clearTimeout(callback.timer);data.error?callback.reject(Error(JSON.stringify(data.error))):callback.resolve(data.result);}});
  const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++nextId;const timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP timeout '+method));},15000);pending.set(id,{resolve,reject,timer});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async expression=>{const result=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert.ok(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
  const wait=async expression=>{for(let i=0;i<100;i++){if(await evaluate(expression))return;await new Promise(r=>setTimeout(r,100));}throw Error('UI timeout: '+expression+'\n'+await evaluate('document.body.innerText'));};
  const click=async text=>evaluate('(()=>{const b=[...document.querySelectorAll("button")].find(e=>e.innerText.includes('+JSON.stringify(text)+'));if(!b)throw Error("No button");b.click()})()');
  const screenshot=async name=>{const image=await call('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(output,name+'.png'),Buffer.from(image.data,'base64'));};
  await call('Runtime.enable');return{root,output,errors,call,evaluate,wait,click,screenshot,close,save(value){writeFileSync(path.join(output,'result.json'),JSON.stringify(value,null,2)+'\n');}};
 }catch(e){await close();throw e;}
}
