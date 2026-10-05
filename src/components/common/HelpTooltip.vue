<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue';
import { NTooltip, type TooltipInst } from 'naive-ui';
defineProps<{ label: string }>();
const id = useId(); const show = ref(false);
const tooltip = ref<TooltipInst | null>(null); const trigger = ref<HTMLButtonElement | null>(null); const content = ref<HTMLElement | null>(null);
const position = ref({ x: 0, y: 0 }); const placement = ref<'top' | 'bottom'>('top'); const arrow = ref(true);
let observer: ResizeObserver | undefined; let frame = 0;
function open() {
  const rect = trigger.value?.getBoundingClientRect();
  if (rect) position.value = { x: rect.left + rect.width / 2, y: rect.top };
  show.value = true;
}
function schedulePosition() {
  if (!show.value) return;
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(() => { void syncPosition(); });
}
async function syncPosition() {
  if (!show.value || !trigger.value || !content.value) return;
  const rect = trigger.value.getBoundingClientRect();
  const width = document.documentElement.clientWidth; const height = document.documentElement.clientHeight;
  if (rect.bottom < 0 || rect.top > height || rect.right < 0 || rect.left > width) { show.value = false; return; }
  const popover = content.value.closest<HTMLElement>('.n-popover');
  if (!popover) return;
  const center = rect.left + rect.width / 2; const half = popover.offsetWidth / 2;
  const x = Math.max(half + 12, Math.min(center, width - half - 12));
  const above = rect.top; const below = height - rect.bottom;
  const top = above >= popover.offsetHeight + 22 || above >= below;
  placement.value = top ? 'top' : 'bottom';
  position.value = { x, y: top ? rect.top : rect.bottom };
  arrow.value = Math.abs(x - center) < 1;
  await nextTick(); if (show.value) tooltip.value?.syncPosition();
}
watch([show, content], async ([visible, element]) => {
  observer?.disconnect();
  if (!visible || !element) return;
  observer = new ResizeObserver(schedulePosition); observer.observe(element);
  await nextTick(); schedulePosition();
}, { flush: 'post' });
let triggerHovered = false; let tooltipHovered = false; let focused = false; let dismissed = false;
let showTimer: ReturnType<typeof setTimeout> | undefined;
let hideTimer: ReturnType<typeof setTimeout> | undefined;
function clearTimers() { clearTimeout(showTimer); clearTimeout(hideTimer); }
function enterTrigger() {
  triggerHovered = true; dismissed = false; clearTimers();
  showTimer = setTimeout(() => { if (triggerHovered && !dismissed) open(); }, 220);
}
function scheduleHide() {
  clearTimeout(showTimer); clearTimeout(hideTimer);
  hideTimer = setTimeout(() => { if (!triggerHovered && !tooltipHovered && !focused) show.value = false; }, 140);
}
function focusTrigger() { focused = true; dismissed = false; clearTimers(); open(); }
function blurTrigger() { focused = false; scheduleHide(); }
function enterTooltip() { tooltipHovered = true; clearTimeout(hideTimer); }
function escape(event: KeyboardEvent) {
  if (event.key === 'Escape' && show.value) {
    dismissed = true; show.value = false; tooltipHovered = false; clearTimers();
    event.stopImmediatePropagation();
  }
}
onMounted(() => { window.addEventListener('keydown', escape, true); window.addEventListener('resize', schedulePosition); window.addEventListener('scroll', schedulePosition, true); });
onBeforeUnmount(() => { clearTimers(); observer?.disconnect(); cancelAnimationFrame(frame); window.removeEventListener('keydown', escape, true); window.removeEventListener('resize', schedulePosition); window.removeEventListener('scroll', schedulePosition, true); });
</script>

<template>
  <!-- Explicit teleport escapes modal/scroll clipping; true also respects fullscreen. -->
  <button ref="trigger" type="button" class="help-tooltip-trigger" :aria-label="label" :aria-describedby="show ? id : undefined"
      @mouseenter="enterTrigger" @mouseleave="triggerHovered = false; scheduleHide()" @focus="focusTrigger" @blur="blurTrigger">?</button>
  <NTooltip ref="tooltip" trigger="manual" :placement="placement" :x="position.x" :y="position.y" :show="show" :to="true" :animated="false" :show-arrow="arrow">
    <div ref="content" :id="id" class="help-tooltip-content" role="tooltip" @mouseenter="enterTooltip" @mouseleave="tooltipHovered = false; scheduleHide()"><slot /></div>
  </NTooltip>
</template>

<style scoped>
.help-tooltip-trigger { display: inline-grid; place-items: center; flex: none; width: 20px; height: 20px; padding: 0; margin-left: 6px; border: 1px solid var(--color-border-1); border-radius: 50%; background: var(--color-surface-2); color: var(--color-text-secondary); font-family: inherit; font-size: 12px; font-weight: 600; line-height: 1; cursor: help; vertical-align: middle; }
.help-tooltip-trigger:hover,.help-tooltip-trigger:focus-visible { color: var(--color-accent); border-color: var(--color-accent); outline: 2px solid var(--color-accent-dim); outline-offset: 2px; }
.help-tooltip-content { max-width: min(370px, calc(100vw - 52px)); max-height: min(400px, calc(50dvh - 40px)); overflow-y: auto; font-size: 12px; line-height: 1.7; white-space: normal; overflow-wrap: anywhere; }
</style>
