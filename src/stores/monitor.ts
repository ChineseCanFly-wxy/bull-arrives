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

  /**
   * 开启（或刷新）一只股票的量化监控。后端自动算止损/止盈。
   *
   * `enabled` 的三种取值（对应后端 `Option<bool>`）：
   * - `true` —— 「开启监控」：明确要开，对已停止的也一并启动；
   * - `null` —— 「刷新」：重算价位但**保持原有的启停状态**，
   *   不能顺手把用户停掉的监控偷偷打开；
   * - `false` —— 保留给需要显式停用又同时重算价位的场景（目前 UI 没用到）。
   */
  async function save(
    code: string,
    market: string,
    name: string,
    enabled: boolean | null = true,
  ): Promise<Monitor | null> {
    error.value = null;
    try {
      const monitor = await invoke<Monitor>('save_monitor', { code, market, name, enabled });
      await fetchMonitors();
      return monitor;
    } catch (e) {
      error.value = `开启监控失败：${e}`;
      return null;
    }
  }

  /**
   * 停止 / 恢复一条监控。
   *
   * 「停止」只翻 `enabled` 开关 —— 参考价、止损/止盈位、触发状态都留着，
   * 恢复后立刻按原价位继续判断。与「删除」是两件事（见后端 `commands/monitor.rs`）。
   */
  async function setEnabled(code: string, market: string, enabled: boolean): Promise<boolean> {
    error.value = null;
    try {
      await invoke('set_monitor_enabled', { code, market, enabled });
      // 就地改，不整表重拉：列表很短，但重拉会让行闪一下
      const target = monitors.value.find(m => m.code === code && m.market === market);
      if (target) target.enabled = enabled;
      return true;
    } catch (e) {
      error.value = `${enabled ? '恢复' : '停止'}监控失败：${e}`;
      return false;
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

  return { monitors, loading, error, hasLoaded, fetchMonitors, save, setEnabled, remove, resetError };
});
