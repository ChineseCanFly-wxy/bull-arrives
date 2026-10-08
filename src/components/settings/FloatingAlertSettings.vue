<script setup lang="ts">
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NSwitch, NButton } from 'naive-ui';
import { useSettingsStore } from '@/stores/settings';
const settings = useSettingsStore();
const busy = ref(false); const error = ref(''); const result = ref('');
const identity = ref<{supported: boolean; registered: boolean; detail: string} | null>(null);
async function save(key: string, value: string) {
  busy.value = true; error.value = '';
  try { if (!await settings.setSetting(key,value)) error.value = settings.error || '保存失败'; }
  finally { busy.value = false; }
}
async function tickerOpacity(event: Event) {
  const input = event.target as HTMLInputElement; busy.value = true; error.value = '';
  try { if (!await settings.setTickerOpacity(Number(input.value))) error.value = settings.error || '保存失败'; }
  finally { input.value = String(settings.tickerOpacity); busy.value = false; }
}
function change(key: string, event: Event) { void save(key, (event.target as HTMLInputElement).value); }
async function test() {
  busy.value = true; error.value = '';
  try { await invoke('test_notification', { category: 'floating' }); result.value = '已投递测试提醒；不操作账户。'; }
  catch (e) { error.value = String(e); } finally { busy.value = false; }
}
async function help(register = false) {
  busy.value = true; error.value = '';
  try { identity.value = await invoke(register ? 'register_notification_identity' : 'get_notification_identity_status'); }
  catch (e) { error.value = String(e); } finally { busy.value = false; }
}
</script>
<template>
<section class="floating-settings" aria-label="悬浮提醒设置">
  <div class="row"><div><b>重要操作提醒悬浮窗</b><p>只显示模拟买卖、模型条件、盘中确认、市场主线和行情/风险提醒。置顶、逐条保留，确认已读后移除；关闭只隐藏窗口，任务继续运行。</p></div><NSwitch :value="settings.settings.important_alerts_enabled !== '0'" :disabled="busy" aria-label="重要操作提醒悬浮窗" @update:value="save('important_alerts_enabled',$event?'1':'0')" /></div>
  <div class="row"><span>重要提醒也发送 Windows 系统通知</span><NSwitch :value="settings.settings.important_alerts_native_enabled === '1'" :disabled="busy" aria-label="重要提醒系统通知" @update:value="save('important_alerts_native_enabled',$event?'1':'0')" /></div>
  <div class="appearance-grid">
    <label>文字颜色<input type="color" :value="settings.settings.important_alerts_text_color || '#EEF5FF'" :disabled="busy" @change="change('important_alerts_text_color',$event)" /></label>
    <label>背景颜色<input type="color" :value="settings.settings.important_alerts_background || '#111821'" :disabled="busy" @change="change('important_alerts_background',$event)" /></label>
    <label>透明度 {{ settings.settings.important_alerts_opacity || '95' }}%<input type="range" min="5" max="100" :value="settings.settings.important_alerts_opacity || '95'" :disabled="busy" @change="change('important_alerts_opacity',$event)" /></label>
  </div>
  <p>拖动窗口标题移动；拖动右下角缩放，也可选中缩放按钮用方向键微调。位置与大小会保存。查看记录会打开对应分类和当时证据，不会确认已读或执行买卖。</p>
  <NButton size="small" :disabled="busy" @click="test">测试重要悬浮提醒</NButton>
  <details><summary>行情悬浮窗</summary><p>行情窗同样支持拖动与缩放，显示或隐藏快捷键：{{ settings.tickerHotkey }}。</p><div class="row"><span>行情文字使用单色</span><NSwitch :value="settings.tickerSingleColor" :disabled="busy" aria-label="行情悬浮窗单色" @update:value="save('ticker_single_color',$event?'1':'0')" /></div><div class="appearance-grid"><label>单色文字<input type="color" :value="settings.tickerTextColor" :disabled="busy" @change="change('ticker_text_color',$event)" /></label><label>透明度 {{ settings.tickerOpacity }}%<input type="range" min="5" max="100" :value="settings.tickerOpacity" :disabled="busy" @change="tickerOpacity" /></label></div></details>
  <details><summary @click="help()">普通通知与 Windows 通知帮助</summary><p>普通通知优先使用系统通知，发送失败时尝试桌面弹窗。系统已受理不代表横幅一定可见，应用需保持运行。</p><div class="row"><span>普通通知同时显示桌面弹窗</span><NSwitch :value="settings.notificationDesktopAlways" :disabled="busy" aria-label="普通通知同时显示桌面弹窗" @update:value="save('notification_desktop_always',$event?'1':'0')" /></div><p v-if="identity">{{ identity.detail }}</p><NButton v-if="identity?.supported" size="small" :disabled="busy" @click="help(true)">{{ identity.registered ? '修复 Windows 通知身份' : '启用 Windows 通知' }}</NButton></details>
  <p v-if="result" role="status">{{ result }}</p><p v-if="error" class="error" role="alert">{{ error }}</p>
</section>
</template>
<style scoped>.floating-settings{font-size:13px;line-height:1.7;margin-top:16px}.row{display:flex;justify-content:space-between;align-items:center;gap:20px;margin:14px 0}.row b{font-size:15px}.floating-settings p{color:var(--color-text-secondary);margin:6px 0}.appearance-grid{display:flex;flex-wrap:wrap;gap:22px;margin:14px 0}.appearance-grid label{display:flex;align-items:center;gap:10px}.appearance-grid input[type=color]{width:42px;height:28px;padding:0;border:1px solid var(--color-border-0);cursor:pointer}.appearance-grid input[type=range]{max-width:150px}.floating-settings details{margin-top:18px;border-top:1px solid var(--color-border-0);padding-top:12px}.floating-settings summary{cursor:pointer;font-weight:600}.floating-settings .error{color:var(--color-error)}</style>
