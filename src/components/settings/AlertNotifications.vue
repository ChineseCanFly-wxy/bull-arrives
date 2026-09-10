<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { NButton, NModal, NPopconfirm, useMessage } from 'naive-ui';
import { applyHistoryWatermark, mergeHistoryEntries } from '@/utils/notificationHistory';
interface Delivery { id: string; history_version: number; native: string; native_error?: string; desktop: string; desktop_error?: string }
interface AlertNotice { id: string; title: string; body: string; received_at: number; history_version: number; delivery?: Delivery }
interface Snapshot { version: number; entries: AlertNotice[] }
const message = useMessage();
const entries = ref<AlertNotice[]>([]); const version = ref(0); const showHistory = ref(false); const clearing = ref(false);
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
  unlisteners.push(await listen<AlertNotice>('price-alert-triggered', ({payload}) => { if(payload.history_version < version.value)return; merge([payload]); }));
  unlisteners.push(await listen<Delivery>('notification-delivery-status', ({payload}) => { if(payload.history_version !== version.value)return; const item=entries.value.find(x=>x.id===payload.id); if(item)item.delivery=payload; if(payload.native_error) message.warning('系统通知发送失败，已尝试桌面提醒兜底。',{duration:10000,closable:true}); }));
  unlisteners.push(await listen<number>('notification-history-cleared', ({payload}) => { watermark(payload); }));
  const snapshot=await invoke<Snapshot>('get_notification_history'); if(!disposed && snapshot.version>=version.value){ watermark(snapshot.version); merge(snapshot.entries); }
 } catch(e){ if(!disposed)message.error(`提醒记录加载失败：${String(e)}`); }
});
onUnmounted(()=>{disposed=true;unlisteners.forEach(fn=>fn());});
</script>
<template>
<NButton v-if="entries.length" class="alert-history-button" size="small" @click="showHistory=true">提醒记录 · {{ entries.length }}</NButton>
<NModal v-model:show="showHistory" preset="card" title="本次运行的提醒记录" style="width:600px;max-width:92vw">
 <div class="history-head"><p class="notice-hint">保留最近 50 条。Windows 已受理不代表横幅必然可见。</p><NPopconfirm positive-text="清空" negative-text="取消" @positive-click="clearHistory"><template #trigger><NButton size="small" type="error" ghost :disabled="!entries.length||clearing" :loading="clearing">清空记录</NButton></template>仅清除提醒历史，不会删除逐票规则、冷却或每日触发标记。确定继续？</NPopconfirm></div>
 <ol v-if="entries.length" class="notice-list"><li v-for="entry in entries" :key="entry.id"><strong>{{entry.title}}</strong><time>{{new Date(entry.received_at).toLocaleTimeString()}}</time><p>{{entry.body}}</p><small v-if="entry.delivery">系统：{{entry.delivery.native==='accepted'?'已受理':entry.delivery.native==='failed'?'失败':'等待中'}} · 桌面：{{entry.delivery.desktop==='queued'?'已排队':entry.delivery.desktop==='failed'?'失败':entry.delivery.desktop==='not-requested'?'未启用':'等待中'}}</small></li></ol>
 <p v-else class="empty">暂无提醒记录</p>
</NModal>
</template>
<style scoped>
.alert-history-button{position:fixed;right:16px;bottom:38px;z-index:20}.history-head{display:flex;align-items:center;justify-content:space-between;gap:12px}.notice-hint{color:var(--color-text-secondary);font-size:12px}.notice-list{list-style:none;padding:0;max-height:55vh;overflow:auto}.notice-list li{padding:12px 0;border-bottom:1px solid var(--color-border)}.notice-list time{float:right;color:var(--color-text-secondary);font-size:12px}.notice-list p{margin:6px 0}.notice-list small{color:var(--color-text-tertiary)}.empty{padding:32px;text-align:center;color:var(--color-text-tertiary)}
</style>
