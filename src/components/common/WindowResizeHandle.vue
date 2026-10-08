<script setup lang="ts">
import { getCurrentWindow, PhysicalSize } from '@tauri-apps/api/window';
const emit = defineEmits<{ error: [message: string] }>();
async function resize() {
  try { await getCurrentWindow().startResizeDragging('SouthEast'); }
  catch (error) { emit('error', String(error)); console.error('悬浮窗缩放失败', error); }
}
async function keyResize(event: KeyboardEvent) {
  if (!['ArrowUp','ArrowDown','ArrowLeft','ArrowRight'].includes(event.key)) return;
  event.preventDefault(); event.stopPropagation();
  try {
    const window = getCurrentWindow(); const size = await window.innerSize(); const step = 20 * await window.scaleFactor();
    await window.setSize(new PhysicalSize(Math.max(1,size.width + (event.key === 'ArrowRight' ? step : event.key === 'ArrowLeft' ? -step : 0)), Math.max(1,size.height + (event.key === 'ArrowDown' ? step : event.key === 'ArrowUp' ? -step : 0))));
  } catch (error) { emit('error', String(error)); console.error('悬浮窗缩放失败', error); }
}
</script>
<template><button class="window-resize-handle" type="button" title="拖动缩放；方向键微调" aria-label="调整悬浮窗大小" @mousedown.left.stop.prevent="resize" @click.stop @keydown.stop="keyResize">◢</button></template>
<style scoped>.window-resize-handle{position:absolute;right:2px;bottom:2px;width:22px;height:22px;padding:0;border:0;background:transparent;color:inherit;opacity:.65;cursor:nwse-resize;line-height:22px;font-size:15px;border-radius:3px}.window-resize-handle:hover{opacity:1;background:color-mix(in srgb,currentColor 12%,transparent)}.window-resize-handle:focus-visible{outline:2px solid currentColor;opacity:1}</style>
