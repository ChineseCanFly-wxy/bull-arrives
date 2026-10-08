<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useSettingsStore, SETTING_CHANGED_EVENT, type SettingChangedPayload } from '@/stores/settings';
import WindowResizeHandle from '@/components/common/WindowResizeHandle.vue';
interface Entry { id: string; title: string; body: string; notification_category: string; received_at: number | string }
interface Snapshot { version: number; entries: Entry[] }
const settings = useSettingsStore(); const snapshot = ref<Snapshot>({version:0,entries:[]}); const error = ref('');
const pending = ref(new Set<string>()); const unlisteners: UnlistenFn[] = []; let disposed = false;
const labels: Record<string,string> = {mainline:'市场主线',trades:'模拟买卖',conditions:'模型条件',intraday:'盘中确认',price:'行情提醒',risk:'风险提醒',floating:'测试提醒'};
const style = computed(() => ({ color:settings.settings.important_alerts_text_color || '#EEF5FF', background:settings.settings.important_alerts_background || '#111821', opacity:navigator.userAgent.includes('Windows') ? 1 : Number(settings.settings.important_alerts_opacity || 95)/100 }));
function apply(value: Snapshot) { if (!disposed && value.version >= snapshot.value.version) snapshot.value = value; }
async function act(command: string, id: string) {
  if (pending.value.has(id)) return; pending.value.add(id); error.value = '';
  try { await invoke(command,{id}); } catch (e) { error.value = String(e); } finally { pending.value.delete(id); }
}
async function drag() { try { await getCurrentWindow().startDragging(); } catch (e) { error.value = String(e); } }
onMounted(async () => {
  try {
    unlisteners.push(await listen<Snapshot>('important-alerts-changed',({payload})=>apply(payload)));
    unlisteners.push(await listen<SettingChangedPayload>(SETTING_CHANGED_EVENT,({payload})=>settings.applyRemoteSetting(payload.key,payload.value)));
    if (!await settings.fetchSettings()) throw new Error(settings.error || '提醒设置加载失败');
    apply(await invoke<Snapshot>('get_important_alerts'));
  } catch (e) { error.value = String(e); }
  if (disposed) unlisteners.splice(0).forEach(fn=>fn());
});
onUnmounted(()=>{disposed=true;unlisteners.splice(0).forEach(fn=>fn());});
</script>
<template>
<main class="important-alerts" :style="style">
  <header @mousedown.left.prevent="drag"><b>重要操作提醒</b><span>{{ snapshot.entries.length }} 条待确认</span></header>
  <p v-if="error" class="error" role="alert">{{ error }}</p>
  <ol aria-label="待确认的重要提醒" aria-live="polite"><li v-for="entry in snapshot.entries" :key="entry.id" :data-notice-id="entry.id"><div class="entry-heading"><span>{{ labels[entry.notification_category] || '提醒' }}</span><time>{{ new Date(entry.received_at).toLocaleTimeString('zh-CN',{hour12:false}) }}</time></div><strong>{{ entry.title }}</strong><p>{{ entry.body }}</p><div class="actions"><button v-if="!entry.id.startsWith('test-')" type="button" :disabled="pending.has(entry.id)" @click="act('view_important_alert',entry.id)">查看记录 / 证据</button><button type="button" :disabled="pending.has(entry.id)" @click="act('dismiss_important_alert',entry.id)">确认已读</button></div></li></ol>
  <p v-if="!snapshot.entries.length" class="empty">暂无待确认提醒</p>
  <WindowResizeHandle @error="error=$event" />
</main>
</template>
<style>html,body,#app{width:100%;height:100%;margin:0;overflow:hidden;background:transparent}*{box-sizing:border-box}.important-alerts{position:relative;display:flex;flex-direction:column;width:100%;height:100%;padding:12px 14px 20px;border:1px solid currentColor;border-radius:8px;font:13px/1.6 var(--font-sans)}.important-alerts header{display:flex;justify-content:space-between;gap:12px;cursor:grab;user-select:none;padding-bottom:8px;border-bottom:1px solid color-mix(in srgb,currentColor 25%,transparent)}.important-alerts header b{font-size:14px}.important-alerts header span{opacity:.8}.important-alerts ol{list-style:none;overflow:auto;min-height:0;margin:0;padding:0;flex:1}.important-alerts li{padding:12px 0;border-bottom:1px solid color-mix(in srgb,currentColor 20%,transparent)}.important-alerts .entry-heading{display:flex;justify-content:space-between;gap:10px;font-size:12px;opacity:.8}.important-alerts strong{display:block;margin:5px 0}.important-alerts li p{margin:5px 0 10px;white-space:pre-wrap;overflow-wrap:anywhere}.important-alerts .actions{display:flex;gap:8px;flex-wrap:wrap}.important-alerts .actions button{border:1px solid currentColor;border-radius:5px;padding:4px 8px;font:inherit;cursor:pointer;background:transparent;color:inherit}.important-alerts button:focus-visible{outline:2px solid currentColor;outline-offset:2px}.important-alerts button:disabled{opacity:.5;cursor:wait}.important-alerts .error{color:#ffb3b3;overflow-wrap:anywhere;margin:6px 0}.important-alerts .empty{text-align:center;opacity:.8}</style>
