import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';

const port = Number(process.argv[2]);
assert.ok(Number.isInteger(port) && port > 0, '用法：node scripts/check-research-wizard.mjs <WebView2-CDP-端口>');
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const main = pages.find(page => page.url === 'http://tauri.localhost/');
assert.ok(main, '未找到主窗口的 WebView');
const ws = new WebSocket(main.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.addEventListener('open', resolve, { once: true });
  ws.addEventListener('error', reject, { once: true });
});
let nextId = 0;
function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    const timeout = setTimeout(() => { ws.removeEventListener('message', onMessage); reject(new Error(`${method} 超时`)); }, 10000);
    const onMessage = event => {
      const response = JSON.parse(event.data);
      if (response.id !== id) return;
      clearTimeout(timeout);
      ws.removeEventListener('message', onMessage);
      if (response.error) reject(new Error(JSON.stringify(response.error)));
      else resolve(response.result);
    };
    ws.addEventListener('message', onMessage);
    ws.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
  return result.result.value;
}
const button = text => `[...document.querySelectorAll('.research-center button')].find(b=>b.textContent.trim()===${JSON.stringify(text)})`;
async function waitFor(expression, message) {
  const until = Date.now() + 8000;
  while (Date.now() < until) {
    if (await evaluate(expression)) return;
    await new Promise(resolve => setTimeout(resolve, 80));
  }
  const diagnostic = await evaluate(`JSON.stringify({banner:[...document.querySelectorAll('.research-center .banner')].map(x=>x.textContent.trim()),wizard:document.querySelector('.wizard-body')?.textContent.slice(0,1200),candidateCount:document.querySelectorAll('.wizard-candidates button').length,mock:window.__bullResearchMock&&{calls:window.__bullResearchMock.calls,stock:window.__bullResearchMock.stock}})`);
  throw new Error(`${message}\n${diagnostic}`);
}
async function click(text) {
  await waitFor(`!!${button(text)} && !${button(text)}.disabled`, `按钮不可用：${text}`);
  await evaluate(`${button(text)}.click()`);
}
async function disabled(text, expected) {
  await waitFor(`!!${button(text)} && ${button(text)}.disabled===${expected}`, `${text} 的禁用状态应为 ${expected}`);
}
async function idle() {
  await waitFor(`![...document.querySelectorAll('.research-center .banner')].some(b=>b.getAttribute('role')==='status')`, '研究动作未结束');
}
async function capture() {
  if (!process.env.BULL_RESEARCH_SCREENSHOT) return;
  const png = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  writeFileSync(process.env.BULL_RESEARCH_SCREENSHOT, Buffer.from(png.data, 'base64'));
  console.log(`截图：${process.env.BULL_RESEARCH_SCREENSHOT}`);
}

try {
  await evaluate(String.raw`(() => {
    if (window.__bullResearchMock) throw new Error('本页面已安装研究 mock；请先重启测试窗口');
    const originalFetch = window.fetch;
    const originalPost = window.chrome.webview.postMessage;
    const clone = value => JSON.parse(JSON.stringify(value));
    const today = new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Shanghai' });
    const stale = new Date(Date.parse(today) - 14 * 86400000).toISOString().slice(0, 10);
    const config = { execution_mode:'realtime',auto_research:false,observation_days:28,min_samples:20,max_drawdown_bps:1500,min_return_bps:0,initial_cash:'1000000000',max_active:3,stock_count:5,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:1,slippage_bps:5 };
    const state = window.__bullResearchMock = { originalFetch, originalPost, today, historyDate:stale, testState:'failed', config, experiments:[], calls:[], stock:{enabled:false,platformSupported:true,state:'not_configured',phase:null,enginePath:null,updaterPath:null,updaterAvailable:false,owned:false,busy:false,message:'测试：待选择历史程序',lastError:null,candidates:[]} };
    const candidate = id => ({id,version_id:id,name:id===1?'测试趋势候选':'测试导入候选',hypothesis:'仅用于验证设置向导的隔离候选，请核对买入理由，确认前不得创建模拟账户。',rule:'trend_follow',state:'candidate',account_id:null,created_at:today,last_message:'测试：候选已登记，尚未启动',selection_json:'{}',filter:{},config:clone(state.config),remaining_days:28,remaining_samples:20,elapsed_days:0,verdict:'candidate',detail:null,curve:[]});
    const respond = async (command, args = {}) => {
      if (command==='get_stockdb_status') return clone(state.stock);
      if (command==='get_local_history_status'||command==='test_local_history') return {state:'connected',message:'测试：本地历史连接正常',start_date:'2020-01-02',end_date:state.historyDate,sample_count:1600,candidates:[]};
      if (command==='get_agent_status') return {installed:true,state:'detected',path:'C:\\mock\\claude.exe',message:'测试：程序已找到，尚未连接',guidance:'请运行连接测试',run_dir:'C:\\mock\\agent'};
      if (command==='test_agent_connection') { state.calls.push({command,args}); return {installed:true,state:state.testState,path:'C:\\mock\\claude.exe',message:state.testState==='ready'?'测试：连接成功':'测试：登录失败',guidance:'测试状态',run_dir:'C:\\mock\\agent'}; }
      if (command==='research_dashboard') return clone({config:state.config,experiments:state.experiments,local_ready:state.stock.enabled,agent_installed:true,last_auto_message:'',busy:false});
      if (command==='scan_stockdb') { state.calls.push({command,args}); state.stock.candidates=[{enginePath:'C:\\mock\\stockdb.exe',updaterPath:'C:\\mock\\数据更新.exe',source:'测试程序'}]; return clone(state.stock); }
      if (command==='select_stockdb_engine') { state.calls.push({command,args}); Object.assign(state.stock,{enginePath:args.path,updaterPath:'C:\\mock\\数据更新.exe',updaterAvailable:true,state:'configured',message:'测试：程序已选定'}); return clone(state.stock); }
      if (command==='set_local_history_enabled') { state.calls.push({command,args}); Object.assign(state.stock,{enabled:args.enabled,state:args.enabled?'running_owned':'disabled',owned:args.enabled,message:'测试：历史程序已启动'}); return clone(state.stock); }
      if (command==='save_research_config') { state.calls.push({command,args}); await new Promise(resolve=>setTimeout(resolve,240)); state.config=clone(args.config); return; }
      if (command==='research_discover'||command==='import_research_candidate') { state.calls.push({command,args}); const id=state.experiments.length+1; state.experiments.push(candidate(id)); return id; }
      if (command==='research_start') { state.calls.push({command,args}); const row=state.experiments.find(e=>e.id===args.experimentId); if(!row) throw new Error('测试：不存在该候选'); if(row.account_id) throw new Error('测试：重复启动'); Object.assign(row,{account_id:100+row.id,state:'observing',last_message:'测试：已确认启动隔离模拟账户',detail:{metrics:{total_return_bps:0,max_drawdown_bps:0,sample_count:0,win_rate_bps:0,benchmark_return_bps:null,equity:row.config.initial_cash},targets:[{symbol:'sh600519',name:'测试标的'}],orders:[]}}); return; }
      if (['research_workspace','open_research_claude'].includes(command)) { state.calls.push({command,args}); return {path:'C:\\mock\\research',skills:[],prompt:'测试任务书'}; }
      if (['select_stockdb_updater','run_stockdb_update','research_action','cancel_agent_analysis'].includes(command)) throw new Error('本检查不允许此研究操作：'+command);
      if (command==='set_setting' && ['agent_claude_path','agent_run_root','local_history_url'].includes(args.key)) { state.calls.push({command,args}); return; }
      throw new Error('未覆盖研究命令：'+command);
    };
    const commands = new Set(['get_stockdb_status','get_local_history_status','test_local_history','get_agent_status','test_agent_connection','research_dashboard','scan_stockdb','select_stockdb_engine','set_local_history_enabled','save_research_config','research_discover','import_research_candidate','research_start','research_workspace','open_research_claude','select_stockdb_updater','run_stockdb_update','research_action','cancel_agent_analysis']);
    const mocked = (command,args) => commands.has(command) || (command==='set_setting' && ['agent_claude_path','agent_run_root','local_history_url'].includes(args?.key));
    window.fetch = async (input, options) => {
      const url = typeof input === 'string' ? input : input.url;
      const ipc = url.startsWith('http://ipc.localhost/') ? decodeURIComponent(new URL(url).pathname.slice(1)) : null;
      const args = options?.body ? JSON.parse(typeof options.body==='string'?options.body:new TextDecoder().decode(options.body)) : {};
      if (!mocked(ipc,args)) return originalFetch.call(window, input, options);
      try {
        const result = await respond(ipc,args);
        return new Response(JSON.stringify(result??null),{status:200,headers:{'Content-Type':'application/json','Tauri-Response':'ok'}});
      } catch(error) {
        return new Response(JSON.stringify(String(error)),{status:200,headers:{'Content-Type':'application/json','Tauri-Response':'error'}});
      }
    };
    window.chrome.webview.postMessage = function(message) {
      let envelope;
      try { envelope = typeof message==='string'?JSON.parse(message):message; } catch { return originalPost.call(this,message); }
      if (!mocked(envelope?.cmd,envelope?.payload)) return originalPost.call(this,message);
      Promise.resolve().then(()=>respond(envelope.cmd,envelope.payload||{})).then(
        result=>window.__TAURI_INTERNALS__.runCallback(envelope.callback,result??null),
        error=>window.__TAURI_INTERNALS__.runCallback(envelope.error,String(error))
      );
    };
  })()`);
  assert.equal(await evaluate(`window.__TAURI_INTERNALS__.invoke('get_stockdb_status').then(s=>s.message)`), '测试：待选择历史程序', '研究 IPC mock 未生效，停止以避免实际写入');
  await evaluate(`document.querySelector('.research-modal button[aria-label="close"]')?.click()`);
  await waitFor(`!![...document.querySelectorAll('.top-bar button')].find(b=>b.textContent.trim()==='研究中心')`, '研究中心入口不存在');
  await evaluate(`[...document.querySelectorAll('.top-bar button')].find(b=>b.textContent.trim()==='研究中心').click()`);
  await waitFor(`document.querySelector('.research-center nav button.active')?.textContent.trim()==='设置向导'`, '首次进入未默认显示设置向导');
  await disabled('下一步：连接 Claude', true);
  await click('自动查找');
  await waitFor(`!!document.querySelector('.wizard-candidates button')`, '未展示查找到的历史程序');
  await evaluate(`document.querySelector('.wizard-candidates button').click()`);
  await idle();
  await click('启用并启动');
  await idle();
  await disabled('下一步：连接 Claude', true);
  assert.match(await evaluate(`document.querySelector('.wizard-result').textContent`), /14 天/);
  await evaluate(`window.__bullResearchMock.historyDate=window.__bullResearchMock.today`);
  await click('测试历史连接');
  await disabled('下一步：连接 Claude', false);
  await click('下一步：连接 Claude');
  await disabled('下一步：设定验证条件', true);
  await click('真实测试连接');
  await idle();
  await disabled('下一步：设定验证条件', true);
  assert.match(await evaluate(`document.querySelector('.wizard-result').textContent`), /登录失败/);
  await evaluate(`window.__bullResearchMock.testState='ready'`);
  await click('真实测试连接');
  await disabled('下一步：设定验证条件', false);
  await click('下一步：设定验证条件');
  await disabled('下一步：开始研究', true);
  await click('确认并保存验证条件');
  assert.equal(await evaluate(`document.querySelector('.config-grid')?.closest('fieldset')?.disabled`), true, '保存过程中应禁用整个配置表单');
  assert.equal(await evaluate(`document.querySelector('.config-grid input')?.matches(':disabled')`), true, '输入应继承 fieldset 的禁用状态');
  await disabled('下一步：开始研究', false);
  await evaluate(`(() => {const input=document.querySelector('.config-grid input');input.value='35';input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await disabled('下一步：开始研究', true);
  await click('确认并保存验证条件');
  await disabled('下一步：开始研究', false);
  assert.equal(await evaluate(`window.__bullResearchMock.config.observation_days`), 35);
  await click('下一步：开始研究');
  await click('AI 提出候选');
  await idle();
  assert.equal(await evaluate(`window.__bullResearchMock.experiments[0].account_id`), null, '生成候选不应自动创建账户');
  assert.equal(await evaluate(`window.__bullResearchMock.calls.filter(c=>c.command==='research_start').length`), 0);
  await click('确认后自动选股并启动验证');
  await idle();
  assert.equal(await evaluate(`window.__bullResearchMock.calls.filter(c=>c.command==='research_start').length`), 1);
  assert.equal(await evaluate(`window.__bullResearchMock.calls.find(c=>c.command==='research_start').args.experimentId`), 1);
  await waitFor(`document.querySelector('.wizard-result')?.textContent.includes('首次模拟已启动')`, '未显示首次模拟完成状态');
  await capture();
  await click('查看模拟账户与买卖理由');
  await waitFor(`document.querySelector('.detail h3')?.textContent==='测试趋势候选'`, '无法查看已启动的实验');
  await click('Claude 交互研究');
  await click('导入 candidate.json');
  await idle();
  await waitFor(`document.querySelector('.detail h3')?.textContent==='测试导入候选'`, '无法查看导入的候选');
  assert.equal(await evaluate(`window.__bullResearchMock.experiments[1].account_id`), null, '导入不应自动创建账户');
  assert.equal(await evaluate(`window.__bullResearchMock.calls.filter(c=>c.command==='research_start').length`), 1);
  await evaluate(`document.querySelector('.research-modal button[aria-label="close"]').click()`);
  await waitFor(`!document.querySelector('.research-center')`, '研究中心未关闭');
  await evaluate(`[...document.querySelectorAll('.top-bar button')].find(b=>b.textContent.trim()==='研究中心').click()`);
  await waitFor(`document.querySelector('.research-center nav button.active')?.textContent.trim()==='策略实验' && document.querySelectorAll('.experiment').length===2`, '已有实验未默认显示或无法查看');
  await click('设置向导');
  await click('下一步：连接 Claude');
  await disabled('下一步：设定验证条件', true);
  console.log('研究向导检查通过：首次引导、历史过期门禁、Claude 失败门禁、保存与修改、手动确认启动、导入及已有实验；全部研究操作为 mock。');
} finally {
  await evaluate(`(() => {const s=window.__bullResearchMock;if(s){window.fetch=s.originalFetch;window.chrome.webview.postMessage=s.originalPost;delete window.__bullResearchMock;}})()`).catch(() => {});
  ws.close();
}
