import assert from 'node:assert/strict';
import { uiHarness } from './ui-check-harness.mjs';
const mock = `
window.__alerts={calls:[],listeners:{},settings:{important_alerts_enabled:'1'},snapshot:{version:0,entries:[]},hidden:false};
export async function invoke(command,args){const s=window.__alerts;s.calls.push({command,args});
 if(command==='get_settings')return {...s.settings};if(command==='get_important_alerts')return s.snapshot;
 if(command==='toggle_important_alerts'){s.hidden=!s.hidden;return;}
 if(command==='dismiss_important_alert'){s.snapshot={version:s.snapshot.version+1,entries:s.snapshot.entries.filter(e=>e.id!==args.id)};s.listeners['important-alerts-changed']?.({payload:s.snapshot});return;}
 if(command==='research_job_list')return [{id:9,kind:'scan',state:'complete',completed:2,total:2,results:[],message:'最新任务'},{id:8,kind:'scan',state:'failed',completed:0,total:2,results:[],message:'旧错误'}];
 if(command==='get_stockdb_status')return {enabled:false,state:'disabled',candidates:[],autoUpdate:{}};
 if(command==='list_datasources')return [];if(command==='get_portable_mode')return false;
 if(command==='set_important_alerts_hotkey'){s.settings.important_alerts_hotkey=args.hotkey;return;}
 if(command==='show_important_alerts'){s.hidden=false;return;}
 return null;}
export async function listen(name,callback){window.__alerts.listeners[name]=callback;return()=>delete window.__alerts.listeners[name];}
export async function emit(){};export async function isEnabled(){return false;}
export async function enable(){};export async function disable(){};export async function openUrl(){};
export class PhysicalSize{constructor(width,height){this.width=width;this.height=height;}}
export function getCurrentWindow(){return {label:'important-alerts',startDragging:async()=>{},startResizeDragging:async()=>{window.__alerts.resized=true;},innerSize:async()=>({width:innerWidth,height:innerHeight}),scaleFactor:async()=>1,setSize:async s=>{window.__alerts.size=s;}};}
`;
const entry = `import {createApp,ref,h} from 'vue';import {createPinia} from 'pinia';import Important from '/src/components/notifications/ImportantAlerts.vue';import Jobs from '/src/components/research/ResearchJobPanel.vue';import Settings from '/src/components/settings/FloatingAlertSettings.vue';const mode=ref('alerts');window.__mode=mode;createApp({setup(){return()=>h(mode.value==='alerts'?Important:mode.value==='jobs'?Jobs:Settings)}}).use(createPinia()).mount('#app');`;
const t=await uiHarness({name:'persistent-alerts',entry,mock});
try {
 await t.wait('document.querySelector(".important-alerts .empty")?.textContent.includes("等待新提醒")');
 assert.equal(await t.evaluate('window.__alerts.hidden'),false);
 for(const [width,height] of [[220,100],[280,160],[420,300],[800,500]]){
  await t.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
  assert.equal(await t.evaluate('document.documentElement.scrollWidth<=innerWidth+1'),true);
  assert.equal(await t.evaluate('(()=>{const r=document.querySelector(".hide-window").getBoundingClientRect();return r.right<=innerWidth&&r.top>=0})()'),true);
  await t.screenshot('empty-'+width);
 }
 await t.evaluate(`window.__alerts.snapshot={version:1,entries:[{id:'notice',title:'实际审核提醒',body:'很长的消息内容。'.repeat(120),notification_category:'trades',received_at:Date.now()}]};window.__alerts.listeners['important-alerts-changed']({payload:window.__alerts.snapshot})`);
 await t.wait('document.querySelector("[data-notice-id=notice]")');
 await t.call('Emulation.setDeviceMetricsOverride',{width:220,height:160,deviceScaleFactor:1,mobile:false});
 assert.equal(await t.evaluate('document.querySelector(".important-alerts ol").scrollHeight>document.querySelector(".important-alerts ol").clientHeight'),true);
 await t.evaluate('document.querySelector(".hide-window").click()');
 assert.equal(await t.evaluate('window.__alerts.hidden'),true);
 await t.evaluate('document.querySelector(".actions button:last-child").click()');
 await t.wait('document.querySelector(".empty")');
 assert.equal(await t.evaluate('window.__alerts.hidden'),true);
 await t.evaluate(`document.querySelector('.window-resize-handle').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}))`);
 await t.wait('window.__alerts.size?.width===240');
 await t.evaluate(`window.__mode.value='jobs'`);
 await t.wait('document.querySelector(".research-jobs article")');
 assert.equal(await t.evaluate('document.querySelectorAll(".research-jobs article").length'),1);
 assert.match(await t.evaluate('document.querySelector(".research-jobs article").textContent'),/#9/);
 assert.equal(await t.evaluate('document.querySelector(".job-more")'),null);
 await t.evaluate(`window.__mode.value='settings'`);
 await t.wait('document.querySelector(".hotkey-actions button")');
 await t.evaluate(`const button=document.querySelector('.hotkey-actions button');button.click();button.dispatchEvent(new KeyboardEvent('keydown',{key:'j',code:'KeyJ',altKey:true,bubbles:true}))`);
 await t.wait(`window.__alerts.calls.some(c=>c.command==='set_important_alerts_hotkey'&&c.args.hotkey==='Alt+J')`);
 assert.equal(t.errors.length,0,JSON.stringify(t.errors));
 t.save({passed:7,scope:'Vue rendering and mocked IPC: persistent empty state, responsive resizing, hidden state, read dismissal, latest job only, shortcut capture; native windows verified separately'});
 console.log('PASS: persistent responsive alerts, hide/read/resize actions, latest job only and shortcut capture. '+t.output);
} catch(error) { console.error(JSON.stringify(t.errors)); console.error(await t.evaluate('document.documentElement.innerHTML')); throw error; } finally {await t.close();}
