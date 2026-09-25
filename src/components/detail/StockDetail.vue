<script setup lang="ts">
import { ref, computed } from 'vue';
import type { WatchItem, PeriodType, SubIndicatorType, MainOverlayType } from '@/types';
import { useQuoteStore } from '@/stores/quote';
import { useMinuteKUnavailable } from '@/composables/minutePeriod';
import { formatCode } from '@/utils/format';
import StockSummary from './StockSummary.vue';
import DepthPanel from './DepthPanel.vue';
import MinuteChart from './MinuteChart.vue';
import KLineChart from './KLineChart.vue';
import ChartSwitcher from './ChartSwitcher.vue';
import SubIndicatorSwitcher from './SubIndicatorSwitcher.vue';
import MainOverlaySwitcher from './MainOverlaySwitcher.vue';

const props = defineProps<{
  item: WatchItem;
}>();

const emit = defineEmits<{
  close: [];
}>();

const quoteStore = useQuoteStore();
const quote = computed(() => quoteStore.getQuote(props.item.code, props.item.market));

const activePeriod = ref<PeriodType>('minute');
const activeSubIndicator = ref<SubIndicatorType>('VOL');
const activeMainOverlay = ref<MainOverlayType>('MA');

// 新浪数据源不支持 1 分钟：若正在查看 1 分钟时切到新浪，自动回落到 5 分钟。
useMinuteKUnavailable(activePeriod);
</script>

<template>
  <div class="stock-detail">
    <div class="detail-header">
      <div class="detail-title">
        <span class="detail-name">{{ item.name }}</span>
        <span class="detail-code">{{ formatCode(item.code) }}</span>
      </div>
      <button class="detail-close" @click="emit('close')" aria-label="关闭详情">&times;</button>
    </div>

    <div class="detail-content">
      <div class="detail-left">
        <StockSummary v-if="quote" :quote="quote" />
        <DepthPanel :code="item.code" :market="item.market" />
      </div>
      <div class="detail-right">
        <div class="chart-toolbar">
          <div class="chart-toolbar-group">
            <ChartSwitcher v-model="activePeriod" />
            <MainOverlaySwitcher
              v-if="activePeriod !== 'minute'"
              v-model="activeMainOverlay"
            />
          </div>
          <SubIndicatorSwitcher
            v-if="activePeriod !== 'minute'"
            v-model="activeSubIndicator"
          />
        </div>
        <MinuteChart
          v-if="activePeriod === 'minute'"
          :code="item.code"
          :market="item.market"
          :name="item.name"
          :prev-close="quote?.prev_close"
        />
        <KLineChart
          v-else
          :code="item.code"
          :market="item.market"
          :name="item.name"
          :period="activePeriod"
          :sub-indicator="activeSubIndicator"
          :main-overlay="activeMainOverlay"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.stock-detail {
  display: flex;
  flex: 1 1 56%;
  flex-direction: column;
  box-sizing: border-box;
  min-height: 0;
  overflow: hidden;
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md) var(--radius-md) 0 0;
  background: var(--color-surface-1);
  padding: var(--panel-padding);
}
:global([data-style="classic"]) .stock-detail { border: 0; border-top: 1px solid var(--color-border); border-radius: 0; padding: 12px 16px; }
.detail-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}
.detail-title {
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.detail-name {
  font-size: 15px;
  font-weight: 600;
  color: var(--color-text-primary);
}
.detail-code {
  font-size: 12px;
  color: var(--color-text-tertiary);
}
.detail-close {
  background: none;
  border: none;
  border-radius: var(--radius-sm);
  color: var(--color-text-tertiary);
  font-size: 20px;
  cursor: pointer;
  padding: 0 4px;
  line-height: 1;
}
.detail-close:hover { color: var(--color-text-primary); }
:global([data-style="trading"]) .detail-close:hover,
:global([data-style="modern"]) .detail-close:hover { background: var(--color-surface-hover); }
.detail-content {
  display: flex;
  flex: 1;
  min-height: 0;
  gap: var(--space-4);
}
.detail-left {
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex-shrink: 0;
  overflow: auto;
}
.detail-right {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.detail-right :deep(.chart-container) {
  height: clamp(180px, calc(45vh - 80px), 560px);
}

.chart-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  min-height: var(--control-height);
}

.chart-toolbar-group {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

@media (max-width: 780px) {
  .detail-content { flex-direction: column; overflow: auto; }
  .detail-left { flex-shrink: 1; overflow: visible; }
  .detail-right { min-height: 180px; }
  :global([data-style="trading"]) .stock-detail,
  :global([data-style="modern"]) .stock-detail { flex-basis: 62%; min-height: 180px; }
  :global([data-style="trading"]) .detail-left,
  :global([data-style="modern"]) .detail-left { flex-shrink: 0; }
  :global([data-style="trading"]) .detail-right,
  :global([data-style="modern"]) .detail-right { flex: 0 0 auto; min-height: 200px; }
  :global([data-style="trading"]) .detail-right :deep(.chart-container),
  :global([data-style="modern"]) .detail-right :deep(.chart-container) { min-height: 200px; height: 240px; }
  :global([data-style="trading"]) .chart-toolbar,
  :global([data-style="modern"]) .chart-toolbar { align-items: flex-start; flex-wrap: wrap; }
}

@media (max-height: 620px) {
  :global([data-style="trading"]) .stock-detail,
  :global([data-style="modern"]) .stock-detail { padding-block: 8px; }
  :global([data-style="trading"]) .detail-header,
  :global([data-style="modern"]) .detail-header { margin-bottom: 8px; }
}
</style>
