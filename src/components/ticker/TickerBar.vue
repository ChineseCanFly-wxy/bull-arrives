<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { useQuoteStore } from '@/stores/quote';
import { useWatchlistStore } from '@/stores/watchlist';
import { useSettingsStore, SETTING_CHANGED_EVENT, type SettingChangedPayload } from '@/stores/settings';
import { formatPrice } from '@/utils/format';

const quoteStore = useQuoteStore();
const watchlist = useWatchlistStore();
const settings = useSettingsStore();
const paused = ref(false);
const page = ref(0);
let cycleTimer: ReturnType<typeof setInterval> | null = null;
const groupName = computed(() => watchlist.groups.find(g => g.id === watchlist.activeGroupId)?.name ?? '全部');
const groupFlash = ref(false);
let groupTimer: ReturnType<typeof setTimeout> | undefined;
watch(() => watchlist.activeGroupId, () => {
  page.value = 0;
  groupFlash.value = true;
  if (groupTimer) clearTimeout(groupTimer);
  groupTimer = setTimeout(() => { groupFlash.value = false; }, 1200);
});
let unlistenSettings: UnlistenFn | null = null;

const initFailed = ref(false);

onMounted(async () => {
  try {
    await settings.fetchSettings();
    settings.applyTheme(settings.theme);
    await watchlist.fetchWatchlist();
    await quoteStore.startListening();
    startCycle();
    startSettingsListen();
  } catch (e) {
    initFailed.value = true;
    console.error('[TickerBar] init failed:', e);
  }
});

onUnmounted(() => {
  quoteStore.stopListening();
  if (cycleTimer) clearInterval(cycleTimer);
  if (unlistenSettings) unlistenSettings();
  if (groupTimer) clearTimeout(groupTimer);
});

/// The main window and this ticker are separate WebViews with independent
/// Pinia stores.  Any setting that changes how the ticker looks (theme,
/// single-color mode, text color, …) is broadcast as `setting-changed` from
/// the window that owns the settings dialog; we apply it here directly.
function startSettingsListen() {
  listen<SettingChangedPayload>(SETTING_CHANGED_EVENT, (event) => {
    const { key, value } = event.payload;
    settings.applyRemoteSetting(key, value);
  }).then((unlisten) => {
    unlistenSettings = unlisten;
  }).catch((e) => {
    console.error('[TickerBar] Failed to listen setting-changed:', e);
  });
}

function startCycle() {
  cycleTimer = setInterval(() => {
    if (!paused.value && tickerItems.value.length > 2) {
      page.value = (page.value + 2) % tickerItems.value.length;
    }
  }, 3000);
}

const tickerItems = computed(() =>
  watchlist.items.map(item => {
    const q = quoteStore.getQuote(item.code, item.market);
    return {
      name: item.name,
      code: item.code,
      price: q?.price ?? null,
      changePct: q?.change_pct ?? null,
    };
  })
);

const visibleItems = computed(() => {
  const items = tickerItems.value;
  if (items.length === 0) return [];
  if (items.length === 1) return [items[0]];
  const count = Math.min(2, items.length);
  const result = [];
  for (let i = 0; i < count; i++) {
    result.push(items[(page.value + i) % items.length]);
  }
  return result;
});

const retryHintVisible = ref(false);

// ── Single-color (stealth) mode ──
// When enabled, every ticker text element uses one user-chosen color instead
// of the default up-red / down-green, making the floating bar look
// unobtrusive at a glance.
// Implemented as a root class + CSS custom property rather than inline
// styles on each span: a single declaration covers name/price/change/na and
// reliably wins over the `.up` / `.down` tone classes.
const monoStyle = computed(() =>
  settings.tickerSingleColor
    ? { '--ticker-mono-color': settings.tickerTextColor }
    : {}
);

function priceTone(changePct: number | null): string {
  if (settings.tickerSingleColor || changePct === null) return '';
  return changePct >= 0 ? 'up' : 'down';
}

// ── Dragging ──
// Uses Tauri's startDragging() API (Win32 DefWindowProc) for smooth
// OS-level window dragging on both Windows 10 and 11.
// Position is auto-saved by the Rust WindowEvent::Moved handler in lib.rs.
// Click vs drag detection via mousemove threshold:
// - Click (mouse moves <3px): @click fires → opens main window
// - Drag (mouse moves ≥3px): startDragging() triggers OS drag → @click does NOT fire
//   because startDragging() enters a Win32 modal drag loop that consumes mouseup.
//   Document-level mousemove listener ensures we catch fast mouse movements
//   that leave the ticker bar element.

let isDragging = false;

function onMouseDown(e: MouseEvent) {
  isDragging = false;
  if (initFailed.value) {
    return;
  }
  const startX = e.clientX;
  const startY = e.clientY;

  const onMouseMove = (ev: MouseEvent) => {
    if (isDragging) return;
    if (Math.abs(ev.clientX - startX) > 3 || Math.abs(ev.clientY - startY) > 3) {
      isDragging = true;
      document.removeEventListener('mousemove', onMouseMove);
      document.removeEventListener('mouseup', onMouseUp);
      getCurrentWindow().startDragging().catch((err) => {
        console.error('[TickerBar] startDragging failed:', err);
      });
    }
  };

  const onMouseUp = () => {
    document.removeEventListener('mousemove', onMouseMove);
    document.removeEventListener('mouseup', onMouseUp);
  };

  document.addEventListener('mousemove', onMouseMove);
  document.addEventListener('mouseup', onMouseUp);
}

async function handleClick() {
  if (isDragging) return;
  if (initFailed.value) {
    if (cycleTimer) { clearInterval(cycleTimer); cycleTimer = null; }
    if (unlistenSettings) { unlistenSettings(); unlistenSettings = null; }
    quoteStore.stopListening();

    initFailed.value = false;
    retryHintVisible.value = true;
    try {
      await settings.fetchSettings();
      settings.applyTheme(settings.theme);
      await watchlist.fetchWatchlist();
      await quoteStore.startListening();
      startCycle();
      startSettingsListen();
        retryHintVisible.value = false;
    } catch (e) {
      initFailed.value = true;
      retryHintVisible.value = false;
      console.error('[TickerBar] retry failed:', e);
    }
    return;
  }
  await invoke('show_main_window').catch((e) => { console.error('[TickerBar] show_main_window failed:', e); });
}
</script>

<template>
  <div
    class="ticker-bar"
    :class="{ mono: settings.tickerSingleColor }"
    :style="monoStyle"
    role="button"
    tabindex="0"
    :aria-label="`${groupName}分组，点击显示主界面`"
    :title="`当前分组：${groupName}`"
    @keydown.enter="handleClick"
    @keydown.space.prevent="handleClick"
    @mousedown="onMouseDown"
    @mouseenter="paused = true"
    @mouseleave="paused = false"
    @click="handleClick"
  >
    <div v-if="groupFlash" class="ticker-empty">当前分组 · {{ groupName }}</div>
    <template v-else-if="initFailed">
      <div class="ticker-row ticker-error-row">
        <span class="ticker-error-text" :title="'点击重试'">Bull Arrives</span>
        <span class="ticker-retry-hint">· 点击重试</span>
      </div>
    </template>
    <template v-else-if="retryHintVisible">
      <div class="ticker-row ticker-error-row">
        <span class="ticker-error-text">重连中...</span>
      </div>
    </template>
    <template v-else-if="visibleItems.length > 0">
      <div v-for="item in visibleItems" :key="item.code" class="ticker-row">
        <span class="ticker-name">{{ item.name }}</span>
        <span
          v-if="item.price !== null"
          class="ticker-price tabular-nums"
          :class="priceTone(item.changePct)"
        >{{ formatPrice(item.price) }}</span>
        <span v-else class="ticker-na">--</span>
        <span
          v-if="item.changePct !== null"
          class="ticker-change tabular-nums"
          :class="priceTone(item.changePct)"
        >{{ item.changePct >= 0 ? '+' : '' }}{{ item.changePct.toFixed(2) }}%</span>
      </div>
    </template>
    <div v-else class="ticker-empty">暂无自选</div>
  </div>
</template>

<style scoped>
.ticker-bar {
  width: 100%;
  height: 100%;
  background: transparent;
  display: flex;
  flex-direction: column;
  justify-content: center;
  user-select: none;
  cursor: grab;
  overflow: hidden;
  padding: var(--space-1) var(--space-2);
  transition: background var(--transition-fast);
}
.ticker-bar:hover {
  background: rgba(255, 255, 255, 0.03);
}
.ticker-row {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  line-height: 1.4;
}
.ticker-name {
  flex: 1;
  min-width: 0;
  color: var(--color-text-primary);
  font-size: var(--text-xs);
  font-weight: var(--font-weight-medium);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ticker-price {
  flex-shrink: 0;
  font-weight: var(--font-weight-semibold);
  font-size: var(--text-xs);
  font-family: var(--font-mono);
  width: 46px;
  text-align: right;
  color: var(--color-text-primary);
}
.ticker-na {
  flex-shrink: 0;
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  font-family: var(--font-mono);
  width: 46px;
  text-align: right;
}
.ticker-change {
  flex-shrink: 0;
  font-size: var(--text-xs);
  font-family: var(--font-mono);
  width: 48px;
  text-align: right;
}
.up { color: var(--color-up); }
.down { color: var(--color-down); }

/* ── Single-color (stealth) mode ──
   Overrides every text color (including .up/.down) with one flat color.
   Placed last so it wins on equal specificity. */
.ticker-bar.mono .ticker-name,
.ticker-bar.mono .ticker-price,
.ticker-bar.mono .ticker-change,
.ticker-bar.mono .ticker-na,
.ticker-bar.mono .ticker-empty,
.ticker-bar.mono .up,
.ticker-bar.mono .down {
  color: var(--ticker-mono-color, var(--color-text-primary));
}
.ticker-empty {
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  text-align: center;
  width: 100%;
}
.ticker-error-row {
  justify-content: center;
}
.ticker-error-text {
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  font-weight: var(--font-weight-medium);
  letter-spacing: 0.05em;
}
.ticker-retry-hint {
  color: var(--color-warning);
  font-size: 9px;
  opacity: 0.7;
}
</style>
