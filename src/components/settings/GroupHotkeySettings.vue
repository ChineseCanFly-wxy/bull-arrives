<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NButton, NInput, useMessage } from 'naive-ui';
import { useSettingsStore } from '@/stores/settings';
const settings = useSettingsStore();
const message = useMessage();
const previous = ref('');
const next = ref('');
const busy = ref(false);
onMounted(() => { previous.value = settings.settings.group_previous_hotkey || ''; next.value = settings.settings.group_next_hotkey || ''; });
async function save(direction: 'previous' | 'next') {
  busy.value = true;
  try {
    const hotkey = (direction === 'previous' ? previous.value : next.value).trim();
    await invoke('set_group_hotkey', { direction, hotkey });
    settings.settings[`group_${direction}_hotkey`] = hotkey;
    message.success('分组快捷键已保存');
  } catch (error) { message.error(`快捷键保存失败，旧快捷键保留：${String(error)}`); }
  finally { busy.value = false; }
}
</script>
<template>
  <section class="hotkey-settings">
    <h3>切换分组快捷键</h3>
    <p>全局生效，两窗口同步切换。输入例如 Ctrl+Alt+Left；留空保存可停用。</p>
    <label>上一分组<NInput v-model:value="previous" placeholder="Ctrl+Alt+Left" :disabled="busy" /><NButton size="small" :disabled="busy" @click="save('previous')">保存</NButton></label>
    <label>下一分组<NInput v-model:value="next" placeholder="Ctrl+Alt+Right" :disabled="busy" /><NButton size="small" :disabled="busy" @click="save('next')">保存</NButton></label>
  </section>
</template>
<style scoped>
.hotkey-settings { padding: 16px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); }
h3 { margin: 0; font-size: 14px; }
p { color: var(--color-text-secondary); font-size: 12px; line-height: 1.6; }
label { display: grid; grid-template-columns: auto minmax(0,1fr) auto; align-items:center; gap:8px; margin-top:10px; font-size:12px; }
</style>
