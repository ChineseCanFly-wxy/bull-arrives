<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '@/stores/settings';
interface Delivery { native: string; native_error?: string; desktop: string; desktop_error?: string }
interface Identity { supported: boolean; registered: boolean; detail: string }
const settings = useSettingsStore();
const enabled = computed(() => settings.researchNotificationsEnabled ?? true);
const panel = ref<HTMLDetailsElement | null>(null);
const root = ref<HTMLElement | null>(null); const busy = ref(''); const error = ref(''); const result = ref<Delivery | null>(null);
const identity = ref<Identity | null>(null); const expanded = ref(false);
async function toggle() {
  if (busy.value) return; busy.value = 'save'; error.value = '';
  try { if (!await settings.setSetting('research_notifications_enabled', enabled.value ? '0' : '1')) error.value = settings.error || '研究提醒保存失败，请重试。'; }
  catch (e) { error.value = String(e); } finally { busy.value = ''; }
}
async function test() {
  if (busy.value) return; busy.value = 'test'; error.value = ''; result.value = null;
  try { result.value = await invoke<Delivery>('test_notification', { research: true }); }
  catch (e) { error.value = String(e); } finally { busy.value = ''; }
}
async function inspectIdentity() {
  if (busy.value) return; expanded.value = !expanded.value;
  if (!expanded.value) return; busy.value = 'identity'; error.value = '';
  try { identity.value = await invoke<Identity>('get_notification_identity_status'); }
  catch (e) { error.value = String(e); } finally { busy.value = ''; }
}
async function register() {
  if (busy.value) return; busy.value = 'register'; error.value = '';
  try { identity.value = await invoke<Identity>('register_notification_identity'); }
  catch (e) { error.value = String(e); } finally { busy.value = ''; }
}
function expand() { if (panel.value) panel.value.open = true; }
async function focus() { expand(); await nextTick(); root.value?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); root.value?.querySelector<HTMLButtonElement>('[role=switch]')?.focus({ preventScroll: true }); }
defineExpose({ expand, focus });
</script>
<template>
  <section ref="root" class="research-notifications" aria-label="研究提醒设置">
    <div class="research-notification-row">
      <div><b>研究提醒</b><span>{{ enabled ? '已开启' : '已暂停' }} · 统一控制自动成交、手动条件和主线消息</span></div>
      <button class="research-notification-switch" role="switch" aria-label="开启全研究提醒" :aria-checked="enabled" :disabled="!!busy" @click="toggle">{{ enabled ? '暂停提醒' : '恢复提醒' }}</button>
    </div>
    <details ref="panel" class="research-notification-details">
      <summary>通知测试与帮助（可选）</summary>
      <p>先点“测试研究提醒”，检查能否收到一次测试消息；未收到时再点“通知帮助”，查看系统返回状态及通知身份，按需启用或修复 Windows 通知。这些操作只测试消息，不运行研究或买卖。</p>
      <div class="research-notification-tools"><button :disabled="!!busy" @click="test">{{ busy === 'test' ? '测试中…' : '测试研究提醒' }}</button><button :disabled="!!busy" :aria-expanded="expanded" @click="inspectIdentity">通知帮助</button></div>
      <p>暂停提醒后，自动买卖和条件检查继续运行。要停止某个模型，请到其账户点击“暂停自动买卖”，或删除该模型的自动账户。</p>
      <p>信号表示发现候选，委托表示订单已提交；成交提醒才表示已模拟买入或卖出，包含股数与成交价。</p>
      <p v-if="result" role="status">系统通知：{{ result.native === 'accepted' ? '已受理' : '失败（' + (result.native_error || '未知原因') + '）' }}；桌面提醒：{{ result.desktop === 'queued' ? '已排队' : '失败（' + (result.desktop_error || result.desktop) + '）' }}。测试仅发送一次通知，不操作账户。</p>
      <div v-if="expanded" class="research-notification-help"><p>研究提醒同时请求系统通知和应用桌面弹窗。系统已受理不代表横幅一定可见，应用需保持运行。</p><p v-if="identity">{{ identity.detail }}</p><button v-if="identity?.supported" :disabled="!!busy" @click="register">{{ identity.registered ? '修复 Windows 通知身份' : '启用 Windows 通知' }}</button></div>
    </details>
    <p v-if="error" class="research-notification-error" role="alert">{{ error }}</p>
  </section>
</template>
<style scoped>
.research-notifications{border:1px solid var(--color-border-0);border-radius:10px;padding:12px;margin:0 0 12px;color:var(--color-text-primary);background:var(--color-surface-1);font-size:12px;min-width:0}.research-notification-details{margin-top:9px}.research-notification-details>summary{cursor:pointer;line-height:1.6;color:var(--color-text-secondary);font-size:var(--text-xs);overflow-wrap:anywhere}.research-notification-details[open]>summary{margin-bottom:10px}.research-notification-row,.research-notification-tools{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.research-notification-row>div{flex:1;min-width:160px}.research-notification-row b{font-size:13px}.research-notification-row span{display:block;margin-top:4px;color:var(--color-text-secondary);line-height:1.6}.research-notifications button{border:1px solid var(--color-border-0);border-radius:7px;padding:6px 10px;background:var(--color-surface-1);color:var(--color-text-primary);font-size:12px;cursor:pointer}.research-notifications button:disabled{opacity:.55;cursor:default}.research-notifications [aria-checked="true"]{border-color:var(--color-accent);color:var(--color-accent);background:var(--color-accent-dim)}.research-notifications p{margin:8px 0 0;line-height:1.6;overflow-wrap:anywhere;color:var(--color-text-secondary)}.research-notifications .research-notification-error{color:var(--color-error)}.research-notification-help{padding-top:4px}.research-notification-help button{margin-top:8px}
</style>
