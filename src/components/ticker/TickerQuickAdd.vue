<script setup lang="ts">
// 快速自选面板：从悬浮条上的「＋」打开，不用切到主界面就能
//   ① 搜索并添加自选股  ② 直接删掉自选股
// 典型用法是「临时看一只票 → 看完就删」。
//
// 注意：本窗口是独立 WebView，和主界面 / 悬浮窗各自持有独立的 Pinia 实例。
// 后端在 add/remove 后会 emit `watchlist-changed`，悬浮窗与主界面都监听该事件，
// 所以这里增删完之后两边都会自动刷新，不需要额外通知。
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { useWatchlistStore } from '@/stores/watchlist';
import { useSettingsStore } from '@/stores/settings';
import type { StockBrief } from '@/types';

const win = getCurrentWindow();
const watchlist = useWatchlistStore();
const settings = useSettingsStore();

/// 输入多少毫秒后开始搜索。太短会在连打时打出一串请求，太长会显得迟钝。
const SEARCH_DEBOUNCE_MS = 220;
/// 下拉最多摆多少条结果，避免长列表把面板撑满。
const MAX_RESULTS = 20;

const keyword = ref('');
const results = ref<StockBrief[]>([]);
const searching = ref(false);
const searchError = ref<string | null>(null);
const notice = ref<string | null>(null);
/// 正在处理的那一只（`market+code`），用来防重复点击并显示忙态。
const busyKey = ref<string | null>(null);
const inputEl = ref<HTMLInputElement | null>(null);

let searchTimer: ReturnType<typeof setTimeout> | null = null;
let searchGeneration = 0;
let noticeTimer: ReturnType<typeof setTimeout> | null = null;
let unlistenFocus: UnlistenFn | null = null;
/// 只有「先拿到过焦点」才把失焦当成关闭信号，否则窗口刚 show 时可能
/// 立刻收到一次失焦事件，面板会自己关掉。
let everFocused = false;

const groupName = computed(
  () => watchlist.groups.find((g) => g.id === watchlist.activeGroupId)?.name ?? '全部',
);

function keyOf(code: string, market: string) {
  return `${market}${code}`;
}

onMounted(async () => {
  try {
    await settings.fetchSettings();
    settings.applyTheme(settings.theme);
    await watchlist.fetchWatchlist();
  } catch (error) {
    console.error('[TickerQuickAdd] init failed:', error);
  }

  window.addEventListener('keydown', onKeydown);
  try {
    unlistenFocus = await win.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        everFocused = true;
        void nextTick(() => inputEl.value?.focus());
      } else if (everFocused) {
        // 点到别处（例如点了悬浮条去开主界面）就收起面板。
        void close();
      }
    });
  } catch (error) {
    console.error('[TickerQuickAdd] focus listener failed:', error);
  }
});

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown);
  if (searchTimer) clearTimeout(searchTimer);
  if (noticeTimer) clearTimeout(noticeTimer);
  if (unlistenFocus) unlistenFocus();
});

function flashNotice(text: string) {
  notice.value = text;
  if (noticeTimer) clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => {
    notice.value = null;
  }, 2600);
}

async function close() {
  try {
    await invoke('close_ticker_quick_add');
  } catch (error) {
    console.error('[TickerQuickAdd] close failed:', error);
  }
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault();
    void close();
  }
}

watch(keyword, () => {
  if (searchTimer) clearTimeout(searchTimer);
  const query = keyword.value.trim();
  if (!query) {
    searchGeneration += 1;
    results.value = [];
    searching.value = false;
    searchError.value = null;
    return;
  }
  searching.value = true;
  searchTimer = setTimeout(() => {
    void runSearch(query);
  }, SEARCH_DEBOUNCE_MS);
});

async function runSearch(query: string) {
  const request = ++searchGeneration;
  try {
    const hits = await invoke<StockBrief[]>('search_stocks', { keyword: query });
    if (request !== searchGeneration) return;
    results.value = hits.slice(0, MAX_RESULTS);
    searchError.value = null;
  } catch (error) {
    if (request !== searchGeneration) return;
    results.value = [];
    searchError.value = String(error);
  } finally {
    if (request === searchGeneration) searching.value = false;
  }
}

/// 回车直接加第一项 —— 输代码时基本不会有歧义，省一次点击。
function addFirst() {
  const first = results.value[0];
  if (first) void addStock(first);
}

async function addStock(item: StockBrief) {
  const key = keyOf(item.code, item.market);
  if (busyKey.value) return;
  busyKey.value = key;
  try {
    await watchlist.addStock(item.code, item.market, item.name);
    keyword.value = '';
    results.value = [];
    flashNotice(`已添加 ${item.name}`);
    await nextTick();
    inputEl.value?.focus();
  } catch (error) {
    flashNotice(`添加失败：${error}`);
  } finally {
    busyKey.value = null;
  }
}

async function removeStock(item: { code: string; market: string; name: string }) {
  const key = keyOf(item.code, item.market);
  if (busyKey.value) return;
  busyKey.value = key;
  try {
    await watchlist.removeStock(item.code, item.market);
    flashNotice(`已删除 ${item.name}`);
  } catch (error) {
    flashNotice(`删除失败：${error}`);
  } finally {
    busyKey.value = null;
  }
}
</script>

<template>
  <div class="quick">
    <header class="quick-head">
      <span class="quick-title">快速自选 · {{ groupName }}</span>
      <button class="quick-icon-btn" type="button" title="收起（Esc）" @click="close">×</button>
    </header>

    <div class="quick-search">
      <input
        ref="inputEl"
        v-model="keyword"
        class="quick-input"
        type="text"
        spellcheck="false"
        autocomplete="off"
        placeholder="输入代码或名称，回车加第一只"
        @keydown.enter.prevent="addFirst"
      />
      <span v-if="searching" class="quick-hint">搜索中…</span>
    </div>

    <p v-if="notice" class="quick-notice">{{ notice }}</p>
    <p v-if="searchError" class="quick-error">搜索失败：{{ searchError }}</p>

    <ul v-if="results.length" class="quick-results">
      <li v-for="item in results" :key="keyOf(item.code, item.market)">
        <button
          class="quick-result"
          type="button"
          :disabled="busyKey === keyOf(item.code, item.market)"
          @click="addStock(item)"
        >
          <span class="quick-code">{{ item.code }}</span>
          <span class="quick-name">{{ item.name }}</span>
          <span class="quick-add-mark">＋</span>
        </button>
      </li>
    </ul>

    <div class="quick-split">
      <span>当前自选 {{ watchlist.items.length }} 只</span>
      <span class="quick-split-tip">点 × 直接删掉</span>
    </div>

    <ul class="quick-current">
      <li
        v-for="item in watchlist.items"
        :key="keyOf(item.code, item.market)"
        class="quick-current-row"
      >
        <span class="quick-code">{{ item.code }}</span>
        <span class="quick-name">{{ item.name }}</span>
        <button
          class="quick-icon-btn quick-remove"
          type="button"
          :title="`删除 ${item.name}`"
          :disabled="busyKey === keyOf(item.code, item.market)"
          @click="removeStock(item)"
        >
          ×
        </button>
      </li>
      <li v-if="!watchlist.items.length" class="quick-empty">还没有自选股，搜一只加上试试</li>
    </ul>
  </div>
</template>

<style scoped>
.quick {
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: var(--space-2);
  gap: var(--space-1);
  overflow: hidden;
}

.quick-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
}
.quick-title {
  color: var(--color-text-primary);
  font-size: var(--text-sm);
  font-weight: var(--font-weight-semibold);
}

.quick-icon-btn {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-tertiary);
  font-size: var(--text-sm);
  line-height: 1;
  cursor: pointer;
  transition: background var(--transition-fast), color var(--transition-fast);
}
.quick-icon-btn:hover {
  background: var(--color-accent-dim);
  color: var(--color-text-primary);
}
.quick-icon-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.quick-search {
  position: relative;
  flex-shrink: 0;
}
.quick-input {
  width: 100%;
  padding: 5px var(--space-2);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-sm);
  background: var(--color-surface-0);
  color: var(--color-text-primary);
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  outline: none;
  transition: border-color var(--transition-fast);
}
.quick-input:focus {
  border-color: var(--color-accent);
}
.quick-input::placeholder {
  color: var(--color-text-tertiary);
}
.quick-hint {
  position: absolute;
  right: var(--space-2);
  top: 50%;
  transform: translateY(-50%);
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  pointer-events: none;
}

.quick-notice,
.quick-error {
  flex-shrink: 0;
  margin: 0;
  font-size: var(--text-xs);
}
.quick-notice {
  color: var(--color-accent);
}
.quick-error {
  color: var(--color-error);
}

.quick-results {
  flex-shrink: 1;
  max-height: 108px;
  margin: 0;
  padding: 0;
  list-style: none;
  overflow-y: auto;
}
.quick-result {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  width: 100%;
  padding: 3px var(--space-1);
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-primary);
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  text-align: left;
  cursor: pointer;
}
.quick-result:hover {
  background: var(--color-surface-2);
}
.quick-result:disabled {
  opacity: 0.5;
  cursor: default;
}
.quick-add-mark {
  margin-left: auto;
  color: var(--color-accent);
  font-size: var(--text-xs);
}

.quick-split {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
  padding-top: var(--space-1);
  border-top: 1px solid var(--color-border-0);
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
}
.quick-split-tip {
  color: var(--color-text-tertiary);
}

.quick-current {
  flex: 1;
  min-height: 0;
  margin: 0;
  padding: 0;
  list-style: none;
  overflow-y: auto;
}
.quick-current-row {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: 2px var(--space-1);
  border-radius: var(--radius-sm);
}
.quick-current-row:hover {
  background: var(--color-surface-2);
}
.quick-code {
  flex-shrink: 0;
  width: 54px;
  color: var(--color-text-secondary);
  font-family: var(--font-mono);
  font-size: var(--text-xs);
}
.quick-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.quick-remove {
  margin-left: auto;
}
.quick-empty {
  padding: var(--space-2) 0;
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  text-align: center;
}
</style>
