<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { NButton, NModal } from 'naive-ui';
import type { MainlineNavigationTarget } from '@/types/navigation';
import MainlineDiscovery from './MainlineDiscovery.vue';
import MainlineResearch from './MainlineResearch.vue';
import MainlineHistory from './MainlineHistory.vue';

const props = defineProps<{ show: boolean; initialTarget?: MainlineNavigationTarget | null }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();
const visible = computed({ get: () => props.show, set: value => emit('update:show', value) });
const selected = ref<MainlineNavigationTarget | null>(null);
watch(() => [props.show, props.initialTarget] as const, ([show, target]) => {
  selected.value = show && target ? { ...target } : null;
}, { immediate: true });
function openCandidate(target: MainlineNavigationTarget) { selected.value = { ...target }; }
</script>

<template>
  <NModal v-model:show="visible" preset="card" title="市场主线" :style="{ width: 'min(1060px, calc(100vw - 24px))' }" :content-style="{ maxHeight: 'max(0px, calc(100dvh - 145px))', overflow: 'auto' }" :bordered="false" size="small" class="market-mainline-dialog">
    <div class="mainline-navigation">
      <NButton v-if="selected" text size="small" @click="selected = null">‹ 返回主线候选排名</NButton>
      <ol v-else class="mainline-steps" aria-label="市场主线使用步骤"><li>开始扫描</li><li>点候选查看板块与个股</li><li>按需开启主线提醒</li></ol>
      <span v-if="selected" class="mainline-selected">{{ selected.name }} · {{ selected.code }}</span>
    </div>
    <MainlineDiscovery v-show="!selected" :active="show && !selected" @open="openCandidate" />
    <MainlineResearch v-if="selected" :key="selected.kind + ':' + selected.code + ':' + (selected.snapshotFingerprint ?? '')" :kind="selected.kind" :sector-code="selected.code" :sector-name="selected.name" :snapshot-fingerprint="selected.snapshotFingerprint" />
    <MainlineHistory :sector-code="selected?.code" :kind="selected?.kind" @open="openCandidate" />
  </NModal>
</template>

<style scoped>
.mainline-navigation { display: flex; align-items: center; flex-wrap: wrap; gap: 12px; margin-bottom: 16px; font-size: var(--text-xs); color: var(--color-text-secondary); line-height: 1.7; }
.mainline-steps { display: flex; flex-wrap: wrap; gap: 8px 24px; margin: 0; padding-left: 20px; }
.mainline-selected { color: var(--color-text-primary); overflow-wrap: anywhere; }
</style>
