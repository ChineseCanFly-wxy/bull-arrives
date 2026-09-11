// src/utils/hotkey.ts
// 快捷键录制与展示的唯一实现，设置面板（悬浮窗 / 分组切换）共用。
//
// 录制结果必须能通过 Rust 侧 `Shortcut`（global-hotkey）解析，因此：
// - 修饰键统一用 Ctrl / Alt / Shift / Super（macOS 的 ⌘ 映射为 Super）
// - 主键用 A–Z / 0–9 / F1–F24 / Left|Right|Up|Down / Space 等后端可识别的写法
// - 必须至少含一个修饰键，否则全局单键会吞掉系统里的同类输入

/** 纯修饰键：单独按下不构成组合键。 */
export const MODIFIER_KEYS = new Set(['Control', 'Alt', 'Shift', 'Meta']);

/** 录制结果：组合键字符串，或控制指令，或 null 表示本次按键无效需忽略。 */
export type HotkeyCaptureResult = string | 'cancel' | 'confirm' | null;

/** 把浏览器按键事件转换为后端可解析的组合键字符串。 */
export function eventToHotkey(event: KeyboardEvent): HotkeyCaptureResult {
  if (event.key === 'Escape') return 'cancel';
  if (event.key === 'Enter') return 'confirm';
  if (MODIFIER_KEYS.has(event.key)) return null; // 只按了修饰键，继续等主键
  const parts: string[] = [];
  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  if (event.metaKey) parts.push('Super');
  if (parts.length === 0) return null; // 没有修饰键，拒绝录制
  let main = event.key;
  if (main === ' ') main = 'Space';
  else if (main.length === 1) main = main.toUpperCase();
  else if (/^F\d{1,2}$/.test(main)) { /* 功能键后端直接支持，原样使用 */ }
  else if (main.startsWith('Arrow')) main = main.slice(5);
  else return null; // Backspace / Tab 等不支持，忽略
  return [...parts, main].join('+');
}

/** 是否运行在 macOS（用于快捷键符号展示与提示文案）。 */
export function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  const platform = navigator.platform || '';
  return /mac/i.test(platform) || /mac os x/i.test(navigator.userAgent || '');
}

const MAC_MODIFIER_SYMBOLS: Record<string, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' };
const KEY_SYMBOLS: Record<string, string> = {
  Left: '←', Right: '→', Up: '↑', Down: '↓', Space: '空格', Backquote: '`',
};

/**
 * 组合键字符串 → 界面展示文本。
 * macOS 用 ⌃⌥⇧⌘ 符号（如 ⌃⌥←），其余平台保持 Ctrl+Alt+← 形式。
 */
export function formatHotkeyLabel(combo: string): string {
  if (!combo) return '';
  const parts = combo.split('+').map(part => KEY_SYMBOLS[part] ?? part);
  if (isMacPlatform()) {
    const head = parts.slice(0, -1).map(part => MAC_MODIFIER_SYMBOLS[part] ?? part).join('');
    const tail = parts[parts.length - 1] ?? '';
    return head ? `${head} ${tail}` : tail;
  }
  return parts.join('+');
}
