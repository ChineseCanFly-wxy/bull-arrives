<script setup lang="ts">
import { computed, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NSwitch, NButton } from 'naive-ui';
import { useSettingsStore } from '@/stores/settings';
const props = withDefaults(defineProps<{ category: string; label: string; settingKey: string; description: string; defaultValue?: string }>(), { defaultValue: '1' });
const settings = useSettingsStore();
const enabled = computed(() => (settings.settings[props.settingKey] ?? props.defaultValue) === '1');
const busy = ref(false); const error = ref(''); const result = ref('');
async function toggle(value: boolean) {
  busy.value = true; error.value = ''; result.value = '';
  try { if (!await settings.setSetting(props.settingKey, value ? '1' : '0')) error.value = settings.error || '保存失败'; }
  finally { busy.value = false; }
}
async function toggleNewsSystem(value: boolean) {
  busy.value = true; error.value = '';
  try { if (!await settings.setSetting('news_system_notifications_enabled', value ? '1' : '0')) error.value = settings.error || '保存失败'; }
  finally { busy.value = false; }
}
async function test() {
  busy.value = true; error.value = ''; result.value = '';
  try {
    const status = await invoke<{ native: string; native_error?: string; desktop: string; desktop_error?: string }>('test_notification', { category: props.category });
    result.value = '系统通知：' + (status.native === 'accepted' ? '已受理' : status.native === 'not-requested' ? '未请求' : status.native_error || '失败') + '；桌面提醒：' + (status.desktop === 'queued' ? '已投递' : status.desktop_error || status.desktop);
  } catch (e) { error.value = String(e); } finally { busy.value = false; }
}
</script>
<template>
<section class="reminder-controls" :aria-label="label + '控制'">
  <div class="control-row"><div><b>{{ label }}</b><p>{{ description }}</p></div><NSwitch :value="enabled" :disabled="busy" :loading="busy" :aria-label="label + '开关'" @update:value="toggle" /></div>
  <div v-if="category === 'news'" class="control-row"><span>资讯使用 Windows 系统通知（关闭后仍采集和保存）</span><NSwitch :value="settings.settings.news_system_notifications_enabled !== '0'" :disabled="busy" aria-label="资讯系统通知" @update:value="toggleNewsSystem" /></div>
  <div class="control-status"><span>{{ enabled ? '已开启' : '已关闭' }} · 此开关只控制本类</span><NButton v-if="category !== 'briefs'" size="small" :disabled="busy" @click="test">测试当前提醒</NButton></div>
  <p v-if="result" role="status">{{ result }}。测试消息不操作账户。</p><p v-if="error" class="error" role="alert">{{ error }}</p>
</section>
</template>
<style scoped>.reminder-controls{padding:14px;margin:14px 0;border:1px solid var(--color-border-0);border-radius:8px;background:var(--color-surface-1);font-size:13px}.control-row,.control-status{display:flex;justify-content:space-between;align-items:center;gap:16px}.control-row+ .control-row{margin-top:10px}.control-row b{font-size:15px}.control-row p{margin:6px 0;line-height:1.7;color:var(--color-text-secondary)}.control-status{margin-top:10px;color:var(--color-text-secondary)}.error{color:var(--color-error)}</style>
