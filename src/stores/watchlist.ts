// src/stores/watchlist.ts
import { defineStore } from 'pinia';
import { ref } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { WatchItem } from '@/types';

export const useWatchlistStore = defineStore('watchlist', () => {
  const items = ref<WatchItem[]>([]);
  const groups = ref<{ id: number; name: string; sort_order: number; item_count: number }[]>([]);
  const activeGroupId = ref(0);
  let generation = 0;
  let listening: Promise<UnlistenFn> | undefined;

  async function startListening() {
    listening ??= listen('watchlist-changed', () => { void fetchWatchlist(); }).catch(error => {
      listening = undefined;
      throw error;
    });
    await listening;
  }

  async function selectGroup(id: number) {
    await invoke('select_watch_group', { id });
    await fetchWatchlist();
  }
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function fetchWatchlist() {
    const request = ++generation;
    loading.value = true;
    error.value = null;
    try {
      await startListening();
      const snapshot = await invoke<{ active_group_id: number; groups: typeof groups.value; items: WatchItem[] }>('get_group_snapshot');
      if (request !== generation) return;
      activeGroupId.value = snapshot.active_group_id;
      groups.value = snapshot.groups;
      items.value = snapshot.items;
    } catch (e) {
      if (request === generation) error.value = `获取自选列表失败: ${e}`;
      console.error('Failed to fetch watchlist:', e);
    } finally {
      if (request === generation) loading.value = false;
    }
  }

  async function addStock(code: string, market: string, name: string) {
    error.value = null;
    try {
      await invoke('add_watch', { code, market, name, groupId: activeGroupId.value });
      await fetchWatchlist();
    } catch (e) {
      error.value = `添加失败: ${e}`;
      console.error('[watchlist] addStock failed:', e);
      throw e;
    }
  }

  async function removeStock(code: string, market: string) {
    error.value = null;
    try {
      const item = items.value.find(item => item.code === code && item.market === market);
      if (activeGroupId.value !== 0 && item) {
        await invoke('set_group_member', { groupId: activeGroupId.value, watchId: item.id, included: false });
      } else {
        await invoke('remove_watch', { code, market });
      }
      await fetchWatchlist();
    } catch (e) {
      error.value = `删除失败: ${e}`;
      console.error('[watchlist] removeStock failed:', e);
      throw e;
    }
  }

  return { items, groups, activeGroupId, selectGroup, loading, error, fetchWatchlist, addStock, removeStock };
});
