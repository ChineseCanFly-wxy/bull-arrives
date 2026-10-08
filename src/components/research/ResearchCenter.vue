<script setup lang="ts">
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import { computed, nextTick, onBeforeUnmount, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { NModal } from 'naive-ui';
import { useSettingsStore } from '@/stores/settings';
import { displayPath } from '@/utils/pathDisplay';
import ResearchEvidence from './ResearchEvidence.vue';
import ResearchExtensions from './ResearchExtensions.vue';
import ResearchJobPanel from './ResearchJobPanel.vue';
import { emit as emitAppEvent } from '@tauri-apps/api/event';
import DailyResearch from './DailyResearch.vue';
import ModelFollowTrading from './ModelFollowTrading.vue';
import NestedResearch from './NestedResearch.vue';
import { jobActive, type ResearchJob } from '@/types/research';
const props=defineProps<{show:boolean}>();
const emit=defineEmits<{'update:show':[boolean];'open-settings':[string]}>();
const appSettings=useSettingsStore();
async function openSetting(section:string){if(section==='alerts'||section==='notifications')await emitAppEvent('notification-open-settings','trades');else emit('open-settings',section);}
interface Config {auto_research:boolean;observation_days:number;min_samples:number;max_drawdown_bps:number;min_return_bps:number;initial_cash:string;max_active:number;stock_count:number;commission_bps:number;min_commission:string;stamp_tax_bps:number;transfer_fee_bps:number;slippage_bps:number}
interface Metrics {total_return_bps:number;max_drawdown_bps:number;sample_count:number;win_rate_bps:number;benchmark_return_bps:number|null;equity:string}
interface Experiment {id:number;version_id:number;name:string;hypothesis:string;rule:string;state:string;account_id:number|null;created_at:string;last_message:string;selection_json:string;filter:Record<string,unknown>;config:Config;remaining_days:number;remaining_samples:number;elapsed_days:number;verdict:string;detail:null|{metrics:Metrics;targets:Array<{symbol:string;name:string}>;orders:Array<{id:number;signal_date:string;symbol:string;side:string;quantity:number;status:string;reject_reason:string|null;decision_reason:string|null}>};curve:Array<{date:string;equity:string;return_bps:number;drawdown_bps:number}>}
interface Dashboard {config:Config;experiments:Experiment[];local_ready:boolean;agent_installed:boolean;last_auto_message:string;busy:boolean}
const dashboard=ref<Dashboard|null>(null);const selectedId=ref<number|null>(null);const busy=ref('');const error=ref('');const notice=ref('');
interface ModelOrder {date:number;code:string;side:string;qty:number;price:number;fee:number;reason:string}
interface ModelRun {id:number;model_id:string;model_name:string;score_semantic:string;mode:string;as_of:string;holding_days:number;comparison:string;state:string;enabled:boolean;forward_start:number|null;content_sha256:string;model_sha256:string;data_sha256:Record<string,string>;signal_watch:Array<{symbol:string;score:number;threshold:number;as_of:string;cash_reference_quantity:number;cash_reference_stage:string;reason:string}>;limitations:string[];ledger:{metrics:{net_return_pct:number;max_drawdown_pct:number;completed_holding_cycles:number;win_rate_pct:number|null;open_positions:number;max_overdue_sessions:number};curve:Array<{date:number;cash:number;equity:number;positions:number;dividend_receivable:number}>;orders:ModelOrder[];unclosed:Array<{code:string;qty:number;bonus_locked:number;mark:number;unrealized_and_receivable_pnl:number}>}}
interface ModelConfig {research_root:string;python:string;snapshot:string;index:string}
const modelConfig=reactive<ModelConfig>({research_root:'',python:'python',snapshot:'',index:''});
const modelRuns=ref<ModelRun[]>([]);const modelTasks=ref<Array<{id:number;name:string;hypothesis:string;model_id:string;holding_days:number;comparison:string;invalidation:string[];missing_data:string[]}>>([]);const modelError=ref('');const modelSelectedId=ref<number|null>(null);
const jobsBusy=ref(false);const jobsExpanded=ref(false);watch(jobsBusy,value=>{if(value)jobsExpanded.value=true;});const activeJobId=ref<number|null>(null);const jobPanel=ref<InstanceType<typeof ResearchJobPanel>|null>(null);
function readMode(){try{return localStorage.getItem('research-center-mode-v1');}catch{return null;}}
const savedMode=readMode();
const centerMode=ref(savedMode==='advanced'?'advanced':'simple');
const tab=ref(centerMode.value==='simple'?'tracking':'guide');
const nestedPanel=ref<InstanceType<typeof NestedResearch>|null>(null);
const dailyPanel=ref<InstanceType<typeof DailyResearch>|null>(null);
const followPanel=ref<InstanceType<typeof ModelFollowTrading>|null>(null);
watch(centerMode,mode=>{try{localStorage.setItem('research-center-mode-v1',mode);}catch{};tab.value=mode==='simple'?'tracking':(modelRuns.value.length?'model-lab':'guide');archiveOpen.value=false;});
function openRecords(id?:number){tab.value='records';if(id){activeJobId.value=id;jobsExpanded.value=true;void jobPanel.value?.refresh();}}
function startedJob(job:ResearchJob){jobsBusy.value=true;openRecords(job.id);}
async function openEvidence(){centerMode.value='advanced';await nextTick();tab.value='evidence';}
async function openModelRun(id:number){centerMode.value='advanced';await nextTick();modelSelectedId.value=id;tab.value='model-lab';}
const modelId=ref('breadth22_h20');const modelComparison=ref('baseline');
const modelOptions=[{id:'breadth22_h20',name:'广度绝对收益22',note:'结合个股走势与全市场强弱，预测20日收益；分数不是上涨概率'}, {id:'index26_h20',name:'指数绝对收益26',note:'在广度模型上加入沪深300走势，判断个股与大盘环境'}, {id:'breadth22_excess_csi20',name:'CSI超额收益22',note:'预测相对沪深300多赚或少亏多少；正分不保证股票赚钱'}, {id:'breadth22_rank20',name:'截面排序22',note:'比较同日股票的相对排名；按已冻结阈值观察，不代表胜率'}, {id:'breadth22_open_downside20',name:'开盘下行惩罚22',note:'在收益预测中惩罚持有期间较弱的开盘表现；不是盘中止损模型'}];
const modelSelected=computed(()=>modelRuns.value.find(v=>v.id===modelSelectedId.value)??null);
const lockedTail=computed(()=>{const rows=modelSelected.value?.ledger.unclosed??[];return{stocks:rows.filter(v=>v.bonus_locked>0).length,shares:rows.reduce((n,v)=>n+v.bonus_locked,0),lockedMarketValue:rows.reduce((n,v)=>n+v.bonus_locked*(v.mark??0),0),sellableShares:rows.reduce((n,v)=>n+v.qty-v.bonus_locked,0)};});
const modelChart=computed(()=>{const rows=modelSelected.value?.ledger.curve??[];const values=[100000,...rows.map(v=>v.equity)];const lo=Math.min(...values),hi=Math.max(...values),range=hi-lo||1;return values.map((v,i)=>`${20+i/Math.max(1,values.length-1)*660},${160-(v-lo)/range*140}`).join(' ');});
async function loadModels(reset=false){const v=await invoke<{runs:ModelRun[];tasks:typeof modelTasks.value;config:ModelConfig;last_error:string|null}>('research_model_runs');modelRuns.value=v.runs;modelTasks.value=v.tasks;modelError.value=v.last_error??'';if(reset)Object.assign(modelConfig,v.config);if(!modelSelectedId.value&&v.runs.length)modelSelectedId.value=v.runs[0].id;}
async function runModel(mode:string,continuation:ModelRun|null=null){await act('model-run',async()=>{const v=continuation;const job=await invoke<ResearchJob>('research_job_start',{request:{kind:mode,models:[v?.model_id??modelId.value],comparisons:[v?.comparison??modelComparison.value],continuation:v?.id??null}});startedJob(job);notice.value='任务已在后台开始，可关闭页面。完成后保存账本，应用退出后可继续未完成项。';});}
async function compareModel(){await act('model-compare',async()=>{const job=await invoke<ResearchJob>('research_job_start',{request:{kind:'compare',models:[modelId.value],comparisons:['baseline','holding15','cost_double'],continuation:null}});startedJob(job);notice.value='按同一输入运行基准、15日和双费三项对照，逐项保存真实账本。仅比较执行参数，不训练或自动采纳新模型。';});}
async function completedJob(job:ResearchJob){if(job.kind==='explore'){await nestedPanel.value?.refresh();notice.value='五年技术组合研究已完成；结果保留在深入研究中。';return;}await loadModels();const first=job.results.find(r=>r.run_id);if(first?.run_id){modelSelectedId.value=first.run_id;if(job.kind==='forward'){await dailyPanel.value?.refresh(first.run_id);await followPanel.value?.refresh();}}notice.value='后台任务已完成，结果与账本已核验并保存。';}
function jobsUpdated(jobs:ResearchJob[]){jobsBusy.value=jobs.some(jobActive);}

async function saveModelPaths(){await act('model-paths',async()=>{await invoke('research_model_config',{config:{...modelConfig}});notice.value='已保存本机路径。运行前逐个核对受信源与模型SHA，不自动安装依赖或训练替代。';});}
async function chooseModelPath(key:keyof ModelConfig,directory:boolean){const v=await open({multiple:false,directory,title:directory?'选择研究项目/兼容导出目录':'选择Python或显式CSI300文件'});if(typeof v==='string')modelConfig[key]=v;}
async function importModelLedger(){await act('model-import',async()=>{const p=await open({multiple:false,directory:false,filters:[{name:'模型账本JSON',extensions:['json']}]});if(typeof p==='string'){modelSelectedId.value=await invoke<number>('research_import_model_run',{path:p});await loadModels();notice.value='模型身份、来源与账本损益已校验；导入只记录研究证据，不能自动准入。';}});}
async function observeModel(v:ModelRun){await act('model-observe',async()=>{await invoke('research_model_observe',{id:v.id,enabled:!v.enabled});await loadModels();});}
interface HistoryStatus {state:string;message:string;start_date:string|null;end_date:string|null;sample_count:number|null}
const historyStatus=ref<HistoryStatus|null>(null);
const historyUrl=ref('http://127.0.0.1:7899');
const archiveOpen=ref(false);const nestedOpen=ref(false);const extensionsOpen=ref(false);
const scrollEl=ref<HTMLElement|null>(null);
watch(tab,()=>{if(scrollEl.value)scrollEl.value.scrollTop=0;});
const primaryTabs=computed(()=>centerMode.value==='simple'?[{id:'tracking',label:'自动模型交易'},{id:'daily',label:'手动条件提醒'},{id:'records',label:'提醒与记录'}]:[{id:'guide',label:'数据准备与更新'},{id:'model-lab',label:'冻结模型回放'},{id:'evidence',label:'研究证据'},{id:'records',label:'提醒与记录'}]);
const settings=reactive<Config>({auto_research:false,observation_days:28,min_samples:20,max_drawdown_bps:1500,min_return_bps:0,initial_cash:'1000000000',max_active:3,stock_count:5,commission_bps:3,min_commission:'50000',stamp_tax_bps:5,transfer_fee_bps:1,slippage_bps:5});
const selected=computed(()=>dashboard.value?.experiments.find(e=>e.id===selectedId.value)??null);
interface LiveInfo {engine:string;message:string;updated_at:string|null;plans:Array<{symbol:string;buy_low:number;buy_high:number;stop:number;take:number;basis_date:string}>;executions:unknown[];marks:number}
const live=computed(()=>(selected.value as (Experiment&{live?:LiveInfo})|null)?.live);
const daily=computed(()=>(selected.value as (Experiment&{daily_comparison?:Experiment['detail']})|null)?.daily_comparison);
const states:Record<string,string>={candidate:'待启动',observing:'前向验证',extended:'延长观察',qualified:'待采纳',adopted:'持续跟踪',paused:'已暂停',rejected:'已淘汰'};
const rules:Record<string,string>={trend_follow:'趋势回踩',mean_reversion:'均值回归',breakout:'放量突破'};
const pct=(n:number|null|undefined)=>n==null?'—':`${(n/100).toFixed(2)}%`;
const money=(n:string)=> (Number(n)/10000).toLocaleString('zh-CN',{maximumFractionDigits:2});
const counts=computed(()=>({all:dashboard.value?.experiments.length??0,running:dashboard.value?.experiments.filter(e=>['observing','extended','adopted'].includes(e.state)).length??0,rejected:dashboard.value?.experiments.filter(e=>e.state==='rejected').length??0}));
const historyAge=computed(()=>{
  const end=historyStatus.value?.end_date;
  if(!end||!/^\d{4}-\d{2}-\d{2}$/.test(end))return null;
  const today=new Date().toLocaleDateString('sv-SE',{timeZone:'Asia/Shanghai'});
  return Math.round((Date.parse(today)-Date.parse(end))/86400000);
});
const historyReady=computed(()=>!!appSettings.localHistoryEnginePath&&appSettings.localHistoryEnabled&&historyStatus.value?.state==='connected'&&historyAge.value!==null&&historyAge.value>=0&&historyAge.value<=7);
let timer:ReturnType<typeof setInterval>|undefined;let request=0;
async function load(reset=false){const req=++request;try{const result=await invoke<Dashboard>('research_dashboard');if(req!==request)return;dashboard.value=result;if(reset){Object.assign(settings,result.config);historyUrl.value=appSettings.localHistoryUrl;archiveOpen.value=false;}if(!selectedId.value&&result.experiments.length)selectedId.value=result.experiments[0].id;await loadModels(reset);if(reset)tab.value=centerMode.value==='simple'?'tracking':(modelRuns.value.length?'model-lab':'guide');}catch(e){if(req===request)error.value=String(e);}}
async function refreshHistory(){historyStatus.value=await invoke<HistoryStatus>('get_local_history_status');}
async function refreshSetup(){try{await appSettings.fetchStockDbStatus();if(!appSettings.localHistoryEnginePath)await appSettings.scanStockDb();if(appSettings.localHistoryEnabled)await refreshHistory();else historyStatus.value=null;}catch(e){error.value=String(e);}}
watch(()=>props.show,open=>{if(timer)clearInterval(timer);if(open){void load(true);void refreshSetup();timer=setInterval(()=>void load(),5000);}}, {immediate:true});

onBeforeUnmount(()=>{++request;if(timer)clearInterval(timer);});
async function act(key:string,fn:()=>Promise<void>){if(busy.value)return;busy.value=key;error.value='';notice.value='';try{await fn();await load();}catch(e){error.value=String(e);await load();}finally{busy.value='';}}

function action(action:string){const id=selected.value?.id;if(!id)return;void act(action,async()=>{await invoke('research_action',{experimentId:id,action});});}
function save(){void act('save',async()=>{await invoke('save_research_config',{config:{...settings}});await load();notice.value='已保存自动研究开关。信号来源账本的盘后更新在各账本中单独开启。';});}
function scanHistory(){void act('scan-history',async()=>{await appSettings.scanStockDb();notice.value='请选择找到的 stockdb.exe，或手动浏览。';});}
function chooseHistory(path:string){void act('choose-history',async()=>{await appSettings.selectStockDbEngine(path);historyStatus.value=null;});}
function browseHistory(){void act('browse-history',async()=>{const path=await open({multiple:false,directory:false,title:'选择 stockdb.exe',filters:[{name:'stockdb',extensions:['exe']}]});if(typeof path==='string'){await appSettings.selectStockDbEngine(path);historyStatus.value=null;}});}
function browseUpdater(){void act('browse-updater',async()=>{const path=await open({multiple:false,directory:false,title:'选择 数据更新.exe',filters:[{name:'数据更新程序',extensions:['exe']}]});if(typeof path==='string')await appSettings.selectStockDbUpdater(path);});}
function enableHistory(){void act('enable-history',async()=>{if(!await appSettings.setLocalHistoryEnabled(true))throw new Error(appSettings.error||'本地历史启动失败');await refreshHistory();});}
function testHistory(){void act('test-history',async()=>{await refreshHistory();});}
function saveHistoryUrl(){void act('history-url',async()=>{if(!await appSettings.setSetting('local_history_url',historyUrl.value.trim()))throw new Error(appSettings.error||'历史服务地址保存失败');historyStatus.value=null;await refreshHistory();});}
function updateHistory(){void act('update-history',async()=>{notice.value=await appSettings.runStockDbUpdate();await refreshHistory();});}
const chart=computed(()=>{const rows=selected.value?.curve??[];const values=[0,...rows.map(p=>p.return_bps)];const low=Math.min(0,...values),high=Math.max(100,...values),range=high-low||1;const points=values.map((v,i)=>`${40+i/(Math.max(values.length-1,1))*650},${150-(v-low)/range*115}`).join(' ');return{points,low,high,zero:150-(0-low)/range*115};});
</script>

<template>
<NModal :show="show" @update:show="emit('update:show',$event)" preset="card" class="research-modal" :bordered="false"
  :style="{width:'min(1240px, calc(100vw - 24px))',height:'min(860px, calc(100dvh - 32px))',display:'flex',flexDirection:'column',overflow:'hidden'}"
  :content-style="{padding:0,minHeight:0,display:'flex',flexDirection:'column'}">
  <template #header><div class="research-title"><span class="research-mark" aria-hidden="true"><svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M7 2h6M8 2v6l-5 8a1 1 0 0 0 1 2h12a1 1 0 0 0 1-2l-5-8V2M6 13h8" /></svg></span><div><h2>研究中心</h2><p>{{centerMode==='simple'?'自动模型交易 · 手动提醒独立':'数据准备 · 冻结模型回放 · 研究证据'}}</p></div></div></template>
  <template #header-extra><span v-if="centerMode==='advanced'" class="header-meta">{{modelRuns.length}} 个研究账本</span></template>
  <div class="research-center">
    <div class="research-navigation">
      <div class="center-mode" role="group" aria-label="研究中心使用模式"><button :class="{active:centerMode==='simple'}" :aria-pressed="centerMode==='simple'" @click="centerMode='simple'">简化版 · 日常使用</button><button :class="{active:centerMode==='advanced'}" :aria-pressed="centerMode==='advanced'" @click="centerMode='advanced'">深入研究</button><HelpTooltip label="两种使用方式">日常使用分为自动模型交易和手动条件提醒。自动账户只供模型买卖，手动提醒不使用交易账户；深入研究用于核对数据、冻结模型回放和研究证据；研究提醒、任务和实验资料统一在“提醒与记录”查看。切换方式会记住，不自动启动计算。</HelpTooltip></div>
      <nav class="research-nav" aria-label="研究中心导航"><button v-for="item in primaryTabs" :key="item.id" :class="{active:tab===item.id}" :aria-current="tab===item.id?'page':undefined" @click="tab=item.id">{{item.label}}</button></nav>
    </div>
    <div ref="scrollEl" class="research-scroll" tabindex="0" aria-label="研究内容">
    <p v-if="centerMode==='advanced'&&tab!=='records'" class="research-orientation">第一次使用按“数据准备 → 冻结模型回放 → 查看研究证据”的顺序；已有账本可直接查看。日常自动买卖在“简化版”管理，这里按你点击的功能开展研究。</p>
    <div v-if="centerMode==='advanced'" class="health"><span :class="{good:historyReady}">{{historyReady?'● 历史数据已验证':dashboard?.local_ready?'○ 历史已启用 · 待检查':'○ 历史尚未连接'}}</span><span>{{modelRuns.filter(v=>v.mode==='forward'&&v.enabled).length}} 个信号来源账本</span></div>
    <p v-if="error" class="banner error" role="alert">{{error}}</p><p v-if="notice" class="banner success">{{notice}}</p>
    <p v-if="busy" class="banner" role="status">{{busy==='update-history'?'正在后台更新并校验数据，完成后自动恢复服务…':'正在处理…'}}</p>
    <section v-if="tab==='records'" class="research-records-intro"><h3>提醒与记录</h3><p>统一管理研究消息、查看后台任务；深入研究的实验资料也在本页。切换页面会保留任务进度。</p></section>
    <button class="minor-btn" type="button" @click="openSetting('notifications')">提醒设置 · 资讯与提醒</button>
    <ModelFollowTrading v-if="tab==='tracking'" ref="followPanel" :source-runs="modelRuns" :active="show" @refresh-sources="loadModels()" />
    <DailyResearch v-if="tab==='daily'" ref="dailyPanel" page="daily" :busy="jobsBusy" @job="startedJob" @records="openRecords" @evidence="openEvidence" @open-settings="openSetting" />
    <ResearchEvidence v-if="tab==='evidence'" />
    <details v-if="show" v-show="tab==='records'" class="research-background-jobs" :open="jobsExpanded" @toggle="jobsExpanded=($event.target as HTMLDetailsElement).open"><summary>研究任务与已核验结果{{jobsBusy ? ' · 正在运行' : ''}}</summary><ResearchJobPanel ref="jobPanel" :selected-id="activeJobId" @complete="completedJob" @updated="jobsUpdated" @select-run="openModelRun" /></details>
    <section v-if="tab==='model-lab'" class="reading">
      <h3>冻结模型历史回放</h3>
      <p class="section-purpose">检查固定模型扣费后的收益、回撤与买卖理由；评估模型或核对历史账本时使用，只生成研究账本，不启动用户模型交易。</p>
      <div class="research-howto" aria-label="冻结模型回放操作步骤">
        <b>怎么操作</b>
        <ol><li>先到“数据准备与更新”核对历史连接和研究路径。</li><li>点下方一个已有模型。第一次保留默认执行对照，再点“运行真实多年回放”；选模型本身不会开始计算。</li><li>任务开始后会切到“提醒与记录”看进度。完成后点账本，或点任务中的“查看账本”，核对收益、回撤和每笔买卖原因。</li></ol>
        <p><b>结果在哪里：</b>下方“信号来源与历史回放账本”。已有账本直接点开即可，无需重新跑。</p>
      </div>
      <div class="mode-grid"><button v-for="m in modelOptions" :key="m.id" class="mode-card" :class="{selected:modelId===m.id}" :disabled="!!busy" @click="modelId=m.id"><b>{{m.name}}</b><p>{{m.note}}</p><small>{{m.id}}</small></button></div>
      <details><summary>期限、费用与前向来源对照（按需）</summary>
      <p class="section-purpose">用同一个冻结模型检查执行条件的影响。默认保留原基准；这里的修改只用于下一次研究，不改变自动账户配置。</p>
      <dl class="research-control-help"><dt>原20日期限＋3ATR失效</dt><dd>按原持有上限与波动失效规则回放；触发失效可以提前卖出，20日不是必须持满。</dd><dt>同模型15日期限</dt><dd>把持有上限改为15个交易日作对照，模型仍使用原20日预测标签，不重新训练。</dd><dt>原20日双成本</dt><dd>把原研究费用翻倍，检查费用更高时结果是否仍成立。</dd><dt>分层执行反证</dt><dd>同时改变入场、减仓和退出方式，查证这种改法是否有效；该项已有失败结果，按需复核。</dd><dt>已核验权益日期</dt><dd>使用已核验的送转上市日与股息到账日修正记账，核对权益挂起问题。</dd></dl>
      <div class="toolbar"><label>冻结执行对照 <select v-model="modelComparison" :disabled="!!busy"><option value="baseline">原20日期限＋3ATR失效</option><option value="holding15">同模型15日期限对照（非15日重训）</option><option value="cost_double">原20日双成本</option><option value="staged">分层执行反证（本轮广度22失败）</option><option value="verified_actions">20日＋已核验权益日期（记账修正）</option></select></label></div>
      <p v-if="modelComparison==='verified_actions'" class="note">使用七份发行人实施公告核验的送转上市与现金支付日期，修正已知权益记账；不是新选股算法。旧结果保留，其他尚未核验的权益仍挂起。研究程序版本升级后，旧账户保留原身份，需要开启新版本账户，不能悄悄改变旧账本。</p>
      <p v-if="modelComparison==='staged'" class="banner error">综合分层对照采用40/30/30入场、2ATR/均线结构失效、弱势减仓与盈利保护，同时改变了退出机制。广度22连续多年实测−62.669%、回撤64.041%；不能把差异归因仅加仓，也不作为默认操作。</p>
      <p class="section-purpose">检查期限、交易成本和新完成日信号是否改变结果；需要反证或持续核对信号时使用，不启用用户自动交易。</p>
      <div class="actions"><button :disabled="!!busy||jobsBusy||dashboard?.busy" @click="runModel('forward')">建立前向信号来源账本</button><button :disabled="!!busy" @click="importModelLedger">导入可信模型账本</button><button :disabled="!!busy||jobsBusy" @click="compareModel">比较本模型：基准 / 15日 / 双费</button></div>
      <dl class="research-control-help"><dt>建立前向信号来源账本</dt><dd>选好模型和对照后点击，为后续新增完成日建立研究记录。完成后选中该账本，再“开启盘后更新”；没有新行情时显示等待。</dd><dt>导入可信模型账本</dt><dd>选择该研究程序生成的模型账本JSON；核验通过后在下方查看。普通表格、任意JSON或模拟账户文件不能代替模型账本。</dd><dt>比较本模型：基准 / 15日 / 双费</dt><dd>选一个模型后点击，后台分别保存三个对照账本。到任务区逐个“查看账本”，比较同一数据截止日的收益、回撤和成交；此按钮固定比较这三项。</dd></dl>
      <p class="note">来源账本核对新的完成交易日，开启盘后更新后需保持应用运行；只推进研究信号和研究账本。来源估算股数、收益与提醒不等于用户交易账户成交。</p>
      </details>
      <div class="actions"><button class="primary" :disabled="!!busy||jobsBusy||dashboard?.busy" @click="runModel('replay')">运行真实多年回放</button><button @click="tab='guide'">检查数据与研究路径</button></div>
      <p class="note">正分不等于胜率或买入许可。前向账户不回填开启之前的买卖；没有新完成行情时保留10万元现金、零周期等待。现金参考股数包含估算费用；基准目标8%，分层首仓3.2%，不是尚未知下一开盘的订单。股票涨跌停、停牌、权益日期未知会阻断或挂起；执行期限可能因不能卖出而超期。</p>
      <p v-if="modelError" class="banner error">最近研究错误：{{modelError}}</p>
      <h4>信号来源与历史回放账本</h4><p class="section-purpose">点一个账本切换结果。累计净收益是整个研究区间扣费后的盈亏；最大回撤是净值从阶段高点跌落的最大幅度；完整持仓周期只计算买入后已卖完的记录。研究结果与用户自动账户收益分别查看。</p><div class="targets"><button v-for="v in modelRuns" :key="v.id" :class="{selected:modelSelectedId===v.id}" @click="modelSelectedId=v.id">#{{v.id}} {{v.model_name}} · {{v.mode==='forward'?'前向观察':'历史回放'}} · {{v.holding_days}}日 · {{v.as_of}}</button></div>
      <article v-if="modelSelected" class="detail"><h3>{{modelSelected.model_name}} · {{modelSelected.state==='waiting_new_data'?'等待新增完成行情':modelSelected.mode==='forward'?'纸上观察':'历史回放'}}</h3><p>{{modelSelected.score_semantic}}</p><p>数据截至 {{modelSelected.as_of}} · {{modelSelected.comparison}} · 标签20日 / 实际期限{{modelSelected.holding_days}}日</p>
        <div class="metrics"><div><small>研究累计净收益</small><b>{{modelSelected.ledger.metrics.net_return_pct.toFixed(3)}}%</b></div><div><small>组合最大回撤</small><b>{{modelSelected.ledger.metrics.max_drawdown_pct.toFixed(3)}}%</b></div><div><small>完整持仓周期 / 胜率</small><b>{{modelSelected.ledger.metrics.completed_holding_cycles}} / {{modelSelected.ledger.metrics.win_rate_pct==null?'—':modelSelected.ledger.metrics.win_rate_pct+'%'}}</b></div><div><small>现金 / 未平股票</small><b>¥{{modelSelected.ledger.curve.at(-1)?.cash.toLocaleString('zh-CN')}} / {{modelSelected.ledger.metrics.open_positions}}</b></div></div>
        <p class="banner error">权益挂起尾部：{{lockedTail.stocks}}只股票、{{lockedTail.shares}}股新增股份仍锁定，按末报价估值¥{{lockedTail.lockedMarketValue.toFixed(2)}}；其余可卖股份{{lockedTail.sellableShares}}股。最长超期{{modelSelected.ledger.metrics.max_overdue_sessions??0}}市场交易日，包含未知上市日、停牌等不可卖尾部，不能称正常20日完成。应收股息¥{{modelSelected.ledger.curve.at(-1)?.dividend_receivable.toFixed(2)}}未当可用现金。</p>
        <svg viewBox="0 0 700 185" role="img" aria-label="模型真实现金账本净值"><polyline :points="modelChart" fill="none" stroke="var(--color-accent)" stroke-width="2"/></svg>
        <div v-if="modelSelected.mode==='forward'" class="actions"><button :disabled="!!busy||jobsBusy" @click="runModel('forward',modelSelected)">按新快照延续来源账本</button><button :disabled="!!busy" @click="observeModel(modelSelected)">{{modelSelected.enabled?'暂停盘后更新':'开启盘后更新'}}</button></div>
        <p v-if="modelSelected.mode==='forward'" class="section-purpose"><b>来源账本怎么继续：</b>“按新快照延续来源账本”手动推进所选账本；“开启盘后更新”让它在应用运行时自动检查新完成日，想停就点“暂停盘后更新”。只延续这一研究账本，不暂停用户自动账户，也不会自动买入观察名单。</p>
        <p class="note">程序在16:00后核对新完成日，数据齐备才推进信号来源；近期缺口保留行情源兜底。缺数据时后台等待并保留原账本，状态中显示原因。补历史只记为回放，来源记录不等于用户账户成交。关闭AI不影响确定性模型计算。</p>
        <h4>最新完成日模型观察 <HelpTooltip label="模型观察说明">固定模型阈值筛出的观察名单，分数不是胜率。日线核验使用前向信号来源账本的盘后更新，数据齐备才检查。用户自动买卖与成交请在自动交易账户查看。</HelpTooltip> · {{modelSelected.signal_watch.length}}只</h4><p v-if="!modelSelected.signal_watch.length">固定阈值下无候选；保持空列表，不降阈值凑股票。</p><div class="table-wrap"><table><thead><tr><th>股票</th><th>模型原始预测</th><th>固定阈值</th><th>现金/收盘参考股数</th><th>边界</th></tr></thead><tbody><tr v-for="v in modelSelected.signal_watch" :key="v.symbol"><td>{{v.symbol}}</td><td>{{v.score.toFixed(8)}}</td><td>{{v.threshold}}</td><td>{{v.cash_reference_quantity}} · {{v.cash_reference_stage}}</td><td>{{v.reason}}</td></tr></tbody></table></div>
        <h4>最近实际回放成交</h4><div class="table-wrap"><table><thead><tr><th>日期</th><th>股票</th><th>方向</th><th>股数</th><th>价格 / 费用</th><th>执行原因</th></tr></thead><tbody><tr v-for="(o,i) in modelSelected.ledger.orders.slice(-30).reverse()" :key="i"><td>{{o.date}}</td><td>{{o.code}}</td><td>{{o.side==='buy'?'买入':'卖出'}}</td><td>{{o.qty}}</td><td>{{o.price}} / {{o.fee}}</td><td>{{o.reason}}</td></tr></tbody></table></div>
        <details><summary>完整数据指纹、未平持仓及限制</summary><p class="section-purpose">需要追查结果来源时展开：指纹用于辨认模型与输入版本，未平持仓是研究截止时还未卖完的股票；限制说明缺失权益日期或不可成交等问题。存在未平或超期尾部时，不能只凭累计收益判断正常持有周期。</p><pre>{{JSON.stringify({model:modelSelected.model_sha256,data:modelSelected.data_sha256,bundle:modelSelected.content_sha256,unclosed:modelSelected.ledger.unclosed,limitations:modelSelected.limitations},null,2)}}</pre></details>
      </article><p v-else>点击运行后这里显示实际账本。没有执行时不生成虚构净值或胜率。</p>
    </section>
    <section v-else-if="tab==='guide'" class="reading data-setup">
        <h3>数据准备与更新</h3>
        <p class="section-purpose">检查日线是否可读、日期是否够新及研究文件是否齐全；首次运行或缺数据时使用，数据更新本身不下单，已开启的自动模型仍按原计划独立运行。</p>
        <div class="research-howto" aria-label="数据准备与更新操作步骤"><b>怎么操作</b><ol><li>未配置服务时，点“自动查找”后选择找到的程序，或“浏览 stockdb.exe”；再点“启用并启动”。已连接时直接“测试历史连接”。</li><li>数据日期不足时，先“选择 数据更新.exe”，再点“更新历史数据”；更新会自动暂停并恢复已确认的本地服务。</li><li>首次回放展开“本机研究环境与数据路径”，核对并保存四项路径；完成后点“数据就绪，查看冻结模型回放”。</li></ol><p><b>结果在哪里：</b>下方连接状态与样本截止日期；样本检查通过后，运行研究还会核验完整模型输入。</p></div>
        <div class="wizard-result" :class="{verified:historyReady}" role="status"><b>{{historyReady?'历史连接与日期已验证':'历史数据尚未通过检查'}}</b><span>{{appSettings.stockDbStatus?.message||'正在检测本地服务…'}}</span><span v-if="historyStatus">{{historyStatus.message}}<template v-if="historyStatus.end_date"> · 600519 样本截至 {{historyStatus.end_date}}，共 {{historyStatus.sample_count}} 根</template></span><span v-if="historyAge!==null&&historyAge>7">数据距今 {{historyAge}} 天；研究启动要求原始日线足够新，请先更新。</span><span v-if="appSettings.stockDbStatus?.state==='running_external'">当前服务由外部启动；更新时自动暂停已确认的本地 StockDB，完成后按原启动配置恢复。</span></div>
        <p class="path">已选程序：{{displayPath(appSettings.localHistoryEnginePath)||'尚未选择'}}</p>
        <div class="actions"><button :disabled="!!busy" @click="scanHistory">自动查找</button><button :disabled="!!busy" @click="browseHistory">浏览 stockdb.exe</button><button v-if="!appSettings.localHistoryEnabled" class="primary" :disabled="!!busy||!appSettings.localHistoryEnginePath" @click="enableHistory">启用并启动</button><template v-else><button :disabled="!!busy" @click="testHistory">测试历史连接</button><button v-if="!historyReady||appSettings.stockDbStatus?.state==='running_external'" :disabled="!!busy||!appSettings.localHistoryEnginePath" @click="enableHistory">重新启动 / 检测服务</button></template></div>
        <div v-if="appSettings.stockDbStatus?.candidates.length" class="wizard-candidates"><button v-for="candidate in appSettings.stockDbStatus.candidates" :key="candidate.enginePath" :disabled="!!busy" :title="displayPath(candidate.enginePath)" @click="chooseHistory(candidate.enginePath)">{{candidate.source}} · {{displayPath(candidate.enginePath)}}</button></div>
        <div v-if="appSettings.localHistoryEnabled" class="wizard-update"><p>数据更新程序：{{displayPath(appSettings.localHistoryUpdaterPath)||'尚未选择'}}</p><button :disabled="!!busy" @click="browseUpdater">选择 数据更新.exe</button><button :disabled="!!busy||appSettings.stockDbStatus?.busy||!appSettings.stockDbStatus?.updaterAvailable" @click="updateHistory">更新历史数据</button></div>
        <details><summary>连接非默认地址的本地 stockdb</summary><p class="section-purpose">仅在服务端口或地址变动时使用：填写实际服务地址，点“保存并测试”，看上方连接结果。普通本机使用保留默认地址即可。</p><label class="wizard-url">服务地址 <input v-model="historyUrl" type="url" placeholder="http://127.0.0.1:7899"><button :disabled="!!busy" @click="saveHistoryUrl">保存并测试</button></label></details>

      <details class="research-paths"><summary>本机研究环境与数据路径</summary><p class="section-purpose">核对冻结快照、指数与可信程序的来源；首次回放或路径变动时使用，只保存计算环境，不发出交易指令。</p><p>请选择完整研究项目目录（包含模型缓存与快照）。Python需要numpy和该模型已有的scikit-learn版本；程序不自动安装依赖或调用Claude替代计算。其他电脑可手动选自己的Python，无需Codex缓存路径。</p><dl class="research-control-help"><dt>项目目录</dt><dd>用“浏览”选择包含 research/research-center-runner、冻结模型及研究文件的项目根目录，不选安装程序所在文件夹。</dd><dt>Python可执行文件</dt><dd>选择已有研究环境的 python.exe，需要原模型依赖；已能正常回放时保留现有值。</dd><dt>兼容快照目录</dt><dd>选择原模型使用的行情快照目录；这是回放数据，不是某一只股票的文件。</dd><dt>显式CSI300 NDJSON</dt><dd>选择原研究输入中的沪深300指数文件，用于核对交易日及大盘条件；不能用股票日线替代。</dd></dl><p class="section-purpose">确认后点“保存路径”。保存只登记位置，随后点击回放时才验证文件和模型版本；不知道路径时先保留现有配置，根据明确的缺失提示核对。</p><div class="config-grid"><label v-for="(_,key) in modelConfig" :key="key">{{({research_root:'项目目录',python:'Python可执行文件',snapshot:'兼容快照目录',index:'显式CSI300 NDJSON'} as Record<string,string>)[key]}}<input :value="displayPath(modelConfig[key])" :title="displayPath(modelConfig[key])" :disabled="!!busy" @input="modelConfig[key]=($event.target as HTMLInputElement).value"/><button :disabled="!!busy" @click="chooseModelPath(key,key==='research_root'||key==='snapshot')">浏览</button></label></div><button :disabled="!!busy" @click="saveModelPaths">保存路径</button><p>完成日优先读取StockDB，近期单日缺口由行情源兜底；程序按冻结股票轴与指数日历构建兼容输入。旧历史/股票轴改变、缺指数日期、SHA漂移或进入2027未审核年度均停止；不会把9/30评分重标成今日。</p></details>
      <button @click="tab='model-lab'">数据就绪，查看冻结模型回放</button>
    </section>
    <details v-if="centerMode==='advanced'" v-show="tab==='records'" class="research-archive" :open="archiveOpen" @toggle="archiveOpen=($event.target as HTMLDetailsElement).open">
      <summary>研究资料与实验记录（可选）</summary>
      <template v-if="archiveOpen">
        <p class="section-purpose">检查新方法未被采用的原因和已有任务依据；追溯实验时再展开，查看资料不会启用模型交易。</p>
        <details :open="nestedOpen" @toggle="nestedOpen=($event.target as HTMLDetailsElement).open"><summary>技术组合实验记录</summary><p class="section-purpose">检查组合选择能否跨年份、成本与延迟保持稳定；研究替代方法时使用，计算结果不自动加入选股模型。</p><NestedResearch v-if="nestedOpen" ref="nestedPanel" :busy="jobsBusy" @job="startedJob" /></details>
        <details :open="extensionsOpen" @toggle="extensionsOpen=($event.target as HTMLDetailsElement).open"><summary>扩展研究报告</summary><ResearchExtensions v-if="extensionsOpen" /></details>
    <details v-if="counts.all" class="legacy-records"><summary>旧模拟账本（{{counts.all}} 条记录）</summary>
      <p class="section-purpose">核对早期策略的账户、成交与失败原因；追查旧结果时使用，查看记录不交易，暂停只停止原实验继续运行。</p><div class="research-howto" aria-label="旧模拟账本操作步骤"><b>怎么操作</b><ol><li>点左侧一条旧实验，右侧显示对应账户、状态和冻结股票池。</li><li>先看“最近模拟指令”的状态与原因，再按需展开成交证据、冻结条件，核对资金、回撤和持仓。</li><li>想停止仍在运行的旧实验，点“暂停原实验”；账户、持仓和历史记录保留，其他模型账户继续独立运行。</li></ol><p><b>结果在哪里：</b>当前旧实验右侧的曲线与记录；没有旧记录时整个入口不显示。</p></div>


      <div class="workspace">
        <aside><button v-for="item in dashboard?.experiments" :key="item.id" class="experiment" :class="{selected:item.id===selectedId}" @click="selectedId=item.id"><span class="badge" :data-state="item.state">{{states[item.state]||item.state}}</span><strong>{{item.name}}</strong><small>{{rules[item.rule]}} · 版本 #{{item.version_id}}</small><span>{{pct(item.detail?.metrics.total_return_bps)}}<small>收益</small>　{{pct(item.detail?.metrics.max_drawdown_bps)}}<small>回撤</small></span></button></aside>
        <article v-if="selected" class="detail">
          <div class="detail-title"><div><span class="eyebrow">EXPERIMENT {{selected.id}} / VERSION {{selected.version_id}}</span><h3>{{selected.name}}</h3></div><span class="badge" :data-state="selected.state">{{states[selected.state]}}</span></div>
          <p class="hypothesis">{{selected.hypothesis}}</p>
          <div class="limits"><span>观察 {{selected.config.observation_days}} 天</span><span>至少 {{selected.config.min_samples}} 笔卖出</span><span>最大回撤 {{pct(selected.config.max_drawdown_bps)}}</span><span>收益超过 {{pct(selected.config.min_return_bps)}} 且跑赢基准</span></div>
           <p class="next-step">{{selected.last_message||'旧实验记录仅供复核。'}}</p>
           <div v-if="['observing','extended','adopted'].includes(selected.state)" class="actions"><button :disabled="!!busy" @click="action('pause')">暂停原实验</button></div>
          <template v-if="selected.detail">
            <p class="next-step" v-if="live">{{live.message}}<small> · {{live.engine==='realtime_a_share'?'A 股实时前向':'日线前向对照'}} · 已观测 {{live.marks}} 个行情分钟</small></p>
            <div v-if="live?.plans.length" class="table-wrap"><table><thead><tr><th>实时买卖计划</th><th>买入区间</th><th>止损触发</th><th>止盈卖点</th></tr></thead><tbody><tr v-for="p in live.plans" :key="p.symbol"><td>{{p.symbol}}</td><td>{{(p.buy_low/10000).toFixed(2)}}–{{(p.buy_high/10000).toFixed(2)}}</td><td>{{(p.stop/10000).toFixed(2)}}</td><td>{{(p.take/10000).toFixed(2)}}</td></tr></tbody></table></div>
            <details v-if="live?.executions.length"><summary>查看实时成交证据：时间、对手盘、数量与费用</summary><pre>{{JSON.stringify(live.executions,null,2)}}</pre></details>
            <div v-if="daily" class="table-wrap"><h4>并行对照 · 资金和订单完全独立，主考核依据实时账户</h4><table><thead><tr><th>执行口径</th><th>收益</th><th>最大回撤</th><th>完成卖出</th></tr></thead><tbody><tr><td>实时 · 新盘口成交</td><td>{{pct(selected.detail.metrics.total_return_bps)}}</td><td>{{pct(selected.detail.metrics.max_drawdown_bps)}}</td><td>{{selected.detail.metrics.sample_count}}</td></tr><tr><td>日线 · 次日开盘假设</td><td>{{pct(daily.metrics.total_return_bps)}}</td><td>{{pct(daily.metrics.max_drawdown_bps)}}</td><td>{{daily.metrics.sample_count}}</td></tr></tbody></table><p class="note">这是两种执行方式的结果对照。买卖时机与触发规则可能不同，差额不能全部归因于滑点。</p></div>
            <div class="metrics"><div><small>账户权益</small><b>¥{{money(selected.detail.metrics.equity)}}</b></div><div><small>累计收益</small><b>{{pct(selected.detail.metrics.total_return_bps)}}</b></div><div><small>最大回撤</small><b>{{pct(selected.detail.metrics.max_drawdown_bps)}}</b></div><div><small>同期基准</small><b>{{pct(selected.detail.metrics.benchmark_return_bps)}}</b></div></div>
            <div class="progress-row"><div><label>时间 · 已记录 {{selected.elapsed_days}} 天 / 还需 {{selected.remaining_days}} 天</label><progress :value="selected.elapsed_days" :max="selected.config.observation_days" /></div><div><label>样本 · {{selected.detail.metrics.sample_count}} 笔 / 还需 {{selected.remaining_samples}} 笔</label><progress :value="selected.detail.metrics.sample_count" :max="selected.config.min_samples" /></div></div>
            <div class="chart"><h4>账户收益曲线 <small>日终概览 · 实时账户回撤按已观测盘中净值计算</small></h4><svg v-if="selected.curve.length" viewBox="0 0 720 185" role="img" aria-label="账户累计收益曲线"><line x1="40" :y1="chart.zero" x2="690" :y2="chart.zero" stroke="currentColor" opacity=".2" stroke-dasharray="4 4"/><polyline :points="chart.points" fill="none" stroke="var(--color-accent)" stroke-width="2.5"/><text x="3" y="25" fill="currentColor" font-size="12">{{pct(chart.high)}}</text><text x="3" y="155" fill="currentColor" font-size="12">{{pct(chart.low)}}</text><text x="40" y="180" fill="currentColor" font-size="12">初始资金</text><text x="590" y="180" fill="currentColor" font-size="12">{{selected.curve.at(-1)?.date}}</text></svg><p v-else>等待首次有效净值记录。没有数据时不绘制虚假曲线。</p></div>
            <h4>冻结股票池 · 模拟账户 #{{selected.account_id}}</h4><div class="targets"><span v-for="stock in selected.detail.targets" :key="stock.symbol">{{stock.name}} <small>{{stock.symbol}}</small></span></div>
            <h4>最近模拟指令</h4><div class="table-wrap"><table><thead><tr><th>信号日</th><th>标的</th><th>方向</th><th>股数</th><th>状态 / 原因</th></tr></thead><tbody><tr v-for="order in selected.detail.orders.slice(0,20)" :key="order.id"><td>{{order.signal_date}}</td><td>{{order.symbol}}</td><td>{{order.side==='buy'?'买入':'卖出'}}</td><td>{{order.quantity}}</td><td>{{order.status}} {{order.decision_reason || order.reject_reason}}</td></tr><tr v-if="!selected.detail.orders.length"><td colspan="5">尚未触发规则；自动运行不代表必须交易。</td></tr></tbody></table></div>
          </template>
          <details><summary>查看冻结条件和选股依据</summary><pre>{{JSON.stringify({rule:selected.rule,filter:selected.filter,selection:JSON.parse(selected.selection_json)},null,2)}}</pre></details>
        </article>
      </div>
    </details>
        <details v-if="modelTasks.length"><summary>已登记的研究任务书（{{modelTasks.length}} 条）</summary><p class="section-purpose">核对假说、反证和缺失数据；追查历史任务依据时使用，选配置后仍需手动运行，不产生交易。</p><p class="section-purpose"><b>怎么操作：</b>展开一条任务书，读假说、反证和缺失项；需要复核时点“选择该任务的可信配置”，页面会切到回放并选好模型和执行对照。检查后再点“运行真实多年回放”，结果在研究任务和账本中查看。选择配置本身不开始计算。</p><details v-for="t in modelTasks" :key="t.id"><summary>#{{t.id}} {{t.name}} · {{t.model_id}} · {{t.comparison}}</summary><p>{{t.hypothesis}}</p><p>反证：{{t.invalidation.join('；')}}</p><p>缺失：{{t.missing_data.join('；')||'仍受共同数据代理边界限制'}}</p><button :disabled="!!busy" @click="modelId=t.model_id;modelComparison=t.comparison;tab='model-lab'">选择该任务的可信配置</button></details></details>
        <details><summary>盘后 AI 研究设置（按需）</summary><p class="section-purpose">用途是继续验证已有模型的期限与成本等对照，补充研究账本。日常自动模型买卖用不到它，默认关闭，想持续做研究时再启用。</p><div class="research-howto" aria-label="盘后AI研究操作步骤"><b>怎么操作</b><ol><li>先点“配置 AI 连接”，在设置中配置已选提供方并测试；研究数据与路径也须准备好。</li><li>需要自动研究时勾选“启用盘后 AI 研究与历史对照”，再点“保存自动研究设置”；保持应用运行，交易日16:00–22:00按规则检查。</li><li>看下方最近研究消息、已登记任务书及新增研究账本；想停止自动研究，取消勾选并再次保存。</li></ol><p><b>实际流程：</b>先检查已有任务书，补做尚未完成的历史对照；没有待验证任务时才请AI提出新的研究假说，再用已有冻结模型核验。结果在本页任务书和回放账本中查看，不会直接调整你的交易模型。<br/><b>费用：</b>提出新假说时可能产生AI费用；每个交易日最多尝试一次，同配置完成后不重跑，失败留原因，当日不循环调用。</p></div><fieldset :disabled="!!busy" class="config-fields"><label class="auto-toggle"><input v-model="settings.auto_research" type="checkbox"/><span><b>启用盘后 AI 研究与历史对照</b><small>应用运行时，交易日16:00后、22:00前最多研究一次；不会自动启用新模型或建立交易账户。</small></span></label></fieldset><div class="actions"><button :disabled="!!busy" @click="save">保存自动研究设置</button><button @click="emit('open-settings','ai')">配置 AI 连接</button></div><p v-if="dashboard?.last_auto_message" class="last-auto">{{dashboard.last_auto_message}}</p></details>
      </template>
    </details>

    </div>
  </div>
</NModal>
</template>

<style scoped>
.research-records-intro{margin:0 0 16px;line-height:1.7}.research-records-intro h3{margin:0 0 4px;font-size:16px}.research-records-intro p{margin:0;color:var(--color-text-secondary);font-size:13px}
.research-orientation{margin:0 0 12px;color:var(--color-text-secondary);font-size:13px;line-height:1.7}.research-howto{margin:12px 0;padding:12px 14px;border:1px solid var(--color-border-0);border-radius:10px;background:var(--color-surface-2);font-size:13px;line-height:1.8}.research-howto ol{margin:6px 0;padding-left:22px}.research-howto li+li{margin-top:4px}.research-howto p{margin:6px 0 0}.research-control-help{display:grid;grid-template-columns:minmax(100px,170px) minmax(0,1fr);gap:8px 14px;margin:12px 0;font-size:13px;line-height:1.75}.research-control-help dt{font-weight:600}.research-control-help dd{margin:0;color:var(--color-text-secondary);overflow-wrap:anywhere}@media(max-width:600px){.research-control-help{grid-template-columns:1fr;gap:4px}.research-control-help dd{margin-bottom:6px}}
.research-background-jobs{margin:10px 0 14px;}
.center-mode{display:flex;align-items:center;gap:8px;flex-wrap:wrap;padding:8px 0}.center-mode button{border:1px solid var(--color-border-0);border-radius:8px;padding:7px 12px;background:var(--color-surface-1);color:var(--color-text-secondary);cursor:pointer}.center-mode button.active{background:var(--color-accent-dim);color:var(--color-accent);border-color:var(--color-accent);font-weight:600}

:global(.research-modal.n-card) { border-radius: 16px; border: 1px solid var(--color-border-0); box-shadow: var(--shadow-md); }
:global(.research-modal.n-card > .n-card-header) { padding: 16px 20px 12px; flex-shrink: 0; }
:global(.research-modal.n-card > .n-card__content) { flex: 1; }
.research-title { display: flex; align-items: center; gap: 10px; min-width: 0; }
.research-title h2 { margin: 0; font-size: 18px; line-height: 1.4; }
.research-title p { margin: 2px 0 0; font-size: 12px; font-weight: 400; color: var(--color-text-tertiary); }
.research-mark { display: grid; place-items: center; width: 36px; height: 36px; flex-shrink: 0; border-radius: 10px; color: var(--color-accent); background: var(--color-accent-dim); }
.research-mark svg { width: 20px; height: 20px; }
.header-meta { color: var(--color-text-tertiary); font-size: 12px; font-weight: 400; white-space: nowrap; }
.research-center { font-family: "Microsoft YaHei UI", "Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", var(--font-sans); font-size: 14px; display: flex; flex-direction: column; flex: 1; min-height: 0; min-width: 0; color: var(--color-text-primary); line-height: 1.65; }
.research-navigation { flex-shrink: 0; padding: 0 20px; border-bottom: 1px solid var(--color-border-0); background: var(--color-surface-1); }
.research-nav { display: flex; gap: 6px; overflow-x: auto; scrollbar-width: thin; }
.research-nav button { flex-shrink: 0; white-space: nowrap; border: 0; border-radius: 0; background: transparent; padding: 12px 14px; }
.research-nav .active { color: var(--color-accent); box-shadow: inset 0 -2px var(--color-accent); font-weight: 600; }
.research-scroll { flex: 1; min-height: 0; min-width: 0; padding: 16px 22px 20px; overflow: auto; overscroll-behavior: contain; scrollbar-gutter: stable; }
.research-scroll > * { min-width: 0; }
.research-scroll:focus { outline: none; }
.research-scroll:focus-visible { box-shadow: inset 0 0 0 2px var(--color-accent); }
h3 { font-size: 18px; line-height: 1.45; margin: 0 0 12px; }
h4 { font-size: 14px; margin: 16px 0 8px; }
h4 small { font-size: 12px; font-weight: 400; color: var(--color-text-tertiary); }
p { color: var(--color-text-secondary); overflow-wrap: anywhere; }
button { font: inherit; font-size: 12px; cursor: pointer; color: var(--color-text-primary); background: var(--color-surface-1); border: 1px solid var(--color-border-0); border-radius: 8px; padding: 8px 12px; max-width: 100%; }
button:disabled { opacity: .5; cursor: not-allowed; }
button:hover:not(:disabled) { border-color: var(--color-accent); background: var(--color-accent-dim); }
.primary { background: var(--color-accent); color: var(--color-accent-contrast); border-color: transparent; }
.primary:hover:not(:disabled) { background: var(--color-accent); filter: brightness(1.07); }
button:focus-visible, input:focus-visible, select:focus-visible, summary:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
input:not([type=checkbox]), select { font: inherit; font-size: 12px; padding: 8px; border: 1px solid var(--color-border-0); border-radius: 7px; background: var(--color-surface-0); color: var(--color-text-primary); min-width: 0; max-width: 100%; }
input[type=checkbox] { accent-color: var(--color-accent); flex-shrink: 0; }
.health { display: flex; flex-wrap: wrap; gap: 8px 18px; margin-bottom: 16px; font-size: 12px; color: var(--color-text-tertiary); }
.health .good { color: var(--color-accent); }
.reading { max-width: none; margin: 0; }
.section-purpose { margin: 8px 0 14px; color: var(--color-text-secondary); font-size: 13px; }
.research-archive { margin-top: 24px; padding-top: 12px; border-top: 1px solid var(--color-border-0); }
.research-archive > summary { font-weight: 600; }

small { font-size: 12px; }
.toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 12px 14px; border-radius: 10px; background: var(--color-surface-2); }
.toolbar p { flex: 1 1 180px; margin: 0; font-size: 12px; }
.actions { display: flex; gap: 8px; flex-wrap: wrap; margin: 14px 0; }
.actions button { overflow-wrap: anywhere; }
.mode-grid { display: grid; grid-template-columns: repeat(3, minmax(0,1fr)); gap: 12px; margin: 16px 0; }
.mode-card { border: 1px solid var(--color-border-0); border-radius: 12px; padding: 16px; min-width: 0; background: var(--color-surface-1); }
.mode-card.selected { border-color: var(--color-accent); background: var(--color-accent-dim); box-shadow: inset 0 0 0 1px var(--color-accent); }
.mode-card h4 { display: flex; justify-content: space-between; align-items: flex-start; flex-wrap: wrap; gap: 8px; margin: 0 0 8px; }
.mode-card p { font-size: 12px; }
.mode-card small { color: var(--color-text-tertiary); font-size: 12px; }
.config-grid { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: 14px; margin: 16px 0; }
.config-grid label { display: flex; flex-direction: column; gap: 7px; min-width: 0; font-size: 12px; color: var(--color-text-secondary); }
.auto-toggle { display: flex; align-items: flex-start; gap: 12px; padding: 16px; border: 1px solid var(--color-border-0); border-radius: 12px; margin: 16px 0; }
.auto-toggle span { display: flex; flex-direction: column; gap: 8px; }
.auto-toggle small, .note, footer { color: var(--color-text-tertiary); font-size: 12px; }
.config-fields { border: 0; padding: 0; margin: 0; min-width: 0; }
.config-fields:disabled { opacity: .65; }
.workspace { display: grid; grid-template-columns: 230px minmax(0,1fr); gap: 16px; margin-top: 12px; }
aside { display: flex; flex-direction: column; gap: 8px; min-width: 0; max-height: 450px; overflow: auto; }
.experiment { display: flex; flex-direction: column; text-align: left; gap: 8px; padding: 14px; }
.experiment.selected { border-color: var(--color-accent); background: var(--color-accent-dim); }
.experiment strong { font-size: 14px; }
.experiment small { color: var(--color-text-tertiary); font-size: 12px; }
.detail { min-width: 0; border: 1px solid var(--color-border-0); padding: 18px; border-radius: 12px; }
.detail-title { display: flex; align-items: flex-start; justify-content: space-between; flex-wrap: wrap; gap: 10px; }
.badge { padding: 2px 8px; font-size: 12px; border-radius: 20px; background: var(--color-accent-dim); color: var(--color-accent); }
.badge[data-state=rejected], .badge[data-state=paused] { color: var(--color-text-tertiary); }
.hypothesis { font-size: 13px; white-space: pre-wrap; }
.limits, .targets { display: flex; gap: 7px; flex-wrap: wrap; }
.limits span, .targets span { background: var(--color-surface-2); padding: 4px 8px; border-radius: 6px; font-size: 12px; }
.targets small { color: var(--color-text-tertiary); }
.next-step { border-left: 3px solid var(--color-accent); padding: 9px 12px; background: var(--color-surface-2); font-size: 12px; }
.metrics { display: grid; grid-template-columns: repeat(4,minmax(0,1fr)); gap: 10px; margin: 16px 0; }
.metrics div { padding: 14px; border: 1px solid var(--color-border-0); background: var(--color-surface-1); border-radius: 10px; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.metrics small { font-size: 12px; color: var(--color-text-tertiary); }
.metrics b { font-size: 18px; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
.progress-row { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: 18px; font-size: 12px; }
.progress-row progress { display: block; width: 100%; height: 6px; accent-color: var(--color-accent); margin-top: 8px; }
.chart { margin: 20px 0; }
.chart svg { width: 100%; height: auto; }
.table-wrap { overflow: auto; max-height: 300px; border: 1px solid var(--color-border-0); border-radius: 8px; }
table { width: 100%; font-size: 12px; border-collapse: collapse; }
td, th { text-align: left; border-bottom: 1px solid var(--color-border-0); padding: 9px 10px; white-space: nowrap; }
th { background: var(--color-surface-2); }
.path, code { overflow-wrap: anywhere; }
details { margin-top: 16px; font-size: 12px; }
summary { cursor: pointer; padding: 4px 0; }
pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 300px; overflow: auto; padding: 12px; background: var(--color-surface-2); border-radius: 8px; font-size: 12px; }
.banner { padding: 12px; background: var(--color-surface-2); border-radius: 8px; font-size: 12px; display: flex; justify-content: space-between; flex-wrap: wrap; gap: 8px; }
.error { border-left: 3px solid #d65c5c; }
.success { border-left: 3px solid var(--color-accent); }
.last-auto { font-size: 12px; }
.wizard-result { display: flex; flex-direction: column; gap: 7px; padding: 16px; margin: 16px 0; border: 1px solid var(--color-border-0); border-radius: 10px; background: var(--color-surface-1); font-size: 12px; }
.wizard-result.verified { border-color: var(--color-accent); }
.wizard-result span { color: var(--color-text-secondary); }
.wizard-candidates { display: flex; flex-wrap: wrap; gap: 7px; margin: 12px 0; }
.wizard-candidates button { overflow-wrap: anywhere; text-align: left; }
.wizard-update { padding: 12px 0; margin: 10px 0; border-top: 1px solid var(--color-border-0); }
.wizard-update p { font-size: 12px; overflow-wrap: anywhere; }
.wizard-update button { margin-right: 8px; }
.wizard-url { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin: 10px 0; font-size: 12px; }
.wizard-url input, .wizard-url select { flex: 1 1 180px; }

@media (max-width: 1000px) { .mode-grid { grid-template-columns: repeat(2,minmax(0,1fr)); } }
@media (max-width: 760px) {
  .workspace { grid-template-columns: minmax(0,1fr); }
  aside { max-height: 180px; }
  .metrics { grid-template-columns: repeat(2,minmax(0,1fr)); }
  .progress-row { grid-template-columns: minmax(0,1fr); }
  .header-meta { display: none; }
  .research-scroll { padding: 14px; }
  .research-navigation { padding-inline: 12px; }
}
@media (max-width: 540px) {
  .mode-grid, .config-grid { grid-template-columns: minmax(0,1fr); }
  .research-title p { display: none; }
  .research-title h2 { font-size: 16px; }
  .research-mark { width: 30px; height: 30px; }
  .research-nav button { padding: 10px 12px; }
}
.research-background-jobs>summary{cursor:pointer;font-size:12px;color:var(--color-text-secondary);padding:10px 0}.research-background-jobs[open]>summary{margin-bottom:8px}
</style>
