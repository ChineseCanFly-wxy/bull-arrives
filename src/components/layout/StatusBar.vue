<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { useSettingsStore } from '@/stores/settings';
import { useUpdaterStore } from '@/stores/updater';
import { useUpdateCheck } from '@/composables/useUpdateCheck';
import { getVersion } from '@tauri-apps/api/app';
import { openUrl } from '@tauri-apps/plugin-opener';

const settings = useSettingsStore();
const updater = useUpdaterStore();
const { manualCheck } = useUpdateCheck();
const appVersion = ref('');
withDefaults(defineProps<{ copyright?: string }>(), { copyright: '© 2026 ChineseCanFly-wxy' });

onMounted(async () => {
  try { appVersion.value = await getVersion(); }
  catch { appVersion.value = ''; }
});

async function openRepository() {
  try {
    await openUrl('https://github.com/ChineseCanFly-wxy/bull-arrives');
  } catch (error) {
    console.error('[StatusBar] Failed to open GitHub repository:', error);
  }
}
</script>

<template>
  <footer class="status-bar">
    <div class="sb-zone sb-info">
      <span v-if="appVersion" class="sb-version">v{{ appVersion }}</span>
      <button
        v-if="!settings.isPortable"
        class="sb-check-btn"
        :class="{ 'sb-up-to-date': updater.isUpToDate }"
        :disabled="updater.updateStatus === 'checking'"
        @click="manualCheck"
      >
        {{ updater.updateStatus === 'checking' ? '检查中...' : updater.isUpToDate ? '已是最新版本' : '检查更新' }}
      </button>
      <span class="sb-sep">·</span>
      <button class="sb-github" title="GitHub 仓库" aria-label="在浏览器中打开 Bull Arrives GitHub 仓库" @click="openRepository">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="currentColor" aria-hidden="true"><path d="M12 .7a11.5 11.5 0 0 0-3.64 22.41c.58.11.79-.25.79-.56v-2.22c-3.22.7-3.9-1.37-3.9-1.37-.53-1.34-1.29-1.7-1.29-1.7-1.05-.72.08-.71.08-.71 1.16.08 1.78 1.2 1.78 1.2 1.04 1.77 2.72 1.26 3.38.96.1-.75.4-1.26.74-1.55-2.57-.29-5.27-1.28-5.27-5.69 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.16 1.18a10.9 10.9 0 0 1 5.76 0c2.19-1.49 3.15-1.18 3.15-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.42-2.71 5.39-5.29 5.68.42.36.79 1.07.79 2.16v3.21c0 .31.21.68.8.56A11.5 11.5 0 0 0 12 .7Z"/></svg>
        <span>GitHub</span>
      </button>
      <span class="sb-sep">·</span>
      <span class="sb-copyright">{{ copyright }}</span>
    </div>
  </footer>
</template>

<style scoped>
.status-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  height: 28px;
  padding: 0 var(--space-4);
  background: var(--color-surface-1);
  border-top: 1px solid var(--color-border-0);
  flex-shrink: 0;
  font-size: var(--text-xs);
  line-height: 1;
  color: var(--color-text-tertiary);
}
.sb-zone { display: flex; align-items: center; height: 100%; gap: var(--space-2); }
.sb-sep { color: var(--color-border-1); user-select: none; font-weight: var(--font-weight-bold); line-height: 1; }
.sb-version { font-weight: var(--font-weight-medium); color: var(--color-accent); font-family: var(--font-mono); line-height: 1; }
.sb-check-btn {
  display: inline-flex;
  align-items: center;
  padding: 1px 6px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  line-height: 1;
  font-family: var(--font-sans);
  cursor: pointer;
  transition: color var(--transition-fast), background var(--transition-fast);
}
.sb-check-btn:hover:not(:disabled) { color: var(--color-accent); background: var(--color-bg-elevated); }
.sb-check-btn:disabled { opacity: 0.6; cursor: not-allowed; }
.sb-github { display: inline-flex; align-items: center; gap: 4px; padding: 2px 5px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--color-text-tertiary); font: inherit; cursor: pointer; }
.sb-github:hover { color: var(--color-accent); background: var(--color-bg-elevated); }
.sb-github:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 1px; }
.sb-github svg { flex: 0 0 auto; }
.sb-check-btn.sb-up-to-date { color: #3fb950; }
.sb-copyright { color: var(--color-text-tertiary); line-height: 1; }
@media (max-width: 680px) {
  .sb-copyright, .sb-copyright + * { display: none; }
  .status-bar { padding-inline: var(--space-2); }
}
</style>
