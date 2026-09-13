<script setup lang="ts">
// src/components/analysis/AnalysisDialog.vue
// 个股技术分析对话框：展示多因子评分、结论与关键指标快照。

import { computed, watch } from 'vue';
import { NModal, NTag, NSpin } from 'naive-ui';
import { useAnalysisStore } from '@/stores/analysis';
import { verdictTone } from '@/types/analysis';

const props = defineProps<{
  show: boolean;
  symbol: string;
  name: string;
}>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();

const store = useAnalysisStore();

const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

// 打开且符号变化时自动分析
watch(
  () => [props.show, props.symbol] as const,
  ([open, sym]) => {
    if (open && sym) void store.analyze(sym);
  },
);

/** 分数条形颜色：高分偏红（看多），低分偏绿（看空） */
function barClass(score: number): string {
  if (score >= 60) return 'bar-up';
  if (score < 45) return 'bar-down';
  return 'bar-mid';
}

function scoreClass(score: number): string {
  if (score >= 60) return 'up';
  if (score < 45) return 'down';
  return 'flat';
}

function pct(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '-';
  return `${v.toFixed(2)}`;
}

function momentum(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '-';
  const sign = v > 0 ? '+' : '';
  return `${sign}${(v * 100).toFixed(1)}%`;
}
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="个股技术分析"
    :style="{ width: '640px' }"
    :bordered="false"
    size="small"
  >
    <div class="analysis">
      <div class="head">
        <span class="stock-name">{{ name || props.symbol }}</span>
        <span class="mono muted">{{ props.symbol }}</span>
      </div>

      <n-spin :show="store.loading">
        <div v-if="store.error" class="error-line">{{ store.error }}</div>

        <template v-else-if="store.analysis">
          <!-- 总分 + 结论 -->
          <div class="score-hero">
            <div class="score-num" :class="scoreClass(store.analysis.total_score)">
              {{ store.analysis.total_score }}
            </div>
            <div class="score-meta">
              <n-tag :type="verdictTone(store.analysis.verdict)" size="medium" :bordered="false">
                {{ store.analysis.verdict }}
              </n-tag>
              <span class="muted">综合评分（0–100）</span>
            </div>
          </div>

          <!-- 因子明细 -->
          <div class="section-title">因子明细</div>
          <div class="factors">
            <div v-for="f in store.analysis.factors" :key="f.name" class="factor">
              <div class="factor-row">
                <span class="factor-name">{{ f.name }}</span>
                <span class="factor-note muted">{{ f.note }}</span>
                <span class="factor-score mono" :class="scoreClass(f.score)">{{ f.score.toFixed(0) }}</span>
              </div>
              <div class="bar">
                <div class="bar-fill" :class="barClass(f.score)" :style="{ width: f.score + '%' }" />
              </div>
            </div>
          </div>

          <!-- 指标快照 -->
          <div class="section-title">指标快照</div>
          <div class="grid">
            <div class="cell"><span class="k">现价</span><span class="v mono">{{ pct(store.analysis.close) }}</span></div>
            <div class="cell"><span class="k">MA5</span><span class="v mono">{{ pct(store.analysis.ma5) }}</span></div>
            <div class="cell"><span class="k">MA10</span><span class="v mono">{{ pct(store.analysis.ma10) }}</span></div>
            <div class="cell"><span class="k">MA20</span><span class="v mono">{{ pct(store.analysis.ma20) }}</span></div>
            <div class="cell"><span class="k">MA60</span><span class="v mono">{{ pct(store.analysis.ma60) }}</span></div>
            <div class="cell"><span class="k">MACD DIF</span><span class="v mono">{{ pct(store.analysis.macd_dif) }}</span></div>
            <div class="cell"><span class="k">MACD DEA</span><span class="v mono">{{ pct(store.analysis.macd_dea) }}</span></div>
            <div class="cell"><span class="k">RSI(12)</span><span class="v mono">{{ pct(store.analysis.rsi12) }}</span></div>
            <div class="cell"><span class="k">KDJ K</span><span class="v mono">{{ pct(store.analysis.kdj_k) }}</span></div>
            <div class="cell"><span class="k">KDJ D</span><span class="v mono">{{ pct(store.analysis.kdj_d) }}</span></div>
            <div class="cell"><span class="k">KDJ J</span><span class="v mono">{{ pct(store.analysis.kdj_j) }}</span></div>
            <div class="cell"><span class="k">BOLL 上</span><span class="v mono">{{ pct(store.analysis.boll_upper) }}</span></div>
            <div class="cell"><span class="k">BOLL 中</span><span class="v mono">{{ pct(store.analysis.boll_mid) }}</span></div>
            <div class="cell"><span class="k">BOLL 下</span><span class="v mono">{{ pct(store.analysis.boll_lower) }}</span></div>
            <div class="cell"><span class="k">20 日动量</span><span class="v mono">{{ momentum(store.analysis.momentum20) }}</span></div>
            <div class="cell"><span class="k">60 日动量</span><span class="v mono">{{ momentum(store.analysis.momentum60) }}</span></div>
            <div class="cell"><span class="k">量比</span><span class="v mono">{{ pct(store.analysis.volume_ratio) }}</span></div>
          </div>
        </template>

        <div v-else-if="!store.loading" class="muted">暂无分析结果</div>
      </n-spin>
    </div>
  </n-modal>
</template>

<style scoped>
.analysis {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.head {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
}
.stock-name {
  font-size: var(--text-lg);
  font-weight: var(--font-weight-semibold);
}
.mono {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.muted {
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
}
.error-line {
  padding: var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-error-bg);
  color: var(--color-error);
  font-size: var(--text-xs);
}

.score-hero {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3);
  border-radius: var(--radius-md);
  background: var(--color-bg-card);
}
.score-num {
  font-size: 48px;
  font-weight: var(--font-weight-bold);
  font-family: var(--font-mono);
  line-height: 1;
}
.score-meta {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  align-items: flex-start;
}

.section-title {
  font-size: var(--text-xs);
  font-weight: var(--font-weight-semibold);
  color: var(--color-text-secondary);
  margin-bottom: var(--space-1);
}

.factors {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.factor-row {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  margin-bottom: 2px;
}
.factor-name {
  flex-shrink: 0;
  width: 44px;
  font-size: var(--text-xs);
  font-weight: var(--font-weight-semibold);
}
.factor-note {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.factor-score {
  flex-shrink: 0;
  width: 28px;
  text-align: right;
}
.bar {
  height: 4px;
  border-radius: var(--radius-full);
  background: var(--color-bg-hover);
  overflow: hidden;
}
.bar-fill {
  height: 100%;
  border-radius: var(--radius-full);
  transition: width var(--transition-fast);
}
.bar-up {
  background: var(--color-up);
}
.bar-down {
  background: var(--color-down);
}
.bar-mid {
  background: var(--color-warning);
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: var(--space-1) var(--space-4);
}
.cell {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  padding: 2px 0;
  border-bottom: 1px dashed var(--color-border-0);
}
.cell .k {
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}
.cell .v {
  font-size: var(--text-xs);
}

.up {
  color: var(--color-up);
}
.down {
  color: var(--color-down);
}
.flat {
  color: var(--color-text-secondary);
}
</style>
