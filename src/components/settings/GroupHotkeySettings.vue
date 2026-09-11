<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NButton } from 'naive-ui';
import { eventToHotkey, formatHotkeyLabel, isMacPlatform, MODIFIER_KEYS } from '@/utils/hotkey';

interface HotkeyEntry { hotkey: string; registered: boolean }
interface HotkeyStatus { previous: HotkeyEntry; next: HotkeyEntry }

type Direction = 'previous' | 'next';

/// 每行一个方向：点击方框 → 直接按键录制 → 立即保存；清空即停用该功能。
const rows: { direction: Direction; label: string; hint: string }[] = [
  { direction: 'previous', label: '上一分组', hint: '例如 Ctrl+Alt+←' },
  { direction: 'next', label: '下一分组', hint: '例如 Ctrl+Alt+→' },
];

const mac = isMacPlatform();
const state = ref<Record<Direction, HotkeyEntry>>({
  previous: { hotkey: '', registered: false },
  next: { hotkey: '', registered: false },
});
const capturing = ref<Direction | null>(null);
const capturedDraft = ref<string | null>(null);
const busy = ref<Direction | null>(null);
const error = ref<string | null>(null);

onMounted(() => { void refresh(); });
onBeforeUnmount(stopCapture);

async function refresh() {
  try {
    const status = await invoke<HotkeyStatus>('get_group_hotkey_status');
    state.value = { previous: status.previous, next: status.next };
  } catch (e) {
    error.value = `读取快捷键状态失败：${String(e)}`;
  }
}

function startCapture(direction: Direction) {
  stopCapture();
  capturing.value = direction;
  capturedDraft.value = null;
  error.value = null;
  window.addEventListener('keydown', onKeyDown, true);
}

function stopCapture() {
  capturing.value = null;
  capturedDraft.value = null;
  window.removeEventListener('keydown', onKeyDown, true);
}

function onKeyDown(event: KeyboardEvent) {
  event.preventDefault();
  event.stopPropagation();
  const combo = eventToHotkey(event);
  if (combo === 'cancel') return stopCapture();
  if (combo === 'confirm') {
    if (capturedDraft.value && capturing.value) void commit(capturing.value, capturedDraft.value);
    return;
  }
  if (!combo) {
    if (!MODIFIER_KEYS.has(event.key)) error.value = '组合键需包含 Ctrl / Alt / Shift / ⌘ 中的至少一个修饰键';
    return;
  }
  const direction = capturing.value;
  if (!direction) return;
  capturedDraft.value = combo;
  void commit(direction, combo);
}

/// 保存并回读真实生效状态：后端返回的 registered 才是「是否真的注册成功」。
async function commit(direction: Direction, hotkey: string) {
  if (busy.value) return;
  busy.value = direction;
  error.value = null;
  try {
    const status = await invoke<HotkeyStatus>('set_group_hotkey', { direction, hotkey });
    state.value = { previous: status.previous, next: status.next };
    stopCapture();
  } catch (e) {
    // 保留录制态，方便立刻换一个组合键重试；Esc 可退出。
    error.value = String(e);
  } finally {
    busy.value = null;
  }
}

function clear(direction: Direction) {
  stopCapture();
  void commit(direction, '');
}
</script>

<template>
  <section class="hotkey-settings">
    <h3>切换分组快捷键</h3>
    <p>
      全局生效，两个窗口同步切换。点击方框后直接按组合键即可录制，
      留空或用「清空」即停用该功能。
      <template v-if="mac">macOS 上请避开 ⌘Q、⌘W 等系统快捷键。</template>
    </p>
    <div v-for="row in rows" :key="row.direction" class="hotkey-row">
      <span class="hotkey-name">{{ row.label }}</span>
      <button
        type="button"
        class="hotkey-box"
        :class="{ capturing: capturing === row.direction, empty: !state[row.direction].hotkey }"
        :disabled="busy === row.direction"
        @click="startCapture(row.direction)"
      >
        <template v-if="capturing === row.direction">
          {{ capturedDraft ? formatHotkeyLabel(capturedDraft) : '请按下组合键…' }}
          <small>{{ capturedDraft ? 'Esc 取消' : '需包含修饰键' }}</small>
        </template>
        <template v-else-if="state[row.direction].hotkey">
          {{ formatHotkeyLabel(state[row.direction].hotkey) }}
          <small :class="{ warn: !state[row.direction].registered }">
            {{ state[row.direction].registered ? '已生效' : '未生效（可能被其他程序占用）' }}
          </small>
        </template>
        <template v-else>
          未设置
          <small>点击此处录制 · {{ row.hint }}</small>
        </template>
      </button>
      <NButton
        size="small"
        :disabled="busy === row.direction || !state[row.direction].hotkey"
        @click="clear(row.direction)"
      >清空</NButton>
    </div>
    <p v-if="error" class="field-error">{{ error }}</p>
  </section>
</template>

<style scoped>
.hotkey-settings { padding: 16px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); }
h3 { margin: 0; font-size: 14px; }
p { color: var(--color-text-secondary); font-size: 12px; line-height: 1.6; }
.hotkey-row { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 8px; margin-top: 10px; }
.hotkey-name { font-size: 12px; color: var(--color-text-secondary); white-space: nowrap; }
.hotkey-box {
  display: flex;
  min-height: 42px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 1px;
  border: 1px solid var(--color-border-1);
  border-radius: var(--radius-sm);
  background: var(--color-surface-1);
  color: var(--color-text-primary);
  font-family: var(--font-mono);
  font-size: 13px;
  cursor: pointer;
}
.hotkey-box.empty { font-family: var(--font-sans); color: var(--color-text-tertiary); }
.hotkey-box:hover { border-color: var(--color-accent); }
.hotkey-box.capturing {
  border-color: var(--color-accent);
  color: var(--color-accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 10%, transparent);
}
.hotkey-box:disabled { opacity: 0.6; cursor: wait; }
.hotkey-box small { color: var(--color-text-tertiary); font-family: var(--font-sans); font-size: 9px; }
.hotkey-box small.warn { color: var(--color-warning, #d03050); }
.field-error { color: var(--color-warning) !important; margin-top: 8px; }
</style>
