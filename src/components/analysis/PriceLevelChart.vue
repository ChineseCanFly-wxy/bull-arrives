<script setup lang="ts">
// src/components/analysis/PriceLevelChart.vue
// 价格位图：把支撑位 / 压力位标在同一根价格轴上，一眼看清现价上下各有哪些位置。
//
// ⚠️ v1.5.1 起这里**不再画筹码分布**。原因：A 股没有公开的筹码原始数据，
// 各软件的"筹码峰"都是自家模型算的、互相之间对不上；实测东财也不提供该接口
// （push2 / push2his 的 cyq 路径均 404）。既然拿不到权威数据，就不摆一张
// 看起来像"真实持仓"的图 —— 详见 CHANGELOG v1.5.1。
//
// 现在图上的每一条线都是**价格自己走出来的**：均线、布林轨道、摆动高低点、
// 前 20 日高低点。它们不依赖任何估算。

import { computed } from 'vue';
import type { PriceLevel } from '@/types/analysis';

const props = defineProps<{
  /** 现价（参考价），没有就不画 */
  close: number | null;
  levels: PriceLevel[];
}>();

// ── 画布坐标（宽度固定 680，靠 width:100% 自适应容器宽度）──
const W = 680;
const H = 380;
const TOP = 26;
const BOTTOM = H - 26;
/** 价格轴的水平位置 */
const AXIS_X = 214;
/** 价格刻度文字右对齐到这个 x（轴左侧） */
const TICK_TEXT_X = AXIS_X - 10;
/** 价位标签起始 x */
const LABEL_X = AXIS_X + 18;
/** 两条标签之间至少留的垂直间距，避免文字叠在一起 */
const LABEL_GAP = 16;
/** 价格轴上的参考刻度条数（含上下限） */
const TICK_COUNT = 5;

const ready = computed(() => props.close != null && props.levels.length > 0);

const supportCount = computed(() => props.levels.filter(l => l.kind === 'support').length);
const resistanceCount = computed(() => props.levels.filter(l => l.kind === 'resistance').length);

/** 纵向价格范围：覆盖所有价位 + 现价，再留一点边距 */
const range = computed(() => {
  const prices: number[] = [];
  if (props.close != null) prices.push(props.close);
  for (const l of props.levels) prices.push(l.price);
  if (!prices.length) return null;
  let lo = Math.min(...prices);
  let hi = Math.max(...prices);
  if (!(hi > lo)) {
    hi = lo * 1.02;
    lo = lo * 0.98;
  }
  const pad = (hi - lo) * 0.06;
  return { lo: lo - pad, hi: hi + pad };
});

function yOf(price: number): number {
  const r = range.value;
  if (!r) return BOTTOM;
  const t = (price - r.lo) / (r.hi - r.lo);
  return BOTTOM - Math.min(1, Math.max(0, t)) * (BOTTOM - TOP);
}

function price(v: number): string {
  return Number.isFinite(v) ? v.toFixed(2) : '-';
}

/**
 * 价格轴刻度：把 [lo, hi] 均分。
 *
 * 不追求"整数关口"那种漂亮刻度 —— 这里的价格区间宽度只有百分之几，
 * 刻度只用来给眼睛一个尺度参照，均分反而更好读（间距一致）。
 */
const ticks = computed(() => {
  const r = range.value;
  if (!r) return [];
  return Array.from({ length: TICK_COUNT }, (_, i) => {
    const t = i / (TICK_COUNT - 1);
    const p = r.lo + (r.hi - r.lo) * t;
    return { price: p, y: yOf(p) };
  });
});

interface Placed {
  key: string;
  /** 标签文字的 y（做过避让） */
  y: number;
  /** 参考线本身的 y（= 真实价位） */
  lineY: number;
  kind: 'support' | 'resistance' | 'close';
  label: string;
  value: number;
  strength: number;
}

/**
 * 标签避让：按真实价位从上到下排，若与上一条挨得太近就往下推。
 * 不这么做的话，价位集中在窄区间时文字会糊成一团。
 */
const placed = computed<Placed[]>(() => {
  if (!range.value) return [];
  const items: Placed[] = props.levels.map(l => ({
    key: `${l.kind}-${l.price}-${l.source}`,
    y: yOf(l.price),
    lineY: yOf(l.price),
    kind: l.kind,
    label: l.label,
    value: l.price,
    strength: l.strength,
  }));
  if (props.close != null) {
    items.push({
      key: 'close',
      y: yOf(props.close),
      lineY: yOf(props.close),
      kind: 'close',
      label: '现价',
      value: props.close,
      strength: 0,
    });
  }
  items.sort((a, b) => a.lineY - b.lineY);
  let last = -Infinity;
  for (const item of items) {
    if (item.y - last < LABEL_GAP) item.y = last + LABEL_GAP;
    last = item.y;
  }
  return items;
});
</script>

<template>
  <div class="plc">
    <div v-if="!ready" class="plc-empty">
      没有足够数据画出价格位图（需要日 K 与至少一个支撑/压力位）。
    </div>

    <template v-else>
      <svg :viewBox="`0 0 ${W} ${H}`" width="100%" role="img">
        <title>支撑位与压力位</title>
        <desc>
          同一根价格轴上的支撑位与压力位：上方虚线为压力，下方虚线为支撑，深色实线为现价。
          每个价位都来自均线、布林轨道、摆动高低点或前 20 日高低点。
        </desc>

        <!-- 价格刻度：给眼睛一个尺度参照 -->
        <g class="ticks">
          <template v-for="t in ticks" :key="`tick-${t.price}`">
            <line :x1="AXIS_X" :x2="AXIS_X + 6" :y1="t.y" :y2="t.y" />
            <text :x="TICK_TEXT_X" :y="t.y + 4" text-anchor="end">{{ price(t.price) }}</text>
          </template>
        </g>

        <!-- 价格轴 -->
        <line :x1="AXIS_X" :x2="AXIS_X" :y1="TOP - 8" :y2="BOTTOM + 8" class="axis" />

        <!-- 支撑 / 压力位 -->
        <g v-for="item in placed" :key="item.key">
          <line
            :x1="AXIS_X"
            :x2="W - 36"
            :y1="item.lineY"
            :y2="item.lineY"
            :class="['lv', `lv-${item.kind}`]"
            :style="{ opacity: item.kind === 'close' ? 1 : 0.35 + Math.min(item.strength, 100) / 160 }"
          />
          <circle
            :cx="AXIS_X"
            :cy="item.lineY"
            r="2.5"
            :class="`dot-${item.kind}`"
          />
          <text
            :x="LABEL_X"
            :y="item.y + 4"
            :class="['lv-text', `tx-${item.kind}`]"
          >
            {{ item.label }} {{ price(item.value) }}
          </text>
        </g>

        <!-- 文字标注 -->
        <text :x="AXIS_X + 18" :y="TOP - 8" class="hint">价位由价格自身走出，不含筹码估算</text>
      </svg>

      <!-- 说明与明细 -->
      <div class="plc-meta">
        <span class="plc-stats">
          现价 <b class="mono">{{ price(close ?? 0) }}</b>
          · 上方压力 <b>{{ resistanceCount }}</b> 个
          · 下方支撑 <b>{{ supportCount }}</b> 个
        </span>
      </div>

      <ul class="plc-list">
        <li v-for="item in placed.filter(i => i.kind !== 'close')" :key="`n-${item.key}`">
          <span class="tag" :class="item.kind">
            {{ item.kind === 'resistance' ? '压力' : '支撑' }}
          </span>
          <span class="mono price">{{ price(item.value) }}</span>
          <span class="src">{{ item.label }}</span>
          <span class="strength">强度 {{ item.strength.toFixed(0) }}</span>
        </li>
      </ul>

      <div class="caveat">
        <div>
          支撑/压力位是<b>参考区间</b>，不是精确点位；强度分只表示"相对更值得看"，
          不代表"到这里一定会停"。图上标出的每一条都来自<b>价格自身</b>——
          均线、布林轨道、摆动高低点、前 20 日高低点。
        </div>
        <div class="dim">
          刻意<b>不提供筹码分布</b>：A 股没有公开的筹码原始数据，各软件的"筹码峰"都是
          自家模型算的、互相之间对不上，摆出来容易让人当成真实持仓。
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.plc {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.plc-empty {
  padding: var(--space-4);
  text-align: center;
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  background: var(--color-bg-card);
  border-radius: var(--radius-md);
}

/* ── SVG ── */
.ticks line {
  stroke: var(--color-border-0);
  stroke-width: 1;
  opacity: 0.8;
}
.ticks text {
  font-size: 11px;
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  fill: var(--color-text-tertiary);
}
.axis {
  stroke: var(--color-border-0);
  stroke-width: 0.5;
}
.lv {
  stroke-width: 0.5;
  stroke-dasharray: 5 4;
}
.lv-close {
  stroke: var(--color-text-primary);
  stroke-width: 1.5;
  stroke-dasharray: none;
}
.lv-resistance {
  stroke: var(--color-warning);
}
.lv-support {
  stroke: var(--color-accent);
}
.dot-close {
  fill: var(--color-text-primary);
}
.dot-resistance {
  fill: var(--color-warning);
}
.dot-support {
  fill: var(--color-accent);
}
.lv-text {
  font-size: 12.5px;
  font-family: var(--font-sans);
}
.tx-close {
  fill: var(--color-text-primary);
  font-size: 13px;
  font-weight: var(--font-weight-semibold);
}
.tx-resistance {
  fill: var(--color-warning);
}
.tx-support {
  fill: var(--color-accent);
}
.hint {
  font-size: 11.5px;
  fill: var(--color-text-tertiary);
  opacity: 0.85;
}

/* ── 说明区 ── */
.plc-meta {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
}
.plc-meta b {
  color: var(--color-text-primary);
  font-weight: var(--font-weight-semibold);
}
.plc-meta .mono {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.plc-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.plc-list li {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  font-size: var(--text-xs);
}
.tag {
  flex-shrink: 0;
  padding: 1px 5px;
  border-radius: var(--radius-sm);
  border: 1px solid currentColor;
  font-size: 11px;
}
.tag.resistance {
  color: var(--color-warning);
}
.tag.support {
  color: var(--color-accent);
}
.plc-list .price {
  font-family: var(--font-mono);
  color: var(--color-text-primary);
  min-width: 62px;
}
.plc-list .src {
  color: var(--color-text-secondary);
  /* 来源名可能由多个来源拼成，占主要宽度 */
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.plc-list .strength {
  color: var(--color-text-tertiary);
  flex-shrink: 0;
}
.caveat {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: var(--space-2);
  border-left: 2px solid var(--color-border-0);
  border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
  background: var(--color-bg-card);
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  line-height: 1.6;
}
.caveat b {
  color: var(--color-text-secondary);
  font-weight: var(--font-weight-semibold);
}
.caveat .dim {
  opacity: 0.85;
}
</style>
