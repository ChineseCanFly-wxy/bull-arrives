import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';
const paths={
  "engine": "\\\\?\\C:\\隔离检查\\历史数据\\StockDB\\stockdb.exe",
  "updater": "\\\\?\\UNC\\server\\共享数据\\StockDB\\数据更新.exe",
  "candidateDrive": "\\\\?\\D:\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\超长候选目录\\StockDB\\stockdb.exe",
  "candidateUnc": "\\\\?\\UNC\\server\\共享数据\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\超长网络目录\\StockDB\\stockdb.exe",
  "agent": "\\\\?\\C:\\隔离检查\\Claude Code\\claude.exe",
  "agentRoot": "\\\\?\\UNC\\server\\共享任务\\Claude 工作目录",
  "browsedAgent": "\\\\?\\D:\\隔离检查\\Claude Code\\claude.cmd",
  "browsedRoot": "\\\\?\\UNC\\server\\其他任务\\Claude 工作目录",
  "exe": "\\\\?\\C:\\隔离检查\\程序\\Bull Arrives.exe",
  "data": "\\\\?\\C:\\隔离检查\\用户数据",
  "database": "\\\\?\\UNC\\server\\共享配置\\settings.db",
  "tasks": "\\\\?\\UNC\\server\\共享任务\\interactive"
};
const extraChecks=[];
const mock=`
const paths=${JSON.stringify(paths)};
const state=window.__stockdbClock={settings:{theme:'light',local_history_enabled:'1',local_history_auto_update_enabled:'1',local_history_auto_update_time:'09:00',ai_enabled:'1',ai_monitor_enabled:'1',agent_claude_path:paths.agent,agent_run_root:paths.agentRoot,agent_timeout_seconds:'180',agent_budget_usd:'10.00'},calls:[],copied:[],dialogPaths:[],confirmations:[],enginePath:paths.engine,updaterPath:paths.updater};
Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{if(state.copyFails)throw Error('隔离复制失败');state.copied.push(text);}}});
window.confirm=message=>{state.confirmations.push(message);return false;};
const stockdbStatus=()=>({enabled:true,platformSupported:true,state:'running_owned',phase:null,enginePath:state.enginePath,updaterPath:state.updaterPath,updaterAvailable:true,owned:true,busy:false,message:'隔离状态，未启动服务',lastError:null,candidates:[{source:'长盘符候选',enginePath:paths.candidateDrive,updaterPath:paths.updater},{source:'长UNC候选',enginePath:paths.candidateUnc,updaterPath:paths.updater}],autoUpdate:{enabled:true,time:state.settings.local_history_auto_update_time,timezone:'Asia/Shanghai',state:'complete',message:'今天已更新，重启应用不会重复执行',dataAsOf:'2026-09-30',lastError:null,failures:0}});
export async function invoke(command,args={}){state.calls.push({command,args:JSON.parse(JSON.stringify(args))});
 if(command==='get_settings')return {...state.settings};
 if(command==='take_pending_navigation')return null;
 if(command==='run_stockdb_update')return '隔离手动更新成功，服务已恢复';
 if(command==='list_datasources')return [['tencent','隔离行情']];if(command==='get_portable_mode')return false;
 if(command==='get_market_session')return {session:'休市',interval_secs:30,is_trading:false,calendar:'隔离日历'};
 if(command==='get_stockdb_status'||command==='scan_stockdb')return stockdbStatus();
 if(command==='select_stockdb_engine'){state.enginePath=args.path;return stockdbStatus();}
 if(command==='select_stockdb_updater'){state.updaterPath=args.path;return stockdbStatus();}
 if(command==='get_build_info')return {version:'2.2.6',profile:'isolated',built_at:'2026-10-05 12:00:00',exe_path:paths.exe};
 if(command==='get_data_paths')return {data_dir:paths.data,database:paths.database,interactive_tasks:paths.tasks};
 if(command==='get_local_history_status')return {state:'connected',message:'仅测试状态',candidates:[]};
 if(command==='get_agent_status')return {installed:true,state:'ready',path:state.settings.agent_claude_path||'C:/isolated/claude.exe',message:'隔离 Claude 状态，仅测试配置界面',guidance:'隔离连接说明',run_dir:state.settings.agent_run_root||'C:/isolated/agent-tasks'};
 if(command==='get_group_hotkey_status')return {previous:{hotkey:'Alt+Left',registered:true},next:{hotkey:'Alt+Right',registered:true}};
 if(command==='get_notification_identity_status')return {supported:true,registered:true,detail:'隔离通知身份'};
 if(command==='set_setting'){if(args.key==='local_history_auto_update_time'&&!/^([01][0-9]|2[0-3]):[0-5][0-9]$/.test(args.value))throw Error('自动更新时间须为HH:mm');state.settings[args.key]=args.value;return;}
 throw Error('Unexpected IPC '+command);
}
export async function listen(){return ()=>{};}export async function emit(){}export async function isEnabled(){return false;}export async function enable(){}export async function disable(){}export async function open(){return state.dialogPaths.shift()??null;}
`;
const entry=`
import {createApp,h,ref} from 'vue';import {createPinia} from 'pinia';import {NConfigProvider,NMessageProvider,darkTheme,lightTheme} from 'naive-ui';import TopBar from '/src/components/layout/TopBar.vue';import SettingsDialog from '/src/components/settings/SettingsDialog.vue';import {useSettingsStore} from '/src/stores/settings.ts';import '/src/assets/styles/variables.css';
import {displayPath} from '/src/utils/pathDisplay.ts';window.__displayPath=displayPath;
const show=ref(true);window.__settingsShow=show;const app=createApp({render:()=>h(NConfigProvider,{theme:settings.theme==='dark'?darkTheme:lightTheme,themeOverrides:{common:{fontFamily:'var(--font-sans)'}}},{default:()=>h(NMessageProvider,null,{default:()=>[h(TopBar),h(SettingsDialog,{show:show.value,initialSection:'market','onUpdate:show':value=>show.value=value})]})})});const pinia=createPinia();app.use(pinia);const settings=useSettingsStore(pinia);await settings.fetchSettings();window.__clockSettings=settings;app.mount('#app');
`;
const t=await uiHarness({name:'stockdb-schedule',entry,mock});
try{
 await t.wait('document.body.innerText.includes("交易日自动更新 · 北京时间 09:00")');
 assert.match(await t.evaluate('document.body.innerText'),/今天已更新.*2026-09-30/s);
 await t.evaluate('document.querySelector("details.history-advanced").open=true');
 assert.equal(await t.evaluate('document.querySelector("input[type=time]").value'),'09:00');
 assert.equal(await t.evaluate(`document.querySelector('[aria-label="StockDB交易日自动更新"]').getAttribute("aria-checked")`),'true');
 const text=await t.evaluate('document.querySelector("details.history-advanced").innerText');assert.match(text,/1分钟.*5次.*一次/s);
 assert.equal(await t.evaluate('document.querySelectorAll("details.history-advanced input[type=number]").length'),0,'Retry policy has no user settings');
 await t.evaluate('(()=>{const el=document.querySelector("input[type=time]");el.value="09:15";el.dispatchEvent(new Event("input",{bubbles:true}));})()');await t.click('保存时间');
 await t.wait('window.__clockSettings.localHistoryAutoUpdateTime==="09:15"');
 assert.equal(await t.evaluate('window.__stockdbClock.settings.local_history_auto_update_time'),'09:15');
 await t.evaluate(`document.querySelector('[aria-label="StockDB交易日自动更新"]').click()`);await t.wait('window.__clockSettings.localHistoryAutoUpdateEnabled===false');
 assert.equal(await t.evaluate('window.__stockdbClock.settings.local_history_auto_update_enabled'),'0');
 await t.evaluate('(()=>{const el=document.querySelector("input[type=time]");el.value="";el.dispatchEvent(new Event("input",{bubbles:true}));})()');await t.click('保存时间');
 await t.wait('document.body.innerText.includes("自动更新时间须为HH:mm")');assert.equal(await t.evaluate('window.__clockSettings.localHistoryAutoUpdateTime'),'09:15','Failed save preserves committed time');
 for(const width of [1280,360]){await t.call('Emulation.setDeviceMetricsOverride',{width,height:860,deviceScaleFactor:1,mobile:false});await t.screenshot('stockdb-clock-'+width);}

 const shown=path=>path.toLowerCase().startsWith('\\\\?\\unc\\')?'\\\\'+path.slice(8):path.slice(4);
 const pathCases=[[null,''],[undefined,''],['',''],[paths.engine,shown(paths.engine)],[paths.updater,shown(paths.updater)],[paths.candidateDrive,shown(paths.candidateDrive)],[paths.candidateUnc,shown(paths.candidateUnc)],['\\\\?\\unc\\server\\share\\目录','\\\\server\\share\\目录'],['C:\\常规\\claude.exe','C:\\常规\\claude.exe'],['\\\\server\\共享\\claude.exe','\\\\server\\共享\\claude.exe'],['/tmp/claude','/tmp/claude'],['https://example.test/path','https://example.test/path'],['说明中的 '+paths.engine,'说明中的 '+paths.engine],['\\\\?\\Volume{abc}\\','\\\\?\\Volume{abc}\\'],['\\\\.\\pipe\\agent','\\\\.\\pipe\\agent'],['\\\\?\\C:relative','\\\\?\\C:relative']];
 for(const [raw,expected] of pathCases)assert.equal(await t.evaluate('window.__displayPath('+JSON.stringify(raw)+')'),expected);
 extraChecks.push('Path helper: drive/UNC/long/Unicode/empty; ordinary paths, prose and device paths unchanged');
 const navigate=async label=>{await t.evaluate('[...document.querySelectorAll(".section-nav button")].find(button=>button.innerText.includes('+JSON.stringify(label)+')).click()');await t.wait('document.querySelector(".settings-panel")');};
 const checkPath=async(selector,raw)=>{const expected=shown(raw);assert.equal(await t.evaluate('document.querySelector('+JSON.stringify(selector)+').textContent'),expected);assert.equal(await t.evaluate('document.querySelector('+JSON.stringify(selector)+').title'),expected);};
 const copy=async(label,raw)=>{const count=await t.evaluate('window.__stockdbClock.copied.length');await t.evaluate('document.querySelector('+JSON.stringify('[aria-label="'+label+'"]')+').click()');await t.wait('window.__stockdbClock.copied.length>'+count+'&&document.querySelector(".copy-feedback")?.innerText==="路径已复制"');assert.equal(await t.evaluate('window.__stockdbClock.copied.at(-1)'),shown(raw));};
 const rows=await t.evaluate('[...document.querySelectorAll(".history-program-row")].map(row=>({path:row.querySelector("code").textContent,title:row.querySelector("code").title}))');assert.deepEqual(rows,[paths.engine,paths.updater].map(raw=>({path:shown(raw),title:shown(raw)})));
 await copy('复制 stockdb 程序路径',paths.engine);await copy('复制数据更新程序路径',paths.updater);
 const candidates=await t.evaluate('[...document.querySelectorAll(".candidate-btn")].map(button=>({path:button.querySelector("code").textContent,title:button.title}))');assert.deepEqual(candidates,[paths.candidateDrive,paths.candidateUnc].map(raw=>({path:shown(raw),title:shown(raw)})));
 for(const [index,raw] of [paths.candidateDrive,paths.candidateUnc].entries()){await t.evaluate('document.querySelectorAll(".candidate-row")['+index+'].querySelector("button[aria-label]").click()');await t.wait('document.querySelector(".copy-feedback")');assert.equal(await t.evaluate('window.__stockdbClock.copied.at(-1)'),shown(raw));await t.evaluate('document.querySelectorAll(".candidate-btn")['+index+'].click()');await t.wait('window.__stockdbClock.calls.filter(c=>c.command==="select_stockdb_engine").at(-1)?.args.path==='+JSON.stringify(raw));assert.equal(await t.evaluate('window.__clockSettings.localHistoryEnginePath'),raw);}
 extraChecks.push('StockDB paths/candidates show complete drive/UNC with titles/copy; selection IPC retains raw prefix');
 for(const [command,raw] of [['select_stockdb_engine',paths.engine],['select_stockdb_updater',paths.updater]]){await t.evaluate('window.__stockdbClock.dialogPaths.push('+JSON.stringify(raw)+');document.querySelectorAll(".history-program-row")['+(command==='select_stockdb_engine'?0:1)+'].querySelector(".field-btns button:last-child").click()');await t.wait('window.__stockdbClock.calls.filter(c=>c.command==='+JSON.stringify(command)+').at(-1)?.args.path==='+JSON.stringify(raw));}
 extraChecks.push('StockDB browse passes original extended drive/UNC paths to IPC');
 const readable=async(label)=>{await t.evaluate('new Promise(requestAnimationFrame)');await t.evaluate('Promise.all(document.querySelector(".settings-panel").getAnimations({subtree:true}).map(animation=>animation.finished.catch(()=>{})))');const bad=await t.evaluate('([...document.querySelectorAll(".settings-shell *, .settings-footer *")].filter(el=>el.getClientRects().length&&([...el.childNodes].some(node=>node.nodeType===Node.TEXT_NODE&&node.textContent.trim())||el.tagName==="INPUT"&&!["radio","checkbox","range"].includes(el.type))).filter(el=>parseFloat(getComputedStyle(el).fontSize)<12).map(el=>({tag:el.tagName,cls:el.className,text:el.textContent.slice(0,70),size:getComputedStyle(el).fontSize})))');assert.deepEqual(bad,[],label+' visible labels/inputs >=12px');assert.ok(await t.evaluate('document.querySelector(".settings-content").scrollWidth<=document.querySelector(".settings-content").clientWidth+1'),label+' no horizontal path overflow');assert.doesNotMatch(await t.evaluate('document.querySelector(".settings-shell").innerText'),/\\\\\?\\/);};
 for(const style of ['classic','modern','elegant'])for(const theme of ['light','dark'])for(const width of [360,1280]){await t.evaluate('window.__clockSettings.applyVisualStyle('+JSON.stringify(style)+');window.__clockSettings.applyTheme('+JSON.stringify(theme)+')');await t.call('Emulation.setDeviceMetricsOverride',{width,height:860,deviceScaleFactor:1,mobile:false});await readable(style+' '+theme+' '+width+' market');assert.ok(await t.evaluate('[...document.querySelectorAll(".candidate-btn code")].every(el=>el.scrollWidth<=el.clientWidth+1 && getComputedStyle(el).whiteSpace==="pre-wrap")'),'Long candidate paths wrap without truncation');assert.equal(await t.evaluate('getComputedStyle(document.body).fontSize'),'14px');await t.evaluate('(()=>{const pane=document.querySelector(".settings-content"),row=document.querySelector(".history-program-row");pane.scrollTop+=row.getBoundingClientRect().top-pane.getBoundingClientRect().top;})()');await t.evaluate('new Promise(requestAnimationFrame)');await t.screenshot('settings-paths-'+style+'-'+theme+'-'+width);}
 extraChecks.push('StockDB long drive/UNC wraps, body14/labels>=12/copy at360/1280 in three styles and both brightness modes');
 await t.evaluate('window.__clockSettings.applyVisualStyle("classic");window.__clockSettings.applyTheme("light")');
 await navigate('智能');await t.wait('document.querySelector(".history-status small")');
 const agentInput=label=>'[...document.querySelectorAll(".settings-panel .history-field")].find(field=>field.querySelector("span").innerText==='+JSON.stringify(label)+').querySelector("input")';
 for(const [label,raw] of [['可执行文件',paths.agent],['工作目录',paths.agentRoot]]){assert.equal(await t.evaluate(agentInput(label)+'.value'),shown(raw));assert.equal(await t.evaluate(agentInput(label)+'.title'),shown(raw));}
 await checkPath('.history-status small code',paths.agent);await checkPath('.run-dir-line code',paths.agentRoot);await copy('复制检测到的 Claude 路径',paths.agent);await copy('复制 Claude 配置路径',paths.agent);await copy('复制 Claude 工作目录',paths.agentRoot);await copy('复制任务文件目录',paths.agentRoot);
 await t.click('保存并检测');await t.wait('window.__stockdbClock.calls.some(c=>c.command==="set_setting"&&c.args.key==="agent_claude_path"&&c.args.value==='+JSON.stringify(paths.agent)+')');
 await t.evaluate('[...document.querySelectorAll(".history-field")].find(field=>field.querySelector("span").innerText==="工作目录").querySelectorAll("button")[1].click()');await t.wait('window.__stockdbClock.calls.some(c=>c.command==="set_setting"&&c.args.key==="agent_run_root"&&c.args.value==='+JSON.stringify(paths.agentRoot)+')');
 extraChecks.push('Claude editable inputs/status/run_dir show/copy clean paths; untouched saves retain raw drive/UNC IPC');
 for(const [label,raw,key] of [['可执行文件',paths.browsedAgent,'agent_claude_path'],['工作目录',paths.browsedRoot,'agent_run_root']]){await t.evaluate('window.__stockdbClock.dialogPaths.push('+JSON.stringify(raw)+');[...document.querySelectorAll(".history-field")].find(field=>field.querySelector("span").innerText==='+JSON.stringify(label)+').querySelector("button").click()');await t.wait('window.__stockdbClock.settings['+JSON.stringify(key)+']==='+JSON.stringify(raw));assert.equal(await t.evaluate(agentInput(label)+'.value'),shown(raw));}
 extraChecks.push('Claude browse persists raw executable/work directory and shows clean paths');
 await t.click('完成');await t.wait('!document.querySelector(".settings-panel")');assert.deepEqual(await t.evaluate('window.__stockdbClock.confirmations'),[],'Formatting does not create an unsaved edit');await t.evaluate('window.__settingsShow.value=true');await t.wait('document.querySelector(".settings-panel")');await navigate('智能');await t.wait('document.querySelector(".history-status small")');
 extraChecks.push('Open/close without editing does not report prefix-only changes');
 await t.evaluate(`[...document.querySelectorAll('.section-nav button')].find(button=>button.innerText.includes('智能')).click()`);
 await t.wait('document.querySelector(".settings-panel")?.innerText.includes("隔离 Claude 状态")');
 const aiText=await t.evaluate('document.querySelector(".settings-panel").innerText');
 assert.doesNotMatch(aiText,/手动触发，无需开关|下面这些只有你主动点击|量化评分|推荐榜/,'Redundant static manual-trigger card is absent from the real AI page');
 assert.equal(await t.evaluate('document.querySelectorAll(".settings-panel .setting-card").length'),2,'Master switch and Claude configuration remain');
 assert.equal(await t.evaluate('document.querySelectorAll(".settings-panel .compact-card,.settings-panel .explain-grid").length'),0);
 assert.doesNotMatch(aiText,/自动运行的功能|智能监控：量化止损止盈/,'Removed automatic-feature card stays absent');
 assert.match(aiText,/AI 智能总开关.*本地 Agent · Claude Code.*可执行文件.*工作目录.*单次超时（秒）.*后台单次预算（美元）/s);
 assert.match(aiText,/此预算仅限制.*任务文件实际写入.*测试连接会进行一次/s,'Setting-specific guidance remains');
 await t.evaluate(`document.querySelector('[aria-label="AI 智能总开关"]').click()`);
 await t.wait('window.__clockSettings.aiEnabled===false');
 assert.equal(await t.evaluate('window.__stockdbClock.settings.ai_enabled'),'0');
 assert.equal(await t.evaluate('window.__clockSettings.aiMonitorEnabled'),true,'Removing the independent card preserves existing preference');
 await t.evaluate(`document.querySelector('[aria-label="AI 智能总开关"]').click()`);
 await t.wait('window.__clockSettings.aiEnabled===true');
 assert.equal(await t.evaluate('window.__stockdbClock.settings.ai_enabled'),'1');
 const setAgentField=async(label,value)=>t.evaluate(`(()=>{const field=[...document.querySelectorAll('.settings-panel .history-field')].find(field=>field.querySelector('span').innerText===${JSON.stringify(label)});const input=field.querySelector('input');input.value=${JSON.stringify(value)};input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
 await setAgentField('可执行文件','C:/isolated/custom-claude.cmd');await t.click('保存并检测');
 await t.wait(`window.__stockdbClock.settings.agent_claude_path==='C:/isolated/custom-claude.cmd' && document.querySelector('.history-status small')?.innerText.includes('custom-claude.cmd')`);
 await setAgentField('工作目录','C:/isolated/agent-work');
 await t.evaluate(`[...document.querySelectorAll('.settings-panel .history-field')].find(field=>field.querySelector('span').innerText==='工作目录').querySelectorAll('button')[1].click()`);
 await t.wait(`window.__stockdbClock.settings.agent_run_root==='C:/isolated/agent-work' && document.querySelector('.run-dir-line')?.innerText.includes('C:/isolated/agent-work')`);
 await setAgentField('单次超时（秒）','240');await t.click('保存超时');
 await t.wait(`window.__stockdbClock.settings.agent_timeout_seconds==='240'`);
 await setAgentField('后台单次预算（美元）','12.5');await t.click('保存预算');
 await t.wait(`window.__stockdbClock.settings.agent_budget_usd==='12.50'`);
 assert.equal(await t.evaluate(`[...document.querySelectorAll('.settings-panel button')].find(button=>button.innerText==='测试连接').disabled`),false,'Manual Claude connection check remains available');

 await setAgentField('工作目录','\\\\server\\共享任务\\自定义目录');await t.evaluate('[...document.querySelectorAll(".history-field")].find(field=>field.querySelector("span").innerText==="工作目录").querySelectorAll("button")[1].click()');await t.wait('window.__stockdbClock.settings.agent_run_root==="\\\\\\\\server\\\\共享任务\\\\自定义目录"');assert.equal(await t.evaluate('window.__stockdbClock.settings.agent_timeout_seconds'),'240');assert.equal(await t.evaluate('window.__stockdbClock.settings.agent_budget_usd'),'12.50');
 extraChecks.push('Editing prefix-free custom Claude paths persists valid UNC and preserves custom timeout/budget');
 const beforeCopyFailure=await t.evaluate('window.__stockdbClock.copied.length');await t.evaluate('window.__stockdbClock.copyFails=true;document.querySelector(".history-field [aria-label]").click()');await t.wait('document.querySelector(".settings-error")?.innerText.includes("隔离复制失败")');assert.equal(await t.evaluate('window.__stockdbClock.copied.length'),beforeCopyFailure);await t.evaluate('window.__stockdbClock.copyFails=false');await t.click('保存超时');await t.wait('!document.querySelector(".settings-error")');
 extraChecks.push('Clipboard failure is shown through existing action error without claiming success');
 const luminosity=hex=>{const rgb=hex.trim().slice(1).match(/../g).map(value=>parseInt(value,16)/255).map(value=>value<=0.04045?value/12.92:((value+0.055)/1.055)**2.4);return rgb[0]*0.2126+rgb[1]*0.7152+rgb[2]*0.0722;};
 for(const style of ['classic','modern','elegant'])for(const theme of ['light','dark'])for(const width of [360,1280]){
  await t.evaluate('window.__clockSettings.applyVisualStyle('+JSON.stringify(style)+');window.__clockSettings.applyTheme('+JSON.stringify(theme)+')');await t.call('Emulation.setDeviceMetricsOverride',{width,height:860,deviceScaleFactor:1,mobile:false});const label=style+' '+theme+' '+width;
  await navigate('智能');await readable(label+' ai');assert.equal(await t.evaluate(agentInput('单次超时（秒）')+'.value'),'240');assert.equal(await t.evaluate(agentInput('后台单次预算（美元）')+'.value'),'12.5');assert.equal(await t.evaluate('document.querySelectorAll(".settings-panel [aria-label^=复制]").length'),4);await t.screenshot('settings-ai-readable-'+label.replaceAll(' ','-'));
  const tokens=await t.evaluate('(()=>{const s=getComputedStyle(document.documentElement);return Object.fromEntries(["--color-text-secondary","--color-text-tertiary",...Array.from({length:4},(_,i)=>"--color-surface-"+i)].map(key=>[key,s.getPropertyValue(key)]))})()');for(const fg of ['--color-text-secondary','--color-text-tertiary'])for(const bg of ['--color-surface-0','--color-surface-1','--color-surface-2','--color-surface-3']){const a=luminosity(tokens[fg]),b=luminosity(tokens[bg]);assert.ok((Math.max(a,b)+.05)/(Math.min(a,b)+.05)>=4.5,label+' '+fg+' against '+bg+' >=4.5');}
  await navigate('系统');await t.wait('document.querySelectorAll(".system-line.history-program-row").length===4');await readable(label+' system');const displayed=await t.evaluate('[...document.querySelectorAll(".system-line.history-program-row")].map(row=>({text:row.querySelector("code").textContent,title:row.querySelector("code").title,copy:!!row.querySelector("button[aria-label]")}))');assert.deepEqual(displayed,[paths.exe,paths.data,paths.database,paths.tasks].map(raw=>({text:shown(raw),title:shown(raw),copy:true})));assert.doesNotMatch(await t.evaluate('getComputedStyle(document.querySelector(".path-text")).fontFamily'),/Consolas|Cascadia|monospace/);await t.screenshot('settings-system-paths-'+label.replaceAll(' ','-'));
  await navigate('外观');await readable(label+' appearance');await navigate('提醒');await readable(label+' reminders');await navigate('悬浮窗');await t.wait('window.__stockdbClock.calls.some(c=>c.command==="get_group_hotkey_status")');await t.evaluate('document.querySelector(".settings-content .hotkey-box").click()');await readable(label+' ticker');await navigate('智能');
 }
 extraChecks.push('AI/system/appearance/reminder/ticker labels>=12 and secondary text contrast>=4.5 in six theme combinations at360/1280');
 await navigate('系统');for(const [label,raw] of [['程序位置',paths.exe],['数据目录',paths.data],['设置数据库',paths.database],['AI 会话文件',paths.tasks]])await copy('复制'+label+'路径',raw);
 extraChecks.push('Program/data/database/interactive task paths have full titles, wrap, copy and sans Chinese');
 await t.evaluate('window.__clockSettings.applyVisualStyle("classic");window.__clockSettings.applyTheme("light")');

 await t.evaluate(`[...document.querySelectorAll('.section-nav button')].find(button=>button.innerText.includes('提醒')).click()`);
 await t.wait('document.querySelector(".settings-panel .explain-grid") && window.__stockdbClock.calls.some(c=>c.command==="get_notification_identity_status")');
 assert.match(await t.evaluate('document.querySelector(".settings-panel .explain-grid").innerText'),/重新穿越.*冷却间隔.*每日一次/s,'Shared explanation grid is retained for actionable reminder settings');
 assert.ok(await t.evaluate('document.querySelector(".settings-panel .compact-card").getBoundingClientRect().height>0'),'Shared compact-card styling remains available on the reminder page');
 await t.evaluate('window.__clockSettings.stockDbStatus.state="running_external"');
 assert.equal(await t.evaluate(`document.querySelector('[aria-label="更新本地 stockdb 数据"]').disabled`),false,'Manual update remains enabled for an external local service with auto update off');
 await t.evaluate(`document.querySelector('[aria-label="更新本地 stockdb 数据"]').click()`);
 await t.wait('document.body.innerText.includes("隔离手动更新成功，服务已恢复")');
 const commands=await t.evaluate('window.__stockdbClock.calls.map(c=>c.command)');
 assert.equal(commands.filter(c=>c==='run_stockdb_update').length,1,'One explicit manual click invokes one isolated update');
 assert.ok(!commands.some(c=>/research_follow|simulation|start_stockdb|test_agent_connection|analyze|run_mainline_discovery/.test(c)),'No trading, Claude calls, discovery or real service starts');
 assert.deepEqual(t.errors,[]);
 const baseChecks=['Default09:00 trading-day text and verified completed date','fixed1minute/fivefailure text/no retry settings','valid time committed','auto switch saves0','failed time save preserves previous value','manual TopBar update retained and enabled for external service with automatic update paused','AI page has no redundant automatic-feature or manual-trigger cards','master switch saves without losing existing monitor preference','Claude path/directory saves and status read remain','Claude timeout/budget save and manual connection control remain','reminder shared compact-card/explain-grid remain','one manual synthetic update and no trading/Claude/discovery calls' ];const checks=[...baseChecks,...extraChecks];t.save({result:'passed',cases:checks.length,checks,commands,pathCases:pathCases.length,themeWidths:{styles:['classic','modern','elegant'],brightness:['light','dark'],widths:[360,1280]},limitation:'Real browser, Vue components and stores; IPC/dialog/clipboard are isolated fixtures. Native services, user DB/StockDB and Claude connection are not invoked.'});console.log('StockDB schedule, readable settings and path display UI passed: '+checks.length+' checks; '+t.output);
}finally{await t.close();}
