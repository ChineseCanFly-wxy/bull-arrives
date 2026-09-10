<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NButton, NInputNumber, NSwitch, useMessage } from 'naive-ui';
import { useSettingsStore } from '@/stores/settings';
const settings = useSettingsStore();
const message = useMessage();
const width = ref<number | null>(1000);
const height = ref<number | null>(680);
const remember = ref(false);
const busy = ref(false);
onMounted(() => {
  try {
    const raw = settings.settings.main_window_size;
    if (raw) { const value = JSON.parse(raw); width.value = value.width; height.value = value.height; remember.value = value.remember; }
  } catch { message.warning('窗口尺寸配置无效，将使用默认大小。'); }
});
async function save() {
  if (width.value === null || height.value === null) { message.warning('请填写宽度和高度'); return; }
  busy.value = true;
  try {
    const config = { width: width.value, height: height.value, remember: remember.value };
    await invoke('set_main_window_size', { config });
    settings.settings.main_window_size = JSON.stringify(config);
    message.success('打开大小已保存并应用');
  } catch (error) { message.error(String(error)); }
  finally { busy.value = false; }
}
</script>

<template>
  <section class="window-size-section">
    <h3>主窗口打开大小</h3>
    <p>默认 1000 × 680，单位为逻辑像素；超出屏幕时自动缩小。</p>
    <div class="size-inputs">
      <label>宽度<NInputNumber v-model:value="width" :min="640" :max="7680" :precision="0" :disabled="busy" /></label>
      <label>高度<NInputNumber v-model:value="height" :min="480" :max="4320" :precision="0" :disabled="busy" /></label>
    </div>
    <label class="remember-row"><span>记住上次窗口大小（否则每次打开使用固定大小）</span><NSwitch v-model:value="remember" :disabled="busy" /></label>
    <NButton size="small" type="primary" :loading="busy" @click="save">保存并应用</NButton>
  </section>
</template>

<style scoped>
.window-size-section { padding: 16px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); }
h3 { margin: 0 0 8px; font-size: 14px; }
p { color: var(--color-text-secondary); font-size: 12px; }
.size-inputs { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.size-inputs label { display: grid; gap: 6px; font-size: 13px; }
.remember-row { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin: 16px 0; font-size: 12px; }
</style>
