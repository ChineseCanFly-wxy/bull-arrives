<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

interface ToastPayload {
  id: string;
  title: string;
  body: string;
}

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
:root { color-scheme: dark; font-family: system-ui, "Microsoft YaHei", sans-serif; background: transparent; }
* { box-sizing: border-box; }
html, body, #app { width: 100%; height: 100%; margin: 0; overflow: hidden; background: transparent; }
.toast { position: absolute; inset: 6px; display: grid; grid-template-columns: 38px 1fr 24px; gap: 12px; padding: 16px; border: 1px solid rgba(120, 170, 255, .42); border-radius: 14px; background: rgba(17, 24, 33, .97); color: #eef5ff; box-shadow: 0 12px 30px rgba(0, 0, 0, .36); }
.mark { display: grid; place-items: center; width: 38px; height: 38px; border-radius: 10px; background: #1677ff; font-weight: 800; }
strong { display: block; padding-right: 4px; font-size: 14px; }
p { margin: 7px 0 10px; color: #b9c5d2; font-size: 12px; line-height: 1.45; }
button { border: 0; cursor: pointer; }
section button { padding: 4px 12px; border-radius: 6px; background: #1677ff; color: white; }
.close { align-self: start; background: transparent; color: #92a0af; font-size: 22px; line-height: 18px; }
</style>
