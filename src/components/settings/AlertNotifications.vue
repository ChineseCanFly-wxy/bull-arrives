<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { NButton, NModal, NSwitch, useMessage } from 'naive-ui';
import { applyHistoryWatermark, mergeHistoryEntries } from '@/utils/notificationHistory';
import { useSettingsStore } from '@/stores/settings';
import MainlineResearch from '@/components/sector/MainlineResearch.vue';
import ModelConditionEvidence from '@/components/research/ModelConditionEvidence.vue';
import ModelFollowTrading from '@/components/research/ModelFollowTrading.vue';
import type { SectorKind } from '@/types/sector';
import { noticeCategory } from '@/utils/notificationCategory';
import ReminderControls from './ReminderControls.vue';
import FloatingAlertSettings from './FloatingAlertSettings.vue';
interface Delivery { id: string; history_version: number; native: string; native_error?: string; desktop: string; desktop_error?: string }
interface MainlineTarget { kind: SectorKind; sector_code: string; sector_name: string; fingerprint?: string | null; as_of?: string | null; match_basis?: string }
interface ModelSignal { symbol: string; score?: number | null; threshold?: number; as_of?: string; close?: number; cash_reference_quantity?: number; cash_reference_eligible?: boolean; signal_eligible?: boolean; reason?: string }
interface ModelNotice { kind?: string; run_id?: number; as_of?: string; follow_account_id?:number; order_id?:number; status?:string; context_as_of?:string; historical_catchup?: boolean; signals?: ModelSignal[]; message?: string }
interface AlertNotice { id: string; signal_id?: string; signal_tag?: string; signal_kind?: string; title: string; body: string; received_at: number; history_version: number; stockdb_update_alert?: {schema?:string}; delivery?: Delivery; news_source?:string; original_title?:string; original_body?:string; agent_summary?:boolean; news_mode?:string; sector_code?:string; sector_name?:string; kind?:SectorKind; fingerprint?:string; as_of?:string; related_mainlines?:MainlineTarget[]; published_at?:string|null; source_received_at?:string|number|null; delivered_at?:number; model_snapshot?:ModelNotice; intraday_snapshot?:Record<string,unknown>; condition_event?:{schema:string;event_key:string;model_name:string}; condition_events?:Array<{schema:string;event_key:string;model_name:string}> }
interface RawNotice extends Omit<AlertNotice,'received_at'> {received_at:number|string}
interface Snapshot { version: number; entries: AlertNotice[] }
interface BriefNews { signal_id: string; title: string; excerpt: string; source?: string; tag?: string; watchlist_match?: boolean; received_at?: number }
interface BriefAlert { title: string; body: string; received_at: number; kind: string }
interface DailyBrief { day: string; stage: 'preopen'|'postclose'; generated_at: string; window_start: string; window_end: string; news_count: number; alert_count: number; major: BriefNews[]; watchlist: BriefNews[]; other: BriefNews[]; alerts: BriefAlert[] }
const message = useMessage();
const entries = ref<AlertNotice[]>([]); const version = ref(0); const showHistory = ref(false);
const settings=useSettingsStore();const archive=ref<AlertNotice[]>([]);const category=ref('news');const keyword=ref('');
const briefs=ref<DailyBrief[]>([]);
const tabs = [
 {id:'news',label:'市场资讯',key:'news_notifications_enabled',defaultValue:'0',description:'只采集和推送市场资讯原文；首次开启只建立水位，不补推历史消息。AI 解读需要手动点击。'},
 {id:'mainline',label:'市场主线',key:'mainline_notifications_enabled',description:'只控制主线发现与观察变化提醒；主线扫描照常运行，与研究中心和资讯开关独立。'},
 {id:'trades',label:'模拟买卖',key:'model_trade_notifications_enabled',description:'研究中心自动跟随账户的委托、成交和拒单提醒。成交会标明买卖方向、股数与价格；关闭提醒不会暂停自动账户。'},
 {id:'conditions',label:'模型条件',key:'model_condition_notifications_enabled',description:'只控制研究中心模型条件触发提醒。候选信号不代表已买入，关闭后条件检查继续运行。'},
 {id:'intraday',label:'盘中确认',key:'intraday_notifications_enabled',description:'只控制研究中心盘中形态与仓位观察提醒；研究观察不等于下单或成交。'},
 {id:'research',label:'模型观察',key:'research_notifications_enabled',description:'只控制研究中心完成行情日的模型研究记录更新；不再联动主线、模拟买卖或模型条件。'},
 {id:'price',label:'行情提醒',key:'alerts_enabled',description:'控制逐票价格与涨跌幅提醒。双击自选股票或右键“设置行情提醒”修改每条规则与开关；仅监控当前分组。'},
 {id:'risk',label:'风险提醒',key:'risk_notifications_enabled',description:'控制智能监控的止损与止盈提醒，与普通行情提醒独立；仍遵循智能监控和逐票监控设置。'},
 {id:'data',label:'数据更新',key:'data_notifications_enabled',description:'控制数据更新连续失败的通知；关闭提醒不会停止更新和重试。'},
 {id:'briefs',label:'每日简报',key:'daily_briefs_enabled',description:'独立控制本机盘前与盘后简报生成；这里只展示简报，不发送买卖通知。已保存的简报仍可查看。'},
 {id:'operations',label:'自动操作',key:'',description:''},
 {id:'floating',label:'悬浮提醒',key:'',description:''},
];
const currentTab = computed(() => tabs.find(tab => tab.id === category.value) || tabs[0]);

interface OperationEntry { id:number; at:number; level:string; source:string; message:string; repeats:number }
interface OperationSnapshot { enabled:boolean; version:number; entries:OperationEntry[] }
const operations=ref<OperationSnapshot>({enabled:settings.automaticOperationsLogEnabled,version:0,entries:[]});
const operationsBusy=ref(false);const operationsToggleBusy=ref(false);const operationsError=ref('');
const shownOperations=computed(()=>operations.value.entries.filter(row=>!keyword.value||row.source.includes(keyword.value)||row.message.includes(keyword.value)));
let operationsTimer:ReturnType<typeof setInterval>|undefined;
function applyOperations(snapshot:OperationSnapshot){if(!disposed&&snapshot.version>=operations.value.version)operations.value=snapshot;}
async function refreshOperations(){if(operationsBusy.value)return;operationsBusy.value=true;try{applyOperations(await invoke<OperationSnapshot>('get_automatic_operations_log'));operationsError.value='';}catch(e){if(!disposed)operationsError.value=`自动操作记录读取失败：${e}`;}finally{operationsBusy.value=false;}}
async function toggleOperations(enabled:boolean){if(operationsToggleBusy.value)return;operationsToggleBusy.value=true;try{if(!await settings.setSetting('automatic_operations_log_enabled',enabled?'1':'0'))message.error(settings.error||'记录开关保存失败');await refreshOperations();}finally{operationsToggleBusy.value=false;}}
async function clearOperations(){try{applyOperations(await invoke<OperationSnapshot>('clear_automatic_operations_log'));message.success('自动操作记录已清空');}catch(e){message.error(`清空自动操作记录失败：${e}`);}}
function refreshCurrent(){if(category.value==='operations')void refreshOperations();else{void refreshNews();void refreshBriefs();void refreshResearch();}}
watch([showHistory,category],([open,tab])=>{if(operationsTimer)clearInterval(operationsTimer);operationsTimer=undefined;if(open&&tab==='operations'){void refreshOperations();operationsTimer=setInterval(()=>void refreshOperations(),2000);}});
const researchArchive=ref<AlertNotice[]>([]);const mainlineTarget=ref<MainlineTarget|null>(null);const showResearch=ref(false);
const conditionEventKey=ref<string|null>(null);const showCondition=ref(false);
function conditionEvents(entry:AlertNotice){return (entry.condition_events?.length?entry.condition_events:entry.condition_event?[entry.condition_event]:[]).filter(e=>e.schema==='model-condition-event-v1'&&typeof e.event_key==='string');}
function openCondition(key:string){conditionEventKey.value=key;showCondition.value=true;}
const modelTarget=ref<ModelNotice|null>(null);const showModel=ref(false);
const showFollow=ref(false);const followAccountId=ref<number|undefined>();
function followSnapshot(entry:AlertNotice){const row=entry.model_snapshot;return typeof row?.follow_account_id==='number'&&Number.isSafeInteger(row.follow_account_id)&&row.follow_account_id>0?row:null;}
function openFollow(entry:AlertNotice){const row=followSnapshot(entry);if(row){followAccountId.value=row.follow_account_id;showFollow.value=true;}}
const intradayTarget=ref<Record<string,unknown>|null>(null);const showIntraday=ref(false);
const intradayLevels=computed(()=>{const value=intradayTarget.value?.levels;return value&&typeof value==='object'?value as Record<string,unknown>:{};});
const intradayPlan=computed(()=>{const value=intradayTarget.value?.execution_plan;return value&&typeof value==='object'?value as Record<string,unknown>:{};});
function intradaySnapshot(entry:AlertNotice){const row=entry.intraday_snapshot;return row?.schema==='ashare-intraday-observation-v1'&&typeof row.symbol==='string'&&typeof row.checked_at==='string'?row:null;}
function openIntraday(entry:AlertNotice){const row=intradaySnapshot(entry);if(row){intradayTarget.value=row;showIntraday.value=true;}}
const archiveById=computed(()=>new Map(archive.value.map(entry=>[entry.signal_id,entry])));
const aiBusyId=ref('');
const selectedId=ref('');
const shown=computed(()=>{
 const rows=category.value==='news'?archive.value.filter(e=>noticeCategory(e)==='news'):[...new Map([...researchArchive.value,...entries.value].filter(e=>noticeCategory(e)===category.value).map(e=>[e.id,e])).values()].sort((a,b)=>b.received_at-a.received_at);
 return rows.filter(e=>!keyword.value||`${e.title} ${e.body}`.includes(keyword.value));
});
function normalizeNotice(row:RawNotice):AlertNotice{const at=typeof row.received_at==='number'?row.received_at:Date.parse(row.received_at);return {...row,received_at:Number.isFinite(at)?at:0};}
async function refreshNews(){try{archive.value=(await invoke<RawNotice[]>('get_news_archive')).map(normalizeNotice);}catch(e){message.error(`资讯记录读取失败：${e}`);}}
async function refreshResearch(){try{researchArchive.value=(await invoke<RawNotice[]>('get_alert_archive')).map(normalizeNotice);}catch(e){message.error(`提醒归档读取失败：${e}`);}}
function modelSnapshot(entry:AlertNotice){const row=entry.model_snapshot;return row?.kind==='model_research_observation'&&typeof row.run_id==='number'&&Number.isFinite(row.run_id)&&Array.isArray(row.signals)?row:null;}
function openModel(entry:AlertNotice){const snapshot=modelSnapshot(entry);if(!snapshot)return;modelTarget.value=snapshot;showModel.value=true;}
function modelNumber(value:unknown,suffix=''){return typeof value==='number'&&Number.isFinite(value)?`${value.toFixed(4)}${suffix}`:'--';}
function mainlines(entry:AlertNotice):MainlineTarget[]{const own=entry.signal_kind==='research'&&entry.sector_code&&entry.sector_name&&entry.kind?[{kind:entry.kind,sector_code:entry.sector_code,sector_name:entry.sector_name,fingerprint:entry.fingerprint,as_of:entry.as_of}]:[];return [...own,...(entry.related_mainlines??[])].filter(row=>['industry','concept'].includes(row.kind)&&/^(BK\d{4}|SW801\d{3})$/.test(row.sector_code)&&row.sector_name);}
function openMainline(target:MainlineTarget){mainlineTarget.value=target;showResearch.value=true;}
async function refreshBriefs(){try{briefs.value=await invoke<DailyBrief[]>('get_daily_briefs');}catch(e){message.error(`每日简报读取失败：${e}`);}}
function briefNewsBody(item:BriefNews){const current=archiveById.value.get(item.signal_id);return current?.agent_summary?current.body:item.excerpt;}
function analyzeBriefNews(item:BriefNews){const current=archiveById.value.get(item.signal_id);if(current)void analyzeNews(current);}
async function analyzeNews(entry:AlertNotice){if(!entry.signal_id||aiBusyId.value)return;aiBusyId.value=entry.signal_id;try{await invoke('analyze_archived_news',{signalId:entry.signal_id});await refreshNews();message.success('AI 解读已保存');}catch(e){message.error(`AI 解读失败：${e}`);}finally{aiBusyId.value='';}}
watch(showHistory,open=>{if(open){void refreshNews();void refreshBriefs();void refreshResearch();}});
watch(category,value=>{if(value==='briefs')void refreshBriefs();});
const unlisteners: UnlistenFn[] = []; let disposed = false;
function watermark(value: number) {
  const next = applyHistoryWatermark({ version: version.value, entries: entries.value }, value);
  version.value = next.version;
  entries.value = next.entries;
}
function merge(items: AlertNotice[]) {
  const next = mergeHistoryEntries({ version: version.value, entries: entries.value }, items);
  version.value = next.version;
  entries.value = next.entries;
}
onMounted(async () => {
 try {
  unlisteners.push(await listen<RawNotice>('price-alert-triggered', ({payload}) => { if(payload.history_version < version.value)return; merge([normalizeNotice(payload)]); if(showHistory.value){void refreshNews();void refreshResearch();} }));
  unlisteners.push(await listen<Delivery>('notification-delivery-status', ({payload}) => { if(payload.history_version !== version.value)return; const item=entries.value.find(x=>x.id===payload.id); if(item)item.delivery=payload; if(payload.native_error) message.warning('系统通知发送失败，已尝试桌面提醒兜底。',{duration:10000,closable:true}); }));
  unlisteners.push(await listen<number>('notification-history-cleared', ({payload}) => { watermark(payload); }));
  unlisteners.push(await listen<string>('notification-open-settings', ({payload}) => { category.value=tabs.some(tab=>tab.id===payload)?payload:'trades'; keyword.value=''; showHistory.value=true; }));
  unlisteners.push(await listen<string>('notification-open-history', async ({payload}) => {
    keyword.value=''; selectedId.value=payload; showHistory.value=true;
    await Promise.all([refreshResearch(),refreshNews()]);
    const entry=[...entries.value,...researchArchive.value,...archive.value].find(e=>e.id===payload);
    category.value=entry?noticeCategory(entry):'price';
    await nextTick(); document.querySelector(`[data-notice-id="${CSS.escape(payload)}"]`)?.scrollIntoView({block:'nearest'});
  }));
  unlisteners.push(await listen<AlertNotice>('news-analysis-updated', ({payload}) => { const current=entries.value.find(entry=>entry.signal_id===payload.signal_id);if(current)Object.assign(current,payload);if(showHistory.value){void refreshNews();} }));
  const snapshot=await invoke<Snapshot>('get_notification_history'); if(!disposed && snapshot.version>=version.value){ watermark(snapshot.version); merge(snapshot.entries.map(normalizeNotice)); }
 } catch(e){ if(!disposed)message.error(`提醒记录加载失败：${String(e)}`); }
});
onUnmounted(()=>{disposed=true;if(operationsTimer)clearInterval(operationsTimer);unlisteners.forEach(fn=>fn());});
</script>
<template>
<NButton class="alert-history-button" size="small" @click="showHistory=true">资讯与提醒</NButton>
<NModal v-model:show="showHistory" preset="card" title="资讯与提醒" style="width:760px;max-width:94vw">
 <div class="news-tabs" role="tablist" aria-label="提醒分类"><button v-for="tab in tabs" :key="tab.id" type="button" role="tab" :aria-selected="category===tab.id" :class="{active:category===tab.id}" @click="category=tab.id">{{tab.label}}</button><input v-if="!['briefs','floating'].includes(category)" v-model="keyword" :placeholder="category==='operations'?'搜索操作或来源':'搜索本类标题或摘要'"/><NButton size="small" @click="refreshCurrent">刷新</NButton></div>
 <ReminderControls v-if="currentTab.key" :key="category" :category="category" :label="currentTab.label" :setting-key="currentTab.key" :default-value="currentTab.defaultValue" :description="currentTab.description" />
 <FloatingAlertSettings v-if="category==='floating'" />
 <p v-if="['mainline','trades','conditions','intraday','research'].includes(category)" class="notice-hint">按通知当时快照留存14日，重启仍可查看。查看记录不会执行买卖；候选、委托和实际模拟成交是不同状态。</p>
 <section v-if="category==='operations'" class="operations-panel">
  <div class="operations-heading"><div><b>自动操作记录</b><p>只监控研究中心、主线和相关数据更新，不包含资讯采集。只记录本次运行，最多200条；关闭记录不影响自动任务与提醒。</p></div><label class="operations-toggle"><span>记录自动操作</span><NSwitch :value="operations.enabled" :disabled="operationsToggleBusy" :loading="operationsToggleBusy" aria-label="记录自动操作" @update:value="toggleOperations" /></label></div>
  <div class="operations-toolbar"><span>{{operations.enabled?'记录已开启':'记录已关闭'}} · {{operations.entries.length}} 条 · 页面打开时自动刷新</span><NButton size="small" :disabled="!operations.entries.length" @click="clearOperations">清空记录</NButton></div>
  <p v-if="operationsError" class="operations-error" role="alert">{{operationsError}}</p>
  <ol v-if="shownOperations.length" class="operations-list" aria-label="自动操作记录列表"><li v-for="row in shownOperations" :key="row.id" :data-operation-id="row.id"><header><time>{{new Date(row.at).toLocaleString('zh-CN',{hour12:false})}}</time><span class="operation-source">{{row.source}}</span><span class="operation-level" :data-level="row.level">{{row.level}}</span><small v-if="row.repeats>1">重复 {{row.repeats}} 次</small></header><p>{{row.message}}</p></li></ol>
  <div v-else class="empty"><b>{{keyword?'没有匹配的自动操作':operations.enabled?'等待下一次自动操作':'自动操作记录已关闭'}}</b><p>数据更新与重试、模型检查与建账、模拟买卖、主线扫描和自动研究会在这里显示。连续相同内容合并；记录不会弹出系统通知，也不会补记关闭期间的操作。</p></div>
 </section>
 <section v-if="category==='briefs'" class="brief-list">
  <p class="brief-caveat">交易日盘前 09:10、盘后 15:15 后，从本机已保存的记录生成。关闭资讯采集时不会纳入新资讯；离线时段无法补齐。简报不会自动调用 AI。</p>
  <article v-for="brief in briefs" :key="`${brief.day}-${brief.stage}`" class="brief-card">
   <header><h3>{{brief.day}} · {{brief.stage==='preopen'?'盘前简报':'盘后简报'}}</h3><small>生成于 {{new Date(brief.generated_at).toLocaleString()}}</small></header>
   <p class="brief-window">采集窗口：{{new Date(brief.window_start).toLocaleString()}} 至 {{new Date(brief.window_end).toLocaleString()}} · 资讯 {{brief.news_count}} 条<span v-if="brief.stage==='postclose'"> · 价格/风险提醒 {{brief.alert_count}} 条</span></p>
   <div v-for="group in [{title:'重大事件 · 最近 5 条',items:brief.major},{title:'自选股相关 · 最近 5 条',items:brief.watchlist},{title:'其他重要资讯 · 最近 3 条',items:brief.other}]" :key="group.title" v-show="group.items.length" class="brief-group">
    <h4>{{group.title}}</h4>
    <ol><li v-for="item in group.items" :key="item.signal_id"><span v-if="item.watchlist_match" class="signal-tag">★自选</span><strong>{{item.title}}</strong><small>{{item.source||'资讯'}} · {{item.tag||'事件'}} · {{item.received_at?new Date(item.received_at).toLocaleString():'时间未知'}}</small><p>{{briefNewsBody(item)}}</p><button v-if="archiveById.get(item.signal_id)&&!archiveById.get(item.signal_id)?.agent_summary" type="button" class="manual-ai-btn" :disabled="!!aiBusyId" @click="analyzeBriefNews(item)">{{aiBusyId===item.signal_id?'AI 解读中…':'手动 AI 解读'}}</button><small v-else-if="archiveById.get(item.signal_id)?.agent_summary">已保存 AI 解读</small></li></ol>
   </div>
   <div v-if="brief.alerts.length" class="brief-group"><h4>当天价格与风险提醒 · 最近 10 条</h4><ol><li v-for="alert in brief.alerts" :key="`${alert.received_at}-${alert.title}`"><strong>{{alert.title}}</strong><small>{{new Date(alert.received_at).toLocaleString()}}</small><p>{{alert.body}}</p></li></ol></div>
  </article>
  <div v-if="!briefs.length" class="empty"><b>还没有每日简报</b><p>交易日运行到盘前或盘后时段后，会根据本机已有的资讯与价格/风险提醒生成；没有记录时不会创建空简报。</p></div>
 </section>
 <template v-else-if="!['operations','floating'].includes(category)">
  <ol v-if="shown.length" class="notice-list"><li v-for="entry in shown" :key="entry.id" :data-notice-id="entry.id" :class="{selected:entry.id===selectedId}"><span v-if="entry.signal_tag" class="signal-tag" :data-kind="entry.signal_kind">{{entry.signal_tag}}</span><strong>{{entry.title}}</strong><time>{{entry.received_at?new Date(entry.received_at).toLocaleString():'接收时间未核实'}}</time><p>{{entry.body}}</p><small v-if="entry.news_source">{{entry.news_source}} · {{entry.agent_summary?'已保存 AI 解读':'原文通知'}} · 发布时间 {{entry.published_at||'未核实'}} · 接收时间 {{entry.source_received_at||new Date(entry.received_at).toLocaleString()}}</small><div class="notice-mainlines"><button v-if="followSnapshot(entry)" class="notice-follow-open" type="button" @click="openFollow(entry)">查看跟随账户 #{{entry.model_snapshot?.follow_account_id}} · 操作单 #{{entry.model_snapshot?.order_id}}</button><button v-for="target in mainlines(entry)" :key="`${target.kind}-${target.sector_code}-${target.fingerprint}`" type="button" @click="openMainline(target)">查看 {{target.sector_name}} · {{target.as_of||'日期未核实'}} {{target.fingerprint?'通知快照':'当前观察'}} <small v-if="target.match_basis">关联依据 {{target.match_basis}}</small></button><button v-for="event in conditionEvents(entry)" :key="event.event_key" class="notice-condition-open" type="button" @click="openCondition(event.event_key)">查看 {{event.model_name}} 触发条件证据</button><button v-if="intradaySnapshot(entry)" class="notice-intraday-open" type="button" @click="openIntraday(entry)">查看分钟形态与仓位研究 · {{entry.intraday_snapshot?.symbol}}</button><button v-if="modelSnapshot(entry)" class="notice-model-open" type="button" @click="openModel(entry)">查看模型账户 #{{entry.model_snapshot?.run_id}} · {{entry.model_snapshot?.as_of}} 通知快照</button></div><button v-if="entry.signal_kind==='news'&&!entry.agent_summary" type="button" class="manual-ai-btn" :disabled="!!aiBusyId" @click="analyzeNews(entry)">{{aiBusyId===entry.signal_id?'AI 解读中…':'手动 AI 解读'}}</button><details v-if="entry.original_title||entry.original_body"><summary>查看采集原文</summary><b>{{entry.original_title}}</b><p>{{entry.original_body||'来源未提供正文'}}</p></details><small v-if="entry.delivery&&entries.some(item=>item.id===entry.id)">系统：{{entry.delivery.native==='accepted'?'已受理':entry.delivery.native==='failed'?'失败':entry.delivery.native==='not-requested'?'仅记录':'等待中'}} · 桌面：{{entry.delivery.desktop==='queued'?'已排队':entry.delivery.desktop==='failed'?'失败':entry.delivery.desktop==='not-requested'?'未启用':'等待中'}}</small></li></ol>
  <div v-else class="empty"><b>暂时没有匹配的内容</b><p>{{currentTab.description}}</p></div>
 </template>
</NModal>
<NModal v-model:show="showResearch" preset="card" :title="`${mainlineTarget?.sector_name||'主线'} · 通知证据`" style="width:1000px;max-width:96vw"><MainlineResearch v-if="showResearch&&mainlineTarget" :kind="mainlineTarget.kind" :sector-code="mainlineTarget.sector_code" :sector-name="mainlineTarget.sector_name" :snapshot-fingerprint="mainlineTarget.fingerprint||undefined" /></NModal>
<NModal v-model:show="showFollow" preset="card" title="模型跟随账户 · 操作与成交" style="width:1120px;max-width:96vw"><div style="max-height:75vh;overflow:auto;min-width:0"><ModelFollowTrading v-if="showFollow" :source-runs="[]" :preferred-account-id="followAccountId" /></div></NModal>
<NModal v-model:show="showModel" preset="card" title="模型账户 · 通知当时观察" style="width:880px;max-width:96vw"><section v-if="modelTarget" class="notice-model-snapshot"><p>账户 #{{modelTarget.run_id}} · 数据截止 {{modelTarget.as_of}} · {{modelTarget.historical_catchup?'历史补齐，不能称实时':'截至该完成行情日'}}。</p><p>{{modelTarget.message||'这是未准入的纸上研究记录，非买入许可。'}}</p><p>此处只展示通知时保存的快照，账户完整净值和订单请在研究中心核对；不会自动重跑模型或调用Claude。</p><p v-if="!modelTarget.signals?.length">该版没有通过模型阈值的观察记录，允许空仓。</p><article v-for="signal in modelTarget.signals" :key="signal.symbol"><b>{{signal.symbol}} · {{signal.as_of||modelTarget.as_of}}</b><p>模型原始标签分 {{modelNumber(signal.score)}} · 阈值 {{modelNumber(signal.threshold)}}（不同标签含义见研究中心，分数不是胜率）；截止收盘 {{modelNumber(signal.close,'元')}}。</p><p>纸上现金参考股数 {{signal.cash_reference_quantity??'--'}} · {{signal.cash_reference_eligible===true?'参考预算满足最低单位，尚未成交':'参考预算未通过或未知，不扩仓'}}；下一开盘/分钟确认仍未知。</p><p>{{signal.reason||'观察记录，未准入且不自动下单。'}}</p></article><details><summary>通知原始快照 JSON</summary><pre>{{JSON.stringify(modelTarget,null,2)}}</pre></details></section></NModal>
<NModal v-model:show="showIntraday" preset="card" title="分钟形态 · 通知时证据" style="width:880px;max-width:96vw"><section v-if="intradayTarget" class="notice-model-snapshot"><p>{{intradayTarget.name||intradayTarget.symbol}} · 模型截止 {{intradayTarget.frozen_as_of}} · 分钟观察 {{intradayTarget.checked_at}}</p><p>{{intradayTarget.message}}</p><p>最新完成分钟 {{intradayTarget.latest_completed_minute||'未知'}}；观测价 {{modelNumber(intradayLevels.observed_close,'元')}} · 成交均价 {{modelNumber(intradayLevels.vwap,'元')}} · 前20日高点 {{modelNumber(intradayLevels.previous_20d_high,'元')}} · 失效支撑 {{modelNumber(intradayLevels.invalidation_close_below,'元')}}。</p><p>突破 {{intradayTarget.breakout_at||'尚未出现'}} → 回踩 {{intradayTarget.retest_at||'尚未出现'}} → 再确认 {{intradayTarget.confirmed_at||'尚未出现'}}。</p><p>{{intradayPlan.initial||'初始仓位条件未知'}}；{{intradayPlan.add||'加仓条件未知'}}</p><p>{{intradayPlan.trim_profit||'止盈研究条件未知'}} · 价格参考 {{modelNumber(intradayPlan.trim_profit_above,'元')}}；{{intradayPlan.reduce_weakness||'减仓条件未知'}}</p><p>观察价格不是成交委托价。分批仓位属于研究计划，A股当天买入不能当天卖出；分钟形态尚未通过多年盈利检验。此处读取保存的通知，不重新联网或调用AI。</p><details><summary>形态、量额、支撑、仓位与来源指纹</summary><pre>{{JSON.stringify(intradayTarget,null,2)}}</pre></details></section></NModal>
<ModelConditionEvidence :show="showCondition" :event-key="conditionEventKey" @update:show="showCondition=$event" />
</template>
<style scoped>
.operations-heading{display:flex;justify-content:space-between;align-items:center;gap:16px;flex-wrap:wrap;margin:16px 0}.operations-heading b{font-size:15px}.operations-heading p{font-size:13px;line-height:1.7;color:var(--color-text-secondary);max-width:470px;margin:6px 0}.operations-toggle{display:flex;align-items:center;gap:10px;font-size:13px;white-space:nowrap}.operations-toolbar{display:flex;align-items:center;justify-content:space-between;gap:12px;font-size:13px;color:var(--color-text-secondary)}.operations-error{color:var(--color-error);font-size:13px}.operations-list{list-style:none;margin:12px 0 0;padding:0;max-height:52vh;overflow:auto}.operations-list li{padding:12px;border:1px solid var(--color-border-0);border-radius:6px;margin-bottom:8px;background:var(--color-surface-1)}.operations-list header{display:flex;align-items:center;gap:8px;flex-wrap:wrap;font-size:13px;color:var(--color-text-secondary)}.operations-list time{font-variant-numeric:tabular-nums}.operation-source{color:var(--color-accent);font-weight:600}.operation-level[data-level="失败"]{color:var(--color-error)}.operation-level[data-level="注意"]{color:var(--color-warning)}.operations-list p{font-size:14px;line-height:1.7;white-space:pre-wrap;overflow-wrap:anywhere;margin:8px 0 0;color:var(--color-text-primary)}

.notice-model-snapshot{line-height:1.7;font-size:13px}.notice-model-snapshot article{padding:10px;margin-top:8px;border:1px solid var(--color-border-0);border-radius:6px}.notice-model-snapshot pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:300px;overflow:auto;font-size:var(--text-xs);font-family:var(--font-sans)}
.notice-mainlines{display:flex;gap:6px;flex-wrap:wrap}.notice-mainlines button{font:inherit;font-size:12px;border:1px solid var(--color-border-0);background:var(--color-surface-2);color:var(--color-accent);border-radius:6px;padding:6px 9px;cursor:pointer}.notice-mainlines small{display:block;margin-top:3px}
.alert-history-button{position:fixed;right:16px;bottom:38px;z-index:20}.history-head{display:flex;align-items:center;justify-content:space-between;gap:12px}.notice-hint{color:var(--color-text-secondary);font-size:12px}.notice-list{list-style:none;padding:0;max-height:55vh;overflow:auto}.notice-list li{padding:12px 0;border-bottom:1px solid var(--color-border)}.notice-list time{float:right;color:var(--color-text-secondary);font-size:12px}.notice-list p{margin:6px 0}.notice-list small{color:var(--color-text-tertiary)}.signal-tag{display:inline-block;margin-right:8px;padding:1px 6px;border-radius:999px;background:var(--color-surface-1);color:var(--color-accent);font-size:var(--text-xs)}.signal-tag[data-kind="risk"]{color:#d03050}.empty{padding:32px;text-align:center;color:var(--color-text-tertiary)}
.news-intro{display:flex;gap:18px;justify-content:space-between;align-items:center;padding:15px;background:var(--color-surface-2);border-radius:10px;margin-bottom:14px}.news-intro p{font-size:12px;color:var(--color-text-secondary);margin:5px 0}.news-tabs{display:flex;gap:8px;align-items:center;flex-wrap:wrap;border-bottom:1px solid var(--color-border-0);padding-bottom:12px}.news-tabs button{font:inherit;font-size:12px;border:0;background:transparent;color:var(--color-text-secondary);padding:7px;cursor:pointer}.news-tabs button.active{color:var(--color-accent);box-shadow:inset 0 -2px var(--color-accent)}.news-tabs input{flex:1;min-width:100px;padding:7px;background:var(--color-surface-2);color:var(--color-text-primary);border:1px solid var(--color-border-0);border-radius:6px}.notice-list details{margin-top:8px;font-size:12px}.notice-list summary{cursor:pointer;color:var(--color-accent)}.empty p{max-width:460px;margin:12px auto;line-height:1.7}.notice-list time{float:none;display:block;margin-top:7px;font-size:var(--text-xs)}.notice-list li>p{white-space:pre-line}.notice-list li.selected{background:var(--color-surface-2);border-radius:6px;padding:12px 8px}.manual-ai-btn{display:block;margin:8px 0;padding:5px 9px;border:1px solid var(--color-border-0);border-radius:6px;background:var(--color-surface-1);color:var(--color-accent);cursor:pointer}.manual-ai-btn:disabled{opacity:.6;cursor:wait}
.brief-list{max-height:58vh;overflow:auto}.brief-caveat,.brief-window{font-size:12px;color:var(--color-text-secondary);line-height:1.6}.brief-card{padding:14px 0;border-bottom:1px solid var(--color-border)}.brief-card header{display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap}.brief-card h3,.brief-group h4{margin:0}.brief-card header small,.brief-group small{display:block;color:var(--color-text-tertiary);font-size:var(--text-xs)}.brief-group{margin-top:14px}.brief-group ol{list-style:none;margin:4px 0 0;padding:0}.brief-group li{padding:9px 0;border-bottom:1px solid var(--color-border-0)}.brief-group li p{margin:5px 0;white-space:pre-line;line-height:1.5}.brief-group li strong{font-size:13px}</style>
