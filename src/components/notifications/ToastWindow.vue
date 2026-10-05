<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useSettingsStore, SETTING_CHANGED_EVENT, type SettingChangedPayload } from '@/stores/settings';

interface ToastPayload {
  id: string;
  title: string;
  body: string;
}

const settings = useSettingsStore();
const current = ref<ToastPayload | null>(null);
const unlisteners: UnlistenFn[] = [];
let timer: number | undefined;

function display(payload: ToastPayload) {
  current.value = payload;
  window.clearTimeout(timer);
  const id = payload.id;
  timer = window.setTimeout(() => dismiss(id), 10_000);
}

async function dismiss(id: string) {
  if (current.value?.id !== id) return;
  current.value = null;
  await invoke('dismiss_desktop_toast', { id });
}

async function view() {
  const id = current.value?.id;
  if (!id) return;
  window.clearTimeout(timer);
  current.value = null;
  await invoke('view_desktop_toast', { id });
}

onMounted(async () => {
  unlisteners.push(
    await listen<ToastPayload>('desktop-toast-show', ({ payload }) => display(payload)),
  );
  await invoke('desktop_toast_ready');
  unlisteners.push(await listen<SettingChangedPayload>(SETTING_CHANGED_EVENT, ({ payload }) => {
    settings.applyRemoteSetting(payload.key, payload.value);
  }));
  await settings.fetchSettings();
});

onUnmounted(() => {
  window.clearTimeout(timer);
  unlisteners.forEach((unlisten) => unlisten());
});
</script>

<template>
  <main v-if="current" class="toast" role="status" aria-live="assertive">
    <div class="mark">Q</div>
    <section>
      <strong>{{ current.title }}</strong>
      <p>{{ current.body }}</p>
      <button @click="view">查看</button>
    </section>
    <button class="close" aria-label="关闭" @click="dismiss(current.id)">×</button>
  </main>
</template>

<style>
:root { color-scheme: dark; font-family: var(--font-sans); background: transparent; }
* { box-sizing: border-box; }
html, body, #app { width: 100%; height: 100%; margin: 0; overflow: hidden; background: transparent; }
.toast { position: absolute; inset: 6px; display: grid; grid-template-columns: 38px 1fr 24px; gap: 12px; padding: 16px; border: 1px solid rgba(120, 170, 255, .42); border-radius: 14px; background: rgba(17, 24, 33, .97); color: #eef5ff; box-shadow: 0 12px 30px rgba(0, 0, 0, .36); }
.mark { display: grid; place-items: center; width: 38px; height: 38px; border-radius: 10px; background: #1677ff; font-weight: 800; }
strong { display: block; padding-right: 4px; font-size: 14px; }
section { min-width: 0; min-height: 0; display: flex; flex-direction: column; align-items: start; }
strong, p { overflow: hidden; overflow-wrap: anywhere; display: -webkit-box; -webkit-box-orient: vertical; }
strong { -webkit-line-clamp: 2; flex-shrink: 0; }
p { margin: 5px 0; color: #c3cfdb; font-size: 13px; line-height: 1.6; -webkit-line-clamp: 2; }
button { border: 0; font-size: var(--text-sm); font-family: inherit; cursor: pointer; }
section button { margin-top: auto; flex-shrink: 0; padding: 4px 12px; border-radius: 6px; background: #1677ff; color: white; }
.close { align-self: start; background: transparent; color: #92a0af; font-size: 22px; line-height: 18px; }
html[data-style="modern"][data-appearance="elegant"] .toast { font-family: var(--font-sans); border-color: var(--color-border-1); border-radius: var(--radius-lg); background: var(--color-surface-1); color: var(--color-text-primary); }
html[data-style="modern"][data-appearance="elegant"] .toast :is(.mark, section button) { background: var(--color-accent); color: var(--color-accent-contrast); }
html[data-style="modern"][data-appearance="elegant"] .toast p { font-size: 13px; line-height: 1.7; color: var(--color-text-secondary); }
html[data-style="modern"][data-appearance="elegant"] .toast .close { color: var(--color-text-secondary); }
</style>
