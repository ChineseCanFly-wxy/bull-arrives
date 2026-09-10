<script setup lang="ts">
import { computed, ref, defineAsyncComponent } from 'vue';
import { useSettingsStore } from '@/stores/settings';
import { NIcon, NDropdown } from 'naive-ui';
const SettingsDialog = defineAsyncComponent(() => import('@/components/settings/SettingsDialog.vue'));

const settings = useSettingsStore();

const dsDisplayName = computed(() => {
  const found = settings.datasources.find(([id]) => id === settings.activeDatasource);
  return found ? found[1] : settings.activeDatasource;
});

const dsOptions = computed(() =>
  settings.datasources.map(([id, name]) => ({
    label: name,
    key: id,
  }))
);

function handleDsSelect(key: string) {
  settings.switchDatasource(key);
}

const showSettings = ref(false);
function openSettings() {
  showSettings.value = true;
}
</script>

<template>
  <header class="top-bar">
    <!-- Slogan -->
    <span class="brand-slogan">实时行情 · 多源切换 · 免费高效</span>

    <!-- Right: data source switcher + settings cog -->
    <div class="top-bar-right">
      <n-dropdown
        trigger="click"
        :options="dsOptions"
        @select="handleDsSelect"
      >
        <span class="ds-tag" title="点击切换数据源">
          <span class="ds-label">{{ dsDisplayName }}</span>
          <n-icon :size="12" class="ds-swap-icon">
            <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M7 3 3 7l4 4" />
              <path d="M17 11v1a4 4 0 0 1-4 4H3" />
              <path d="M13 17 17 13l-4-4" />
              <path d="M3 9V8a4 4 0 0 1 4-4h10" />
            </svg>
          </n-icon>
        </span>
      </n-dropdown>

      <button
        class="cog-btn"
        :aria-label="`打开设置 (${settings.tickerHotkey})`"
        :title="`设置 (悬浮窗快捷键: ${settings.tickerHotkey})`"
        @click="openSettings"
      >
        <svg viewBox="0 0 20 20" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="10" cy="10" r="2.4" />
          <path d="M16.4 12.4l1.3 1.3a1 1 0 0 1 0 1.4l-1.1 1.1a1 1 0 0 1-1.4 0l-1.3-1.3a6.5 6.5 0 0 1-1.6.7l-.2 1.7a1 1 0 0 1-1 .9h-2.2a1 1 0 0 1-1-.9l-.2-1.7a6.5 6.5 0 0 1-1.6-.7L5 15.6a1 1 0 0 1-1.4 0L2.5 14.5a1 1 0 0 1 0-1.4l1.3-1.3a6.5 6.5 0 0 1-.7-1.6L1.4 10a1 1 0 0 1-.9-1V6.8a1 1 0 0 1 .9-1l1.7-.2a6.5 6.5 0 0 1 .7-1.6L2.5 2.7a1 1 0 0 1 0-1.4L3.6.2a1 1 0 0 1 1.4 0L6.3 1.5a6.5 6.5 0 0 1 1.6-.7l.2-1.7a1 1 0 0 1 1-.9h2.2a1 1 0 0 1 1 .9l.2 1.7a6.5 6.5 0 0 1 1.6.7l1.3-1.3a1 1 0 0 1 1.4 0l1.1 1.1a1 1 0 0 1 0 1.4l-1.3 1.3a6.5 6.5 0 0 1 .7 1.6l1.7.2a1 1 0 0 1 .9 1V9a1 1 0 0 1-.9 1l-1.7.2a6.5 6.5 0 0 1-.7 1.6Z" />
        </svg>
      </button>
    </div>

    <SettingsDialog v-if="showSettings" v-model:show="showSettings" />
  </header>
</template>

<style scoped>
.top-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: var(--header-height);
  padding: 0 var(--space-4);
  background: var(--color-surface-1);
  border-bottom: 1px solid var(--color-border-0);
  flex-shrink: 0;
  -webkit-app-region: drag;
}

.top-bar-right {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  -webkit-app-region: no-drag;
}

/* ── Slogan ── */
.brand-slogan {
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
  white-space: nowrap;
  letter-spacing: 0.02em;
}

/* ── Data source tag ── */
.ds-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 20px;
  padding: 0 var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-accent-dim);
  color: var(--color-accent);
  font-size: var(--text-xs);
  cursor: pointer;
  user-select: none;
  transition: background var(--transition-fast), color var(--transition-fast);
  -webkit-app-region: no-drag;
}
.ds-tag:hover {
  filter: brightness(1.4);
}
.ds-label {
  line-height: 1;
}
.ds-swap-icon {
  color: inherit;
  flex-shrink: 0;
}

/* ── Settings cog button ── */
.cog-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-tertiary);
  cursor: pointer;
  transition: color var(--transition-fast), background var(--transition-fast), border-color var(--transition-fast);
}
.cog-btn:hover {
  color: var(--color-accent);
  background: var(--color-accent-dim);
  border-color: var(--color-accent-dim);
}
</style>
