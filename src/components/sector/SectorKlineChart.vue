<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { DataLoader, KLineData as ChartBar } from 'klinecharts';
import type { PeriodType } from '@/types';
import type { SectorHistory, SectorPeriod } from '@/types/sector';
import { useChartCore } from '@/composables/useChartCore';

const props = defineProps<{ code: string; name: string; period: SectorPeriod }>();
const chartRef = ref<HTMLElement | null>(null);
const source = ref('');
const asOf = ref('');
let generation = 0;

const { chart, loading, error, initChartCore, periodToKlinecharts, syncPrecision } = useChartCore({
  chartRef,
  code: computed(() => props.code),
  market: 'sector',
  name: computed(() => props.name),
});

async function load() {
  const request = ++generation;
  loading.value = true;
  error.value = '';
  try {
    const response = await invoke<SectorHistory>('get_sector_history', {
      sectorCode: props.code,
      period: props.period,
    });
    if (request !== generation) return;
    const bars: ChartBar[] = response.items.map(item => ({
      timestamp: new Date(`${item.date}T00:00:00+08:00`).getTime(),
      open: item.open,
      close: item.close,
      high: item.high,
      low: item.low,
      volume: item.volume,
      turnover: item.amount,
    }));
    const loader: DataLoader = {
      getBars: params => params.callback(bars, { forward: false, backward: false }),
    };
    chart.value?.setSymbol({ ticker: props.code, name: props.name });
    chart.value?.setPeriod(periodToKlinecharts(props.period as PeriodType));
    chart.value?.setDataLoader(loader);
    syncPrecision(bars);
    source.value = response.source;
    asOf.value = response.as_of;
  } catch (e) {
    if (request === generation) error.value = `板块 K 线加载失败：${e}`;
  } finally {
    if (request === generation) loading.value = false;
  }
}

async function initAndLoad() {
  await nextTick();
  initChartCore(props.period as PeriodType);
  await load();
}

onMounted(initAndLoad);
watch(() => [props.code, props.period], initAndLoad);
</script>

<template>
  <div class="sector-kline">
    <div class="chart-meta">{{ source || '东方财富板块指数' }} · {{ asOf || '--' }}</div>
    <div class="chart-host">
      <div v-if="loading" class="chart-state">加载板块 K 线…</div>
      <div v-else-if="error" class="chart-state error" role="alert">{{ error }} <button @click="load">重试</button></div>
      <div ref="chartRef" class="chart"></div>
    </div>
  </div>
</template>

<style scoped>
.sector-kline { display: flex; flex-direction: column; min-height: 360px; }
.chart-meta { color: var(--color-text-tertiary); font-size: var(--text-xs); padding-bottom: 6px; }
.chart-host { position: relative; height: 350px; min-height: 300px; }
.chart { width: 100%; height: 100%; }
.chart-state { position: absolute; inset: 0; z-index: 2; display: grid; place-content: center; color: var(--color-text-secondary); background: var(--color-surface-1); }
.chart-state.error { color: var(--color-warning); gap: 8px; }
.chart-state button { color: inherit; border: 0; background: transparent; text-decoration: underline; cursor: pointer; }
</style>
