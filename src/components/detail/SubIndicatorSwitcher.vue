<script setup lang="ts">
import { ref, computed } from 'vue';
import { NDropdown } from 'naive-ui';
import type { SubIndicatorType } from '@/types';

const props = defineProps<{
  modelValue: SubIndicatorType;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: SubIndicatorType];
}>();

const dropdownOpen = ref(false);

const indicatorTabs: { key: SubIndicatorType; label: string }[] = [
  { key: 'VOL', label: '成交量' },
  { key: 'MACD', label: 'MACD' },
];

const extraIndicators: { key: string; label: string }[] = [
  { key: 'KDJ', label: 'KDJ' },
  { key: 'RSI', label: 'RSI' },
];

const isExtraActive = computed(() =>
  extraIndicators.some((o) => o.key === props.modelValue)
);

const dropdownOptions = computed(() =>
  extraIndicators.map((o) => ({
    label: props.modelValue === o.key ? `✓ ${o.label}` : o.label,
    key: o.key,
  }))
);

const moreLabel = computed(() => {
  const found = extraIndicators.find((o) => o.key === props.modelValue);
  return found ? found.label : '更多';
});

function handleSelect(key: string) {
  emit('update:modelValue', key as SubIndicatorType);
}
</script>

<template>
  <div class="sub-indicator-switcher" role="tablist" aria-label="副图指标切换">
    <button
      v-for="tab in indicatorTabs"
      :key="tab.key"
      role="tab"
      :aria-selected="modelValue === tab.key"
      class="switcher-tab"
      :class="{ active: modelValue === tab.key }"
      @click="emit('update:modelValue', tab.key)"
    >
      {{ tab.label }}
    </button>

    <n-dropdown
      trigger="click"
      :show="dropdownOpen"
      @update:show="(v: boolean) => dropdownOpen = v"
      :options="dropdownOptions"
      @select="handleSelect"
    >
      <button
        class="switcher-tab switcher-more"
        :class="{ active: isExtraActive }"
        :aria-selected="isExtraActive"
        aria-haspopup="menu"
        :aria-expanded="dropdownOpen"
      >
        {{ moreLabel }}<span class="more-caret" aria-hidden="true">▾</span>
      </button>
    </n-dropdown>
  </div>
</template>
