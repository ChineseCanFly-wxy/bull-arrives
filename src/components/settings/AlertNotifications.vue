<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { NButton, NModal, NPopconfirm, useMessage } from 'naive-ui';
import { applyHistoryWatermark, mergeHistoryEntries } from '@/utils/notificationHistory';
import { useSettingsStore } from '@/stores/settings';
interface Delivery { id: string; history_version: number; native: string; native_error?: string; desktop: string; desktop_error?: string }
interface AlertNotice { id: string; signal_id?: string; signal_tag?: string; signal_kind?: string; title: string; body: string; received_at: number; history_version: number; delivery?: Delivery; news_source?:string; original_title?:string; original_body?:string; agent_summary?:boolean }
interface Snapshot { version: number; entries: AlertNotice[] }
const message = useMessage();
const entries = ref<AlertNotice[]>([]); const version = ref(0); const showHistory = ref(false); const clearing = ref(false);
const settings=useSettingsStore();const archive=ref<AlertNotice[]>([]);const category=ref('news');const keyword=ref('');const newsBusy=ref(false);
const shown=computed(()=>{const rows=category.value==='alerts'?entries.value:archive.value.filter(e=>category.value==='timeline'?e.signal_kind==='timeline':e.signal_kind==='news');return rows.filter(e=>!keyword.value||`${e.title} ${e.body}`.includes(keyword.value));});
async function refreshNews(){try{archive.value=await invoke<AlertNotice[]>('get_news_archive');}catch(e){message.error(`资讯记录读取失败：${e}`);}}
async function toggleNews(){if(newsBusy.value)return;newsBusy.value=true;try{await settings.setSetting('news_notifications_enabled',settings.newsNotificationsEnabled?'0':'1');}finally{newsBusy.value=false;}}
watch(showHistory,open=>{if(open)void refreshNews();});
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
  unlisteners.push(await listen<AlertNotice>('price-alert-triggered', ({payload}) => { if(payload.history_version < version.value)return; merge([payload]); if(showHistory.value)void refreshNews(); }));
  unlisteners.push(await listen<Delivery>('notification-delivery-status', ({payload}) => { if(payload.history_version !== version.value)return; const item=entries.value.find(x=>x.id===payload.id); if(item)item.delivery=payload; if(payload.native_error) message.warning('系统通知发送失败，已尝试桌面提醒兜底。',{duration:10000,closable:true}); }));
  unlisteners.push(await listen<number>('notification-history-cleared', ({payload}) => { watermark(payload); }));
  const snapshot=await invoke<Snapshot>('get_notification_history'); if(!disposed && snapshot.version>=version.value){ watermark(snapshot.version); merge(snapshot.entries); }
 } catch(e){ if(!disposed)message.error(`提醒记录加载失败：${String(e)}`); }
});
onUnmounted(()=>{disposed=true;unlisteners.forEach(fn=>fn());});
</script>
<template>
<NButton class="alert-history-button" size="small" @click="showHistory=true">资讯与提醒 · {{ entries.length }}</NButton>
<NModal v-model:show="showHistory" preset="card" title="资讯与提醒" style="width:760px;max-width:94vw">
 <div class="news-intro"><div><b>与你关注的股票相关的事件</b><p>资讯保留最近 500 条，重启可查；首轮开启建立水位，后续交易时段有新事件才推送。</p></div><NButton size="small" :loading="newsBusy" @click="toggleNews">{{settings.newsNotificationsEnabled?'暂停资讯采集':'开启资讯采集'}}</NButton></div>
 <div class="news-tabs"><button v-for="tab in [{id:'news',label:'公司资讯'},{id:'timeline',label:'盘前 / 盘后'},{id:'alerts',label:'本次提醒'}]" :key="tab.id" :class="{active:category===tab.id}" @click="category=tab.id">{{tab.label}}</button><input v-model="keyword" placeholder="搜索标题或摘要"/><NButton size="small" @click="refreshNews">刷新</NButton></div>
 <template v-if="category==='alerts'">
 <div class="history-head"><p class="notice-hint">保留最近 50 条。Windows 已受理不代表横幅必然可见。</p><NPopconfirm positive-text="清空" negative-text="取消" @positive-click="clearHistory"><template #trigger><NButton size="small" type="error" ghost :disabled="!entries.length||clearing" :loading="clearing">清空记录</NButton></template>仅清除提醒历史，不会删除逐票规则、冷却或每日触发标记。确定继续？</NPopconfirm></div>
 </template>
 <ol v-if="shown.length" class="notice-list"><li v-for="entry in shown" :key="entry.id"><span v-if="entry.signal_tag" class="signal-tag" :data-kind="entry.signal_kind">{{entry.signal_tag}}</span><strong>{{entry.title}}</strong><time>{{new Date(entry.received_at).toLocaleString()}}</time><p>{{entry.body}}</p><small v-if="entry.news_source">{{entry.news_source}} · {{entry.agent_summary?'AI 摘要，以下可查采集原文':'规则原文'}} · 时间为接收时间</small><details v-if="entry.original_title||entry.original_body"><summary>查看采集原文</summary><b>{{entry.original_title}}</b><p>{{entry.original_body||'来源未提供正文'}}</p></details><small v-if="entry.delivery&&category==='alerts'">系统：{{entry.delivery.native==='accepted'?'已受理':entry.delivery.native==='failed'?'失败':entry.delivery.native==='not-requested'?'仅记录':'等待中'}} · 桌面：{{entry.delivery.desktop==='queued'?'已排队':entry.delivery.desktop==='failed'?'失败':entry.delivery.desktop==='not-requested'?'未启用':'等待中'}}</small></li></ol>
 <div v-else class="empty"><b>暂时没有匹配的内容</b><p>先开启资讯采集并添加自选股。周末、休市或没有命中规则时不会产生资讯；不会补推开启前的历史新闻。</p><p>盘前 / 盘后当前是规则统计卡片，尚不是 AI 全市场日报。</p></div>
</NModal>
</template>
<style scoped>
.alert-history-button{position:fixed;right:16px;bottom:38px;z-index:20}.history-head{display:flex;align-items:center;justify-content:space-between;gap:12px}.notice-hint{color:var(--color-text-secondary);font-size:12px}.notice-list{list-style:none;padding:0;max-height:55vh;overflow:auto}.notice-list li{padding:12px 0;border-bottom:1px solid var(--color-border)}.notice-list time{float:right;color:var(--color-text-secondary);font-size:12px}.notice-list p{margin:6px 0}.notice-list small{color:var(--color-text-tertiary)}.signal-tag{display:inline-block;margin-right:8px;padding:1px 6px;border-radius:999px;background:var(--color-surface-1);color:var(--color-accent);font-size:11px}.signal-tag[data-kind="risk"]{color:#d03050}.empty{padding:32px;text-align:center;color:var(--color-text-tertiary)}
.news-intro{display:flex;gap:18px;justify-content:space-between;align-items:center;padding:15px;background:var(--color-surface-2);border-radius:10px;margin-bottom:14px}.news-intro p{font-size:12px;color:var(--color-text-secondary);margin:5px 0}.news-tabs{display:flex;gap:8px;align-items:center;flex-wrap:wrap;border-bottom:1px solid var(--color-border-0);padding-bottom:12px}.news-tabs button{font:inherit;font-size:12px;border:0;background:transparent;color:var(--color-text-secondary);padding:7px;cursor:pointer}.news-tabs button.active{color:var(--color-accent);box-shadow:inset 0 -2px var(--color-accent)}.news-tabs input{flex:1;min-width:100px;padding:7px;background:var(--color-surface-2);color:var(--color-text-primary);border:1px solid var(--color-border-0);border-radius:6px}.notice-list details{margin-top:8px;font-size:12px}.notice-list summary{cursor:pointer;color:var(--color-accent)}.empty p{max-width:460px;margin:12px auto;line-height:1.7}.notice-list time{float:none;display:block;margin-top:7px;font-size:11px}</style>
