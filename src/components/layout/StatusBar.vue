<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { useSettingsStore } from '@/stores/settings';
import { useUpdaterStore } from '@/stores/updater';
import { useUpdateCheck } from '@/composables/useUpdateCheck';
import { getVersion } from '@tauri-apps/api/app';

const settings = useSettingsStore();
const updater = useUpdaterStore();
const { manualCheck } = useUpdateCheck();
const appVersion = ref('');
withDefaults(defineProps<{ copyright?: string }>(), { copyright: '© 2026 ChineseCanFly-wxy' });

onMounted(async () => {
  try { appVersion.value = await getVersion(); }
  catch { appVersion.value = ''; }
});
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
.sb-check-btn.sb-up-to-date { color: #3fb950; }
.sb-copyright { color: var(--color-text-tertiary); line-height: 1; }
</style>
