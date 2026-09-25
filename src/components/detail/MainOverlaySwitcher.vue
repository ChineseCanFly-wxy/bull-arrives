<script setup lang="ts">
import type { MainOverlayType } from '@/types';

defineProps<{
  modelValue: MainOverlayType;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: MainOverlayType];
}>();

const overlays: { key: MainOverlayType; label: string }[] = [
  { key: 'MA', label: '均线' },
  { key: 'BOLL', label: 'BOLL' },
];
</script>

<template>
  <div class="main-overlay-switcher" role="tablist" aria-label="主图叠加指标切换">
    <button
      v-for="tab in overlays"
      :key="tab.key"
      role="tab"
      :aria-selected="modelValue === tab.key"
      class="switcher-tab"
      :class="{ active: modelValue === tab.key }"
      @click="emit('update:modelValue', tab.key)"
    >
      {{ tab.label }}
    </button>
  </div>
</template>
