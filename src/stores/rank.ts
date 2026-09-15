// src/stores/rank.ts
// 推荐榜状态管理。

import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { MarketFilter } from '@/types/universe';
import type { RankResponse } from '@/types/rank';

export const useRankStore = defineStore('rank', () => {
  const result = ref<RankResponse | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const visible = ref(false);
  const activeFilter = ref<MarketFilter | null>(null);
  let opening = false;
  let openingTimer: ReturnType<typeof setTimeout> | null = null;

  // 竞态保护
  let generation = 0;

  async function scan(filter: MarketFilter, topN = 30, forceRefresh = false) {
    const request = ++generation;
    loading.value = true;
    error.value = null;
    try {
      const resp = await invoke<RankResponse>('scan_and_rank', {
        filter,
        topN,
        forceRefresh,
      });
      if (request !== generation) return;
      result.value = resp;
    } catch (e) {
      if (request === generation) {
        result.value = null;
        error.value = `生成推荐榜失败：${e}`;
      }
    } finally {
      if (request === generation) loading.value = false;
    }
  }

  function reset() {
    result.value = null;
    error.value = null;
  }

  function open(filter: MarketFilter) {
    // 保留点击时的条件快照，避免用户随后改筛选条件影响正在生成的榜单。
    activeFilter.value = { ...filter, boards: [...filter.boards] };
    result.value = null;
    error.value = null;
    loading.value = true;
    opening = true;
    visible.value = true;
    if (openingTimer) clearTimeout(openingTimer);
    openingTimer = setTimeout(() => {
      opening = false;
      openingTimer = null;
    }, 250);
  }

  function setVisible(next: boolean) {
    if (!next && opening) return;
    visible.value = next;
  }

  return { result, loading, error, visible, activeFilter, scan, reset, open, setVisible };
});
