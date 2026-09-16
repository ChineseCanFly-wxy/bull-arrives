// src/stores/monitor.ts
// 个股监控（量化自动止损/止盈）状态管理。

import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Monitor } from '@/types/monitor';

export const useMonitorStore = defineStore('monitor', () => {
  const monitors = ref<Monitor[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const hasLoaded = ref(false);

  async function fetchMonitors() {
    loading.value = true;
    error.value = null;
    try {
      monitors.value = await invoke<Monitor[]>('get_monitors');
      hasLoaded.value = true;
    } catch (e) {
      error.value = `加载监控失败：${e}`;
    } finally {
      loading.value = false;
    }
  }

  /** 开启（或刷新）一只股票的量化监控。后端自动算止损/止盈。 */
  async function save(code: string, market: string, name: string): Promise<Monitor | null> {
    error.value = null;
    try {
      const monitor = await invoke<Monitor>('save_monitor', { code, market, name });
      await fetchMonitors();
      return monitor;
    } catch (e) {
      error.value = `开启监控失败：${e}`;
      return null;
    }
  }

  async function remove(code: string, market: string): Promise<boolean> {
    try {
      await invoke('delete_monitor', { code, market });
      monitors.value = monitors.value.filter(m => !(m.code === code && m.market === market));
      return true;
    } catch (e) {
      error.value = `删除监控失败：${e}`;
      return false;
    }
  }

  function resetError() {
    error.value = null;
  }

  return { monitors, loading, error, hasLoaded, fetchMonitors, save, remove, resetError };
});
