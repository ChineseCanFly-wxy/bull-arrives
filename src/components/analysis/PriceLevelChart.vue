<script setup lang="ts">
// src/components/analysis/PriceLevelChart.vue
// 价格位图：把「筹码分布」与「支撑/压力位」叠在同一根价格轴上。
//
// 为什么叠在一起看 —— 两者本来就是一回事的两面：
// 筹码分布画的是"每个价位上堆了多少持仓"，而筹码密集处天然就是支撑/压力位。
// 分开画两张图，用户还得自己在脑子里对齐价格，不如叠起来直观。
//
// ⚠️ 筹码是**模型估算**，不是交易所数据。界面上必须写清楚，不能让人以为是"真实持仓"。

import { computed } from 'vue';
import type { ChipDistribution, PriceLevel } from '@/types/analysis';
import { CHIP_RATE_BASIS_LABEL } from '@/types/analysis';

const props = defineProps<{
  /** 现价（参考价），没有就不画 */
  close: number | null;
  levels: PriceLevel[];
  chips: ChipDistribution | null;
}>();

// ── 画布坐标（宽度固定 680，靠 width:100% 自适应容器宽度）──
const W = 680;
const H = 380;
const TOP = 26;
const BOTTOM = H - 26;
/** 筹码直方图的基线 —— 同时也是价格轴的水平位置 */
const BASELINE = 300;
/** 筹码条最大长度（占比最大的档位对应这么宽） */
const CHIP_MAX_W = 150;
/** 标签起始 x */
const LABEL_X = 322;
/** 两条标签之间至少留的垂直间距，避免文字叠在一起 */
const LABEL_GAP = 16;

const ready = computed(
  () => props.close != null && (props.levels.length > 0 || (props.chips?.bins.length ?? 0) > 0),
);

/** 纵向价格范围：覆盖筹码全区间 + 所有价位 + 现价，再留一点边距 */
const range = computed(() => {
  const prices: number[] = [];
  if (props.close != null) prices.push(props.close);
  for (const l of props.levels) prices.push(l.price);
  const bins = props.chips?.bins ?? [];
  if (bins.length) {
    prices.push(bins[0].price, bins[bins.length - 1].price);
  }
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

/** 占比最大的档位用来归一化筹码条长度 */
const maxRatio = computed(() => {
  const bins = props.chips?.bins ?? [];
  return bins.reduce((m, b) => Math.max(m, b.ratio), 0) || 1;
});

/**
 * 筹码分布的轮廓路径（面积图）。
 * 从基线出发沿每个档位向左画到对应长度，再回到基线闭合。
 */
const chipPath = computed(() => {
  const bins = props.chips?.bins ?? [];
  if (bins.length < 2 || !range.value) return '';
  const parts: string[] = [`M ${BASELINE} ${yOf(bins[0].price)}`];
  for (const b of bins) {
    const x = BASELINE - (b.ratio / maxRatio.value) * CHIP_MAX_W;
    parts.push(`L ${x.toFixed(1)} ${yOf(b.price).toFixed(1)}`);
  }
  parts.push(`L ${BASELINE} ${yOf(bins[bins.length - 1].price)}`);
  parts.push('Z');
  return parts.join(' ');
});

/** 筹码峰的水平标记（在图左侧画一小段横线，指出峰在哪） */
const peakMarks = computed(() => {
  const peaks = props.chips?.peaks ?? [];
  return peaks
    .filter(p => p.price >= (range.value?.lo ?? 0) && p.price <= (range.value?.hi ?? 0))
    .map(p => ({ price: p.price, y: yOf(p.price), ratio: p.ratio }));
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
  const items: Placed[] = props.levels.map((l: PriceLevel) => ({
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

const rateBasisNote = computed(() =>
  props.chips ? CHIP_RATE_BASIS_LABEL[props.chips.rate_basis] : '',
);
</script>

<template>
  <div class="plc">
    <div v-if="!ready" class="plc-empty">
      没有足够数据画出价格位图（需要日 K 与至少一个支撑/压力位）。
    </div>

    <template v-else>
      <svg :viewBox="`0 0 ${W} ${H}`" width="100%" role="img">
        <title>筹码分布与支撑压力位</title>
        <desc>
          左灰色区域是估算的筹码分布（每个价位上的持仓量），右侧横线标出各支撑位与压力位，深色横线为现价。
        </desc>

        <!-- 筹码分布轮廓 -->
        <path v-if="chipPath" :d="chipPath" class="chip-area" />

        <!-- 筹码峰的水平短标 -->
        <g class="peak-marks">
          <line
            v-for="p in peakMarks"
            :key="`pk-${p.price}`"
            :x1="BASELINE - 172"
            :x2="BASELINE - 158"
            :y1="p.y"
            :y2="p.y"
            stroke-width="2"
          />
        </g>

        <!-- 价格轴 -->
        <line :x1="BASELINE" :x2="BASELINE" :y1="TOP - 8" :y2="BOTTOM + 8" class="axis" />

        <!-- 支撑 / 压力位 -->
        <g v-for="item in placed" :key="item.key">
          <line
            :x1="item.kind === 'close' ? BASELINE : 120"
            :x2="W - 36"
            :y1="item.lineY"
            :y2="item.lineY"
            :class="['lv', `lv-${item.kind}`]"
            :style="{ opacity: item.kind === 'close' ? 1 : 0.35 + Math.min(item.strength, 100) / 160 }"
          />
          <circle
            :cx="item.kind === 'close' ? BASELINE : 120"
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

        <!-- 文字标注：筹码区 -->
        <text :x="60" :y="TOP + 12" class="hint">估算筹码分布</text>
        <text :x="60" :y="TOP + 30" class="hint-dim">每个价位上的持仓量</text>
      </svg>

      <!-- 说明与明细 -->
      <div class="plc-meta">
        <span v-if="chips" class="chip-stats">
          获利盘 <b>{{ (chips.profit_ratio * 100).toFixed(0) }}%</b>
          · 平均成本 <b class="mono">{{ price(chips.avg_cost) }}</b>
          · 90% 筹码落在 <b class="mono">{{ price(chips.cost_90_low) }}–{{ price(chips.cost_90_high) }}</b>
          · 集中度 <b>{{ chips.concentration_90.toFixed(1) }}%</b>
        </span>
        <span v-else class="hint-dim">筹码分布：数据不足，未计算</span>
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
          筹码分布是<b>按日 K 与换手率推算的模型估算</b>，不是交易所公布的持仓数据 ——
          各券商软件用的参数不同，互相之间也对不上，所以这里的数字不必和别的软件完全一致。
        </div>
        <div v-if="rateBasisNote" class="dim">换手率口径：{{ rateBasisNote }}</div>
        <div class="dim">
          支撑/压力位是<b>参考区间</b>，不是精确点位；强度分只表示"相对更值得看"，
          不代表"到这里一定会停"。窗口内有解禁、增发或大比例送转时，估算偏差会明显变大。
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
.chip-area {
  fill: var(--color-border-0);
  fill-opacity: 0.55;
  stroke: var(--color-border-0);
  stroke-width: 0.5;
}
.peak-marks line {
  stroke: var(--color-text-tertiary);
  opacity: 0.7;
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
  font-size: 12px;
  fill: var(--color-text-tertiary);
}
.hint-dim {
  font-size: 11.5px;
  fill: var(--color-text-tertiary);
  opacity: 0.8;
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
