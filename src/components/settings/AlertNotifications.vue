<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { NButton, NModal, NPopconfirm, useMessage } from 'naive-ui';
import { applyHistoryWatermark, mergeHistoryEntries } from '@/utils/notificationHistory';
import { useSettingsStore } from '@/stores/settings';
interface Delivery { id: string; history_version: number; native: string; native_error?: string; desktop: string; desktop_error?: string }
interface AlertNotice { id: string; signal_id?: string; signal_tag?: string; signal_kind?: string; title: string; body: string; received_at: number; history_version: number; delivery?: Delivery; news_source?:string; original_title?:string; original_body?:string; agent_summary?:boolean; news_mode?:'direct'|'hybrid'|'ai' }
interface Snapshot { version: number; entries: AlertNotice[] }
interface BriefNews { signal_id: string; title: string; excerpt: string; source?: string; tag?: string; watchlist_match?: boolean; received_at?: number }
interface BriefAlert { title: string; body: string; received_at: number; kind: string }
interface DailyBrief { day: string; stage: 'preopen'|'postclose'; generated_at: string; window_start: string; window_end: string; news_count: number; alert_count: number; major: BriefNews[]; watchlist: BriefNews[]; other: BriefNews[]; alerts: BriefAlert[] }
const message = useMessage();
const entries = ref<AlertNotice[]>([]); const version = ref(0); const showHistory = ref(false); const clearing = ref(false);
const settings=useSettingsStore();const archive=ref<AlertNotice[]>([]);const category=ref('news');const keyword=ref('');const newsBusy=ref(false);
const briefs=ref<DailyBrief[]>([]);
const archiveById=computed(()=>new Map(archive.value.map(entry=>[entry.signal_id,entry])));
const aiBusyId=ref('');const aiUsage=ref<{day:string;used:number;limit:number}|null>(null);
const selectedId=ref('');
const aiOnly=ref(false);
const shown=computed(()=>{const rows=category.value==='alerts'?entries.value:archive.value.filter(e=>e.signal_kind==='news');return rows.filter(e=>(!aiOnly.value||e.agent_summary===true)&&(!keyword.value||`${e.title} ${e.body}`.includes(keyword.value)));});
async function refreshNews(){try{archive.value=await invoke<AlertNotice[]>('get_news_archive');}catch(e){message.error(`资讯记录读取失败：${e}`);}}
async function refreshBriefs(){try{briefs.value=await invoke<DailyBrief[]>('get_daily_briefs');}catch(e){message.error(`每日简报读取失败：${e}`);}}
function briefNewsBody(item:BriefNews){const current=archiveById.value.get(item.signal_id);return current?.agent_summary?current.body:item.excerpt;}
function analyzeBriefNews(item:BriefNews){const current=archiveById.value.get(item.signal_id);if(current)void analyzeNews(current);}
async function refreshAiUsage(){try{aiUsage.value=await invoke('get_news_ai_usage');}catch(e){message.error(`AI 名额读取失败：${e}`);}}
async function analyzeNews(entry:AlertNotice){if(!entry.signal_id||aiBusyId.value)return;aiBusyId.value=entry.signal_id;try{await invoke('analyze_archived_news',{signalId:entry.signal_id});await refreshNews();message.success('AI 解读已保存');}catch(e){message.error(`AI 解读失败：${e}`);}finally{aiBusyId.value='';}}
async function toggleNews(){if(newsBusy.value)return;newsBusy.value=true;try{if(!await settings.setSetting('news_notifications_enabled',settings.newsNotificationsEnabled?'0':'1'))message.error(settings.error||'资讯采集设置失败');}finally{newsBusy.value=false;}}
async function setNewsMode(mode:'direct'|'hybrid'){if(newsBusy.value)return;newsBusy.value=true;try{if(!await settings.setSetting('news_notification_mode',mode))message.error(settings.error||'切换资讯方式失败');}finally{newsBusy.value=false;}}
watch(showHistory,open=>{if(open){void refreshNews();void refreshBriefs();void refreshAiUsage();}});
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
async function clearHistory() { if (clearing.value) return; clearing.value = true; try { const next = await invoke<number>('clear_notification_history'); watermark(next); message.success('提醒记录已清空'); } catch(e) { message.error(`清空提醒记录失败：${String(e)}`); } finally { clearing.value = false; } }
onMounted(async () => {
 try {
  unlisteners.push(await listen<AlertNotice>('price-alert-triggered', ({payload}) => { if(payload.history_version < version.value)return; merge([payload]); if(showHistory.value){void refreshNews();void refreshAiUsage();} }));
  unlisteners.push(await listen<Delivery>('notification-delivery-status', ({payload}) => { if(payload.history_version !== version.value)return; const item=entries.value.find(x=>x.id===payload.id); if(item)item.delivery=payload; if(payload.native_error) message.warning('系统通知发送失败，已尝试桌面提醒兜底。',{duration:10000,closable:true}); }));
  unlisteners.push(await listen<number>('notification-history-cleared', ({payload}) => { watermark(payload); }));
  unlisteners.push(await listen<string>('notification-open-history', async ({payload}) => { category.value='alerts'; keyword.value=''; aiOnly.value=false; selectedId.value=payload; showHistory.value=true; await nextTick(); document.querySelector(`[data-notice-id="${CSS.escape(payload)}"]`)?.scrollIntoView({block:'nearest'}); }));
  unlisteners.push(await listen<AlertNotice>('news-analysis-updated', ({payload}) => { const current=entries.value.find(entry=>entry.signal_id===payload.signal_id);if(current)Object.assign(current,payload);if(showHistory.value){void refreshNews();void refreshAiUsage();} }));
  const snapshot=await invoke<Snapshot>('get_notification_history'); if(!disposed && snapshot.version>=version.value){ watermark(snapshot.version); merge(snapshot.entries); }
 } catch(e){ if(!disposed)message.error(`提醒记录加载失败：${String(e)}`); }
});
onUnmounted(()=>{disposed=true;unlisteners.forEach(fn=>fn());});
</script>
<template>
<NButton class="alert-history-button" size="small" @click="showHistory=true">资讯与提醒 · {{ entries.length }}</NButton>
<NModal v-model:show="showHistory" preset="card" title="资讯与提醒" style="width:760px;max-width:94vw">
 <div class="news-intro"><div><b>全市场重要事件</b><p>资讯保留最近 500 条，自选股重点标记；首次开启只建立水位，后续有新事件才通知。</p><div class="news-mode"><button type="button" :aria-pressed="settings.newsNotificationMode==='direct'" :disabled="newsBusy" @click="setNewsMode('direct')">全部直接通知</button><button type="button" :aria-pressed="settings.newsNotificationMode==='hybrid'" :disabled="newsBusy" @click="setNewsMode('hybrid')">混合模式</button></div><p v-if="aiUsage">今天自动 AI 解读 {{aiUsage.used}} / {{aiUsage.limit}} 条。重大事件、自选股、关注词自动解读；其余直接通知。关注词和上限在设置中修改。</p></div><NButton size="small" :loading="newsBusy" @click="toggleNews">{{settings.newsNotificationsEnabled?'暂停资讯采集':'开启资讯采集'}}</NButton></div>
 <div class="news-tabs"><button v-for="tab in [{id:'news',label:'市场资讯'},{id:'briefs',label:'每日简报'},{id:'alerts',label:'本次提醒'}]" :key="tab.id" :class="{active:category===tab.id}" @click="category=tab.id">{{tab.label}}</button><label v-if="category!=='briefs'" class="ai-filter"><input v-model="aiOnly" type="checkbox"/>仅看 AI 解读</label><input v-if="category!=='briefs'" v-model="keyword" placeholder="搜索标题或摘要"/><NButton size="small" @click="refreshNews();refreshBriefs()">刷新</NButton></div>
 <template v-if="category==='alerts'">
 <div class="history-head"><p class="notice-hint">保留最近 50 条。Windows 已受理不代表横幅必然可见。</p><NPopconfirm positive-text="清空" negative-text="取消" @positive-click="clearHistory"><template #trigger><NButton size="small" type="error" ghost :disabled="!entries.length||clearing" :loading="clearing">清空记录</NButton></template>仅清除提醒历史，不会删除逐票规则、冷却或每日触发标记。确定继续？</NPopconfirm></div>
 </template>
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
 <template v-else>
  <ol v-if="shown.length" class="notice-list"><li v-for="entry in shown" :key="entry.id" :data-notice-id="entry.id" :class="{selected:entry.id===selectedId}"><span v-if="entry.signal_tag" class="signal-tag" :data-kind="entry.signal_kind">{{entry.signal_tag}}</span><strong>{{entry.title}}</strong><time>{{new Date(entry.received_at).toLocaleString()}}</time><p>{{entry.body}}</p><small v-if="entry.news_source">{{entry.news_source}} · {{entry.agent_summary?'AI 解读':entry.news_mode==='hybrid'?'已播报原文，等待或未完成 AI 解读':'直接通知'}} · 时间为接收时间</small><button v-if="entry.signal_kind==='news'&&!entry.agent_summary" type="button" class="manual-ai-btn" :disabled="!!aiBusyId" @click="analyzeNews(entry)">{{aiBusyId===entry.signal_id?'AI 解读中…':'手动 AI 解读'}}</button><details v-if="entry.original_title||entry.original_body"><summary>查看采集原文</summary><b>{{entry.original_title}}</b><p>{{entry.original_body||'来源未提供正文'}}</p></details><small v-if="entry.delivery&&category==='alerts'">系统：{{entry.delivery.native==='accepted'?'已受理':entry.delivery.native==='failed'?'失败':entry.delivery.native==='not-requested'?'仅记录':'等待中'}} · 桌面：{{entry.delivery.desktop==='queued'?'已排队':entry.delivery.desktop==='failed'?'失败':entry.delivery.desktop==='not-requested'?'未启用':'等待中'}}</small></li></ol>
  <div v-else class="empty"><b>{{aiOnly?'没有匹配的 AI 解读':'暂时没有匹配的内容'}}</b><p v-if="aiOnly">这里只显示已完成并保存的 AI 解读。可以取消勾选，在资讯中手动解读；有搜索词时也可清空搜索。</p><p v-else>开启资讯采集后，全天命中重要事件规则的全市场快讯和公告会进入这里；无需添加自选股。首次开启不会补推历史内容。</p></div>
 </template>
</NModal>
</template>
<style scoped>
.ai-filter{display:flex;align-items:center;gap:5px;white-space:nowrap;font-size:12px;cursor:pointer}.news-tabs .ai-filter input{flex:none;min-width:0;margin:0;accent-color:var(--color-accent)}
.alert-history-button{position:fixed;right:16px;bottom:38px;z-index:20}.history-head{display:flex;align-items:center;justify-content:space-between;gap:12px}.notice-hint{color:var(--color-text-secondary);font-size:12px}.notice-list{list-style:none;padding:0;max-height:55vh;overflow:auto}.notice-list li{padding:12px 0;border-bottom:1px solid var(--color-border)}.notice-list time{float:right;color:var(--color-text-secondary);font-size:12px}.notice-list p{margin:6px 0}.notice-list small{color:var(--color-text-tertiary)}.signal-tag{display:inline-block;margin-right:8px;padding:1px 6px;border-radius:999px;background:var(--color-surface-1);color:var(--color-accent);font-size:11px}.signal-tag[data-kind="risk"]{color:#d03050}.empty{padding:32px;text-align:center;color:var(--color-text-tertiary)}
.news-intro{display:flex;gap:18px;justify-content:space-between;align-items:center;padding:15px;background:var(--color-surface-2);border-radius:10px;margin-bottom:14px}.news-intro p{font-size:12px;color:var(--color-text-secondary);margin:5px 0}.news-mode{display:flex;gap:6px;margin-top:8px}.news-mode button{border:1px solid var(--color-border-0);border-radius:6px;padding:5px 8px;background:var(--color-surface-1);color:var(--color-text-secondary);cursor:pointer}.news-mode button[aria-pressed="true"]{color:var(--color-accent);border-color:var(--color-accent)}.news-tabs{display:flex;gap:8px;align-items:center;flex-wrap:wrap;border-bottom:1px solid var(--color-border-0);padding-bottom:12px}.news-tabs button{font:inherit;font-size:12px;border:0;background:transparent;color:var(--color-text-secondary);padding:7px;cursor:pointer}.news-tabs button.active{color:var(--color-accent);box-shadow:inset 0 -2px var(--color-accent)}.news-tabs input{flex:1;min-width:100px;padding:7px;background:var(--color-surface-2);color:var(--color-text-primary);border:1px solid var(--color-border-0);border-radius:6px}.notice-list details{margin-top:8px;font-size:12px}.notice-list summary{cursor:pointer;color:var(--color-accent)}.empty p{max-width:460px;margin:12px auto;line-height:1.7}.notice-list time{float:none;display:block;margin-top:7px;font-size:11px}.notice-list li>p{white-space:pre-line}.notice-list li.selected{background:var(--color-surface-2);border-radius:6px;padding:12px 8px}.manual-ai-btn{display:block;margin:8px 0;padding:5px 9px;border:1px solid var(--color-border-0);border-radius:6px;background:var(--color-surface-1);color:var(--color-accent);cursor:pointer}.manual-ai-btn:disabled{opacity:.6;cursor:wait}
.brief-list{max-height:58vh;overflow:auto}.brief-caveat,.brief-window{font-size:12px;color:var(--color-text-secondary);line-height:1.6}.brief-card{padding:14px 0;border-bottom:1px solid var(--color-border)}.brief-card header{display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap}.brief-card h3,.brief-group h4{margin:0}.brief-card header small,.brief-group small{display:block;color:var(--color-text-tertiary);font-size:11px}.brief-group{margin-top:14px}.brief-group ol{list-style:none;margin:4px 0 0;padding:0}.brief-group li{padding:9px 0;border-bottom:1px solid var(--color-border-0)}.brief-group li p{margin:5px 0;white-space:pre-line;line-height:1.5}.brief-group li strong{font-size:13px}</style>
