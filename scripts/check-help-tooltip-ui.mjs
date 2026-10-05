import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdirSync,mkdtempSync,readFileSync,readdirSync,rmSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createServer} from 'vite';
import vue from '@vitejs/plugin-vue';

// Regression of the real shared component and screenshot's real job panel.
// Isolated headless browser/profile and read-only mocked IPC; no application data.
const root=fileURLToPath(new URL('..',import.meta.url));
const output=mkdtempSync(path.join(root,'src-tauri/target/ui-previews/help-tooltip-ui-'));
const profile=mkdtempSync(path.join(output,'browser-profile-'));
const baseline=process.argv.includes('--baseline');
const behaviorOnly=process.argv.includes('--behavior-only');
const quick=process.argv.includes('--quick');
const catalog=[];
function inventory(folder){for(const entry of readdirSync(folder,{withFileTypes:true})){
 const file=path.join(folder,entry.name);
 if(entry.isDirectory())inventory(file);
 else if(entry.name.endsWith('.vue')){
  const text=readFileSync(file,'utf8');
  const matches=[...text.matchAll(/<HelpTooltip\b([^>]*)>([\s\S]*?)<\/HelpTooltip>/g)];
  assert.equal(matches.length,(text.match(/<HelpTooltip(?:\s|>)/g)||[]).length,'Inventory all help entries in '+file);
  for(const match of matches){const label=match[1].match(/(?:^|\s)label="([^"]+)"/);if(label){assert.ok(!match[2].includes('<'),'Fixture requires plain tooltip text');catalog.push({file:path.relative(root,file).replaceAll('\\','/'),label:label[1],text:match[2].trim()});}else{assert.ok(file.endsWith('ResearchExtensions.vue')&&match[1].includes(':label=')&&match[2].includes('study.limitations'),'Inventory new dynamic entries explicitly');const evidence=JSON.parse(readFileSync(path.join(root,'src-tauri/external-strategy-evidence.json'),'utf8'));for(const study of evidence.studies)catalog.push({file:path.relative(root,file).replaceAll('\\','/'),label:study.title+'研究口径说明',text:study.limitations.join('；')});}}
 }
}}
inventory(path.join(root,'src/components'));
assert.ok(catalog.length>0,'Audit current question-mark entries');
assert.ok(catalog.some(entry=>entry.file.endsWith('ResearchJobPanel.vue')),'Original clipped job help remains in the inventory');
const entry=String.raw`
import {createApp,h,reactive} from 'vue';
import {NConfigProvider,NModal,darkTheme,lightTheme} from 'naive-ui';
import HelpTooltip from '/src/components/common/HelpTooltip.vue';
import ResearchJobPanel from '/src/components/research/ResearchJobPanel.vue';
import '/src/assets/styles/variables.css';
const catalog=CATALOG;
const state=reactive({index:0,position:'top-left',theme:'light',show:true,jobs:false,tall:false});window.__tooltipFixture=state;
const positions={'top-left':{top:'12px',left:'12px'},'top-right':{top:'12px',right:'12px'},'bottom-left':{bottom:'12px',left:'12px'},'bottom-right':{bottom:'12px',right:'12px'},center:{top:'calc(50% - 10px)',left:'calc(50% - 10px)'}};
createApp({render(){document.documentElement.dataset.theme=state.theme;return h(NConfigProvider,{theme:state.theme==='dark'?darkTheme:lightTheme},()=>h(NModal,{show:state.show,'onUpdate:show':v=>state.show=v,preset:'card',title:'问号提示回归测试',class:'tooltip-fixture-modal',style:{width:'min(1170px, calc(100vw - 24px))',height:'calc(100dvh - 32px)',display:'flex',flexDirection:'column',overflow:'hidden'}},()=>h('div',{class:'fixture-scroll'},[
 h('div',{class:'fixture-stage',style:{height:state.tall?'1200px':'100%'}},[
  state.jobs?h('div',{class:'fixture-jobs'},[h(ResearchJobPanel)]):h('span',{class:'fixture-anchor',style:positions[state.position]},[h(HelpTooltip,{label:catalog[state.index].label},()=>catalog[state.index].text)])
 ])
])));}}).mount('#app');
`.replace('CATALOG',JSON.stringify(catalog));
const mock=`export async function invoke(command){if(command==='research_job_list')return[{id:1,kind:'scan',state:'running',phase:'calculating',completed:0,total:5,message:'第1/5项：breadth22_h20 / baseline；实际计算最近完成日模型信号',results:[]}];throw Error('Unexpected IPC '+command);}`;
const server=await createServer({root,configFile:false,cacheDir:path.join(profile,'vite-cache'),
 optimizeDeps:{noDiscovery:true,entries:[],include:['vue','naive-ui'],exclude:['@tauri-apps/api/core']},
 resolve:{alias:{'@':path.join(root,'src')},dedupe:['vue']},
 plugins:[vue(),{name:'tooltip-fixture',enforce:'pre',resolveId(id){if(id==='@tauri-apps/api/core')return'\0tooltip-ipc';if(id==='/tooltip-entry.js')return'\0tooltip-entry';},load(id){if(id==='\0tooltip-ipc')return mock;if(id==='\0tooltip-entry')return entry;},configureServer(s){s.middlewares.use((req,res,next)=>{if(req.url!=='/')return next();res.setHeader('Content-Type','text/html');res.end(`<!doctype html><html><head><meta charset="utf-8"><style>body{margin:0}.tooltip-fixture-modal>.n-card__content{flex:1;min-height:0;overflow:hidden;padding:8px}.fixture-scroll{height:100%;overflow:auto;border:1px solid var(--color-border-0)}.fixture-stage{position:relative;min-height:0}.fixture-anchor{position:absolute}.fixture-jobs{position:absolute;top:30%;left:12px;right:12px}</style></head><body><div id="app"></div><script type="module" src="/tooltip-entry.js"></script></body></html>`);});}}],
 server:{host:'127.0.0.1',port:0,watch:{ignored:['**/src-tauri/**']}}});
let browser,ws,counter=0;const pending=new Map(),runtimeErrors=[],results=[],behavior=[];
try{
 await server.listen();const port=server.httpServer.address().port;
 browser=spawn(process.env.NEWS_TEST_BROWSER||'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--disable-gpu','--disable-extensions','--no-first-run','--no-default-browser-check','--remote-debugging-port=0',`--user-data-dir=${profile}`,`http://127.0.0.1:${port}/`],{windowsHide:true,stdio:'ignore'});
 let debug;for(let i=0;i<100;i++){try{debug=Number(readFileSync(path.join(profile,'DevToolsActivePort'),'utf8').split('\n')[0]);break;}catch{await new Promise(r=>setTimeout(r,100));}}assert.ok(debug,'Isolated browser starts');
 const pages=await(await fetch(`http://127.0.0.1:${debug}/json/list`)).json();const page=pages.find(p=>p.type==='page'&&p.url.startsWith(`http://127.0.0.1:${port}/`));assert.ok(page);ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 ws.addEventListener('message',event=>{const value=JSON.parse(event.data);if(value.method==='Runtime.exceptionThrown')runtimeErrors.push(value.params.exceptionDetails);const callback=pending.get(value.id);if(callback){pending.delete(value.id);value.error?callback.reject(Error(JSON.stringify(value.error))):callback.resolve(value.result);}});
 const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++counter;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
 const evaluate=async expression=>{const value=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!value.exceptionDetails,JSON.stringify(value.exceptionDetails));return value.result.value;};
 const wait=async expression=>{for(let i=0;i<100;i++){if(await evaluate(expression))return;await new Promise(r=>setTimeout(r,50));}throw Error('Timeout '+expression);};
 const settle=()=>evaluate(`new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(r,50))))`);
 const close=async()=>{await evaluate(`document.activeElement?.blur();window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));`);await wait(`!document.querySelector('.help-tooltip-trigger[aria-describedby]')`);await evaluate(`new Promise(r=>setTimeout(r,160))`);};
 const bounds=()=>evaluate(`(()=>{const content=document.querySelector('.help-tooltip-content[role=tooltip]'),popover=content?.closest('.n-popover');if(!content||!popover)return null;const rect=e=>{const r=e.getBoundingClientRect();return{left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:r.width,height:r.height}};const c=rect(content),p=rect(popover),trigger=document.querySelector('.help-tooltip-trigger[aria-describedby]'),tr=rect(trigger);const hitPoints=[[c.left+2,c.top+2],[c.right-2,c.top+2],[c.left+2,c.bottom-2],[c.right-2,c.bottom-2],[c.left+c.width/2,c.top+c.height/2]];return{content:c,popover:p,trigger:tr,width:innerWidth,height:innerHeight,inModal:!!content.closest('.n-modal'),hitVisible:hitPoints.every(([x,y])=>{const e=document.elementFromPoint(x,y);return !!e&&(e===content||content.contains(e)||e===popover||popover.contains(e));}),wrapped:content.scrollWidth<=content.clientWidth+1,scrollable:content.scrollHeight>content.clientHeight+1,text:content.textContent,described:trigger.getAttribute('aria-describedby')===content.id};})()`);
 const record=async(meta)=>{await settle();const value=await bounds();assert.ok(value,'Tooltip rendered');const inside=value.popover.left>=-1&&value.popover.right<=value.width+1&&value.popover.top>=-1&&value.popover.bottom<=value.height+1;results.push({...meta,...value,pass:inside&&value.hitVisible&&value.wrapped&&value.described});return results.at(-1);};
 const screenshot=async name=>{const image=await call('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(output,name),Buffer.from(image.data,'base64'));};
 await call('Runtime.enable');await wait(`document.querySelector('.fixture-anchor .help-tooltip-trigger')`);
 // Actual screenshot context: active job, left edge, scroll container, modal clipping.
 await call('Emulation.setDeviceMetricsOverride',{width:1314,height:900,deviceScaleFactor:1,mobile:false});
 await evaluate(`window.__tooltipFixture.jobs=true`);await wait(`document.querySelector('.research-jobs article[data-job-state=running]')`);
 await evaluate(`document.querySelector('[aria-label="后台任务说明"]').focus()`);await wait(`document.querySelector('.help-tooltip-content[role=tooltip]')`);
 const jobResult=await record({scenario:'real-running-job',theme:'light',viewport:[1314,900]});await screenshot(baseline?'before-running-job.png':'after-running-job.png');await close();await evaluate(`window.__tooltipFixture.jobs=false`);
 // Every help entry, both themes, all corners plus centre, normal/narrow/short viewports.
 for(const theme of behaviorOnly?[]:quick?['light']:['light','dark'])for(const [width,height]of quick?[[390,480],[320,300]]:[[1314,900],[900,650],[390,480],[320,300]]){
  await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
  await evaluate(`window.__tooltipFixture.theme=${JSON.stringify(theme)}`);await settle();
  for(let index=0;index<catalog.length;index++)for(const position of ['top-left','top-right','bottom-left','bottom-right','center']){
   await evaluate(`Object.assign(window.__tooltipFixture,{index:${index},position:${JSON.stringify(position)}})`);await settle();
   await evaluate(`document.querySelector('.fixture-anchor .help-tooltip-trigger').focus()`);await wait(`document.querySelector('.help-tooltip-trigger[aria-describedby]')`);
   const result=await record({scenario:'shared-entry',file:catalog[index].file,label:catalog[index].label,theme,viewport:[width,height],position});assert.equal(result.text,catalog[index].text,'No lost tooltip text');
   if(!baseline&&theme==='light'&&width===390&&index===3&&position==='top-left')await screenshot('after-narrow-long-text.png');
   await close();
  }
  console.log(`Tooltip layout: ${theme} ${width}x${height}, ${catalog.length} entries x 5 positions`);
 }
 if(!baseline){
  await call('Emulation.setDeviceMetricsOverride',{width:900,height:650,deviceScaleFactor:1,mobile:false});await evaluate(`Object.assign(window.__tooltipFixture,{theme:'light',index:3,position:'center'})`);await settle();
  const before=await evaluate(`(()=>{const r=document.querySelector('.fixture-anchor').getBoundingClientRect();return{left:r.left,top:r.top,width:r.width,height:r.height}})()`);
  const trigger=await evaluate(`(()=>{const r=document.querySelector('.fixture-anchor .help-tooltip-trigger').getBoundingClientRect();return{x:r.left+r.width/2,y:r.top+r.height/2}})()`);
  await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:trigger.x,y:trigger.y});await wait(`document.querySelector('.help-tooltip-trigger[aria-describedby]')`);await settle();const hovered=await bounds();
  await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:hovered.content.left+hovered.content.width/2,y:hovered.content.top+hovered.content.height/2});await evaluate(`new Promise(r=>setTimeout(r,300))`);assert.equal(await evaluate(`!!document.querySelector('.help-tooltip-trigger[aria-describedby]')`),true,'Tooltip stays readable when mouse enters it');behavior.push('hover across trigger and tooltip');
  await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await wait(`!document.querySelector('.help-tooltip-trigger[aria-describedby]')`);assert.equal(await evaluate(`window.__tooltipFixture.show`),true,'Escape closes only tooltip');behavior.push('Escape leaves modal open');
  const after=await evaluate(`(()=>{const r=document.querySelector('.fixture-anchor').getBoundingClientRect();return{left:r.left,top:r.top,width:r.width,height:r.height}})()`);assert.deepEqual(after,before,'Tooltip does not shift layout');behavior.push('no layout shift');
  await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:1,y:1});await close();
  await evaluate(`window.__tooltipFixture.tall=true`);await settle();await evaluate(`document.querySelector('.fixture-scroll').scrollTop=220`);await settle();await evaluate(`document.querySelector('.fixture-anchor .help-tooltip-trigger').blur();document.querySelector('.fixture-anchor .help-tooltip-trigger').focus()`);await wait(`document.querySelector('.help-tooltip-trigger[aria-describedby]')`);const scrolled=await record({scenario:'nested-scroll',viewport:[900,650]});assert.ok(scrolled.pass,'Tooltip follows scrolling trigger without clipping');behavior.push('nested scrolling');
  await evaluate(`window.__tooltipFixture.tall=false;document.querySelector('.fixture-scroll').scrollTop=0`);await settle();
  await call('Emulation.setDeviceMetricsOverride',{width:600,height:480,deviceScaleFactor:1,mobile:false});await settle();const resized=await record({scenario:'resize-open-tooltip',viewport:[600,480]});assert.ok(resized.pass,'Open tooltip follows window resize');behavior.push('resize while open');await close();
  await evaluate(`window.__tooltipFixture.show=false`);await settle();assert.equal(await evaluate(`!!document.querySelector('.help-tooltip-trigger[aria-describedby]')`),false,'Unmount clears tooltip');behavior.push('modal close cleanup');
 }
 const failures=results.filter(v=>!v.pass);const report={schema:'help-tooltip-ui-regression-v1',baseline,entries:catalog.length,files:new Set(catalog.map(c=>c.file)).size,catalog,cases:results.length,failures:failures.length,behavior,runtimeErrors,results};writeFileSync(path.join(output,'result.json'),JSON.stringify(report,null,2)+'\n');
 console.log(JSON.stringify({output,baseline,entries:catalog.length,cases:results.length,failures:failures.length,behavior,runtimeErrors:runtimeErrors.length},null,2));
 assert.deepEqual(runtimeErrors,[],'No browser runtime exceptions');
 if(baseline){assert.equal(jobResult.pass,false,'Must reproduce screenshot before changing shared component');assert.ok(failures.length,'Original clipping detected');}else assert.equal(failures.length,0,'Tooltip failures: '+JSON.stringify(failures.slice(0,3)));
}finally{
 ws?.close();if(browser?.exitCode===null){const exited=new Promise(r=>browser.once('exit',r));browser.kill();await Promise.race([exited,new Promise(r=>setTimeout(r,2000))]);}await server.close();
 const resolved=path.resolve(profile);assert.equal(path.dirname(resolved),path.resolve(output));assert.ok(path.basename(resolved).startsWith('browser-profile-'));try{rmSync(resolved,{recursive:true,force:true,maxRetries:10,retryDelay:100});}catch{console.log('Isolated browser profile retained: '+resolved);}
}
