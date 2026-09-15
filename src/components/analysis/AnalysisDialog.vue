<script setup lang="ts">
// src/components/analysis/AnalysisDialog.vue
// 个股技术分析对话框：展示多因子评分、结论与关键指标快照。

import { computed, watch } from 'vue';
import { NModal, NTag, NSpin } from 'naive-ui';
import { useAnalysisStore } from '@/stores/analysis';
import { evaluateBacktest, tradeRuleFocus, tradeRuleLabel, verdictTone } from '@/types/analysis';
import PriceLevelChart from '@/components/analysis/PriceLevelChart.vue';

const props = defineProps<{
  show: boolean;
  symbol: string;
  name: string;
  /** 用哪套交易规则算买卖点。缺省趋势跟随；筛选器会把当前策略配套的规则传进来 */
  rule?: string;
}>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();

const store = useAnalysisStore();

const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

// 打开、换股票、换规则时都要重新分析 —— 规则会改变买点/止损/止盈与回测结果
watch(
  () => [props.show, props.symbol, props.rule] as const,
  ([open, sym, rule]) => {
    if (open && sym) void store.analyze(sym, rule);
  },
);

const plan = computed(() => store.analysis?.trade_plan ?? null);
const backtest = computed(() => store.analysis?.backtest ?? null);
const backtestVerdict = computed(() => (backtest.value ? evaluateBacktest(backtest.value) : null));

/** 筹码分布与支撑/压力位（后端已按当前规则加权排序） */
const chips = computed(() => store.analysis?.chips ?? null);
const levels = computed(() => store.analysis?.levels ?? []);
/** 当前规则最该盯哪类价位 —— 免得用户面对一堆价位不知道看重哪个 */
const ruleFocus = computed(() => tradeRuleFocus(store.ruleUsed));

/** 自动匹配到的规则与三条规则各自的状态 */
const ruleMatch = computed(() => store.analysis?.rule_match ?? null);
/** 当前用的是自动匹配还是用户手动指定 */
const isAuto = computed(() => store.ruleOverride === null);
/** 当前生效那条规则的状态（用于显示「为什么是它」） */
const activeCandidate = computed(
  () => ruleMatch.value?.candidates.find(c => c.rule === store.ruleUsed) ?? null,
);

/** 手动切到某条规则；再点一次已选中的那条即回到自动匹配 */
function switchRule(rule: string) {
  if (store.ruleOverride === rule) {
    void store.analyze(props.symbol, 'auto');
    return;
  }
  void store.analyze(props.symbol, rule);
}

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

/** 价格：统一两位小数，便于和行情软件对照 */
function price(v: number): string {
  return Number.isFinite(v) ? v.toFixed(2) : '-';
}

/** 带符号的数值（期望、累计收益等） */
function signed(v: number, digits = 2): string {
  if (!Number.isFinite(v)) return '-';
  const sign = v > 0 ? '+' : '';
  return `${sign}${v.toFixed(digits)}`;
}

function rate(v: number): string {
  return Number.isFinite(v) ? `${(v * 100).toFixed(0)}%` : '-';
}
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="个股技术分析"
    :style="{ width: 'min(760px, calc(100vw - 24px))' }"
    :content-style="{ maxHeight: 'calc(100vh - 120px)', overflow: 'auto' }"
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

          <!-- 交易规则：按当前市场状态自动匹配，也可以手动切换 -->
          <template v-if="ruleMatch">
            <div class="section-title plan-title">
              <span>交易规则</span>
              <span class="rule-tag">{{ ruleMatch.recommended_label }}</span>
              <span class="muted">
                {{ isAuto ? '按当前状态自动匹配' : '已手动指定' }}
              </span>
              <button
                v-if="!isAuto"
                class="link-btn"
                title="交还给按状态自动匹配"
                @click="store.analyze(props.symbol, 'auto')"
              >
                恢复自动
              </button>
            </div>

            <div class="rule-match">
              <div class="rule-reason">{{ ruleMatch.reason }}</div>

              <div class="rule-cands">
                <button
                  v-for="c in ruleMatch.candidates"
                  :key="c.rule"
                  class="rule-cand"
                  :class="{
                    ready: c.ready,
                    active: c.rule === store.ruleUsed,
                    recommended: c.rule === ruleMatch.recommended,
                  }"
                  :title="c.note"
                  @click="switchRule(c.rule)"
                >
                  <span class="rc-name">{{ c.rule_label }}</span>
                  <span class="rc-state">{{ c.ready ? '满足' : '未触发' }}</span>
                  <span class="rc-strength mono">{{ c.strength.toFixed(0) }}</span>
                </button>
              </div>

              <div v-if="activeCandidate" class="rule-note">
                <b>{{ activeCandidate.rule_label }}</b>：{{ activeCandidate.note }}
              </div>

              <div class="rule-caveat">
                选哪条规则看的是<b>当前处于什么状态</b>（趋势 / 超跌 / 突破），
                不是「历史上哪条赚得多」—— 后者实测选对率只有 40%，随机挑还有 33%，
                本质是在噪声里挑最大值。回测数字仅供参照，没有参与这里的判断。
              </div>
            </div>
          </template>

          <!-- 操作计划：把「选出来」变成「照着做」 -->
          <div class="section-title plan-title">
            <span>操作计划</span>
            <span class="rule-tag">{{ plan ? plan.rule_label : tradeRuleLabel(store.ruleUsed) }}</span>
            <span v-if="plan" class="ready-tag" :class="plan.ready ? 'ok' : 'wait'">
              {{ plan.ready ? '当前满足入场条件' : '当前未触发' }}
            </span>
          </div>

          <div v-if="plan" class="plan" :class="{ 'plan-wait': !plan.ready }">
            <div class="plan-levels">
              <div class="level">
                <span class="level-k">买入区间</span>
                <span class="level-v mono">{{ price(plan.buy_low) }} – {{ price(plan.buy_high) }}</span>
              </div>
              <div class="level">
                <span class="level-k">止损</span>
                <span class="level-v mono down">
                  {{ price(plan.stop_loss) }}<small>（-{{ plan.stop_pct.toFixed(1) }}%）</small>
                </span>
              </div>
              <div class="level">
                <span class="level-k">止盈</span>
                <span class="level-v mono up">{{ price(plan.take_profit) }}</span>
              </div>
              <div class="level">
                <span class="level-k">盈亏比</span>
                <span class="level-v mono">1 : {{ plan.risk_reward.toFixed(1) }}</span>
              </div>
              <div class="level">
                <span class="level-k">仓位上限</span>
                <span class="level-v mono">{{ plan.position_pct.toFixed(0) }}%</span>
              </div>
              <div class="level">
                <span class="level-k">参考价</span>
                <span class="level-v mono">{{ price(plan.reference_price) }}</span>
              </div>
            </div>

            <div v-if="!plan.ready && plan.waiting_for" class="plan-waiting">
              还没到买点：{{ plan.waiting_for }}
            </div>

            <div class="plan-profile">{{ plan.rule_profile }}</div>

            <ul class="plan-notes">
              <li v-for="(note, index) in plan.notes" :key="index">{{ note }}</li>
            </ul>
          </div>
          <div v-else-if="store.analysis" class="muted plan-missing">
            拿不到操作计划：日 K 不足（至少需要 15 根才能算出 ATR）或价格数据异常。宁可不给价位，也不编一个。
          </div>

          <!-- 价格位：筹码分布 + 支撑/压力位，权重随当前策略变 -->
          <template v-if="levels.length || chips">
            <div class="section-title plan-title">
              <span>价格位</span>
              <span class="rule-tag">{{ tradeRuleLabel(store.ruleUsed) }}</span>
              <span class="muted focus-hint">{{ ruleFocus }}</span>
            </div>
            <PriceLevelChart :close="store.analysis.close" :levels="levels" :chips="chips" />
          </template>

          <!-- 规则回测：让「胜率」落到这只股票自己的历史上 -->
          <template v-if="backtest">
            <div class="section-title plan-title">
              <span>规则历史回测</span>
              <span class="rule-tag">{{ backtest.rule_label }}</span>
              <span class="muted">过去 {{ backtest.bars }} 根日 K</span>
            </div>

            <div class="backtest" :class="`bt-${backtestVerdict?.tone ?? 'warn'}`">
              <div class="grid">
                <div class="cell"><span class="k">触发次数</span><span class="v mono">{{ backtest.trades }}</span></div>
                <div class="cell"><span class="k">胜率</span><span class="v mono">{{ rate(backtest.win_rate) }}</span></div>
                <div class="cell"><span class="k">盈亏比</span><span class="v mono">{{ backtest.payoff_ratio.toFixed(2) }}</span></div>
                <div class="cell">
                  <span class="k">每笔期望</span>
                  <span class="v mono" :class="backtest.expectancy_pct > 0 ? 'up' : 'down'">
                    {{ signed(backtest.expectancy_pct) }}%
                  </span>
                </div>
                <div class="cell"><span class="k">平均盈利</span><span class="v mono up">+{{ backtest.avg_win_pct.toFixed(2) }}%</span></div>
                <div class="cell"><span class="k">平均亏损</span><span class="v mono down">-{{ backtest.avg_loss_pct.toFixed(2) }}%</span></div>
                <div class="cell">
                  <span class="k">累计</span>
                  <span class="v mono" :class="backtest.total_return_pct > 0 ? 'up' : 'down'">
                    {{ signed(backtest.total_return_pct) }}%
                  </span>
                </div>
                <div class="cell"><span class="k">最大回撤</span><span class="v mono down">-{{ backtest.max_drawdown_pct.toFixed(1) }}%</span></div>
                <div class="cell"><span class="k">平均持仓</span><span class="v mono">{{ backtest.avg_hold_days.toFixed(1) }} 天</span></div>
              </div>

              <div class="bt-verdict">{{ backtestVerdict?.text }}</div>
              <div class="bt-caveat">
                只看胜率会误判：高胜率配低盈亏比照样亏钱。上面这几项要一起看，尤其是「每笔期望」。
              </div>
              <div class="bt-note muted">{{ backtest.note }}</div>
            </div>
          </template>

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

          <!-- 指标快照：只列彼此不重复的指标，重复/派生的项已删（见 types/analysis.ts） -->
          <div class="section-title">指标快照</div>
          <div class="grid">
            <div class="cell"><span class="k">现价</span><span class="v mono">{{ pct(store.analysis.close) }}</span></div>
            <div class="cell"><span class="k">MA20</span><span class="v mono">{{ pct(store.analysis.ma20) }}</span></div>
            <div class="cell"><span class="k">MA60</span><span class="v mono">{{ pct(store.analysis.ma60) }}</span></div>
            <div class="cell"><span class="k">MACD DIF</span><span class="v mono">{{ pct(store.analysis.macd_dif) }}</span></div>
            <div class="cell"><span class="k">RSI(12)</span><span class="v mono">{{ pct(store.analysis.rsi12) }}</span></div>
            <div class="cell"><span class="k">BOLL 上</span><span class="v mono">{{ pct(store.analysis.boll_upper) }}</span></div>
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

/* ── 操作计划 ── */
.plan-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.rule-tag {
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--color-accent-dim);
  color: var(--color-accent);
  font-weight: var(--font-weight-normal);
}
.ready-tag {
  padding: 1px 8px;
  border-radius: var(--radius-full);
  font-weight: var(--font-weight-normal);
}
/* 「可入场」用品牌蓝而不是红/绿 —— 红绿在本项目里表示涨跌，借用会误导 */
.ready-tag.ok {
  background: var(--color-accent-dim);
  color: var(--color-accent);
}
.ready-tag.wait {
  background: var(--color-bg-hover);
  color: var(--color-text-tertiary);
}
/* 当前规则最该盯哪类价位：比普通 muted 再弱一点，别抢标题 */
.focus-hint {
  font-weight: var(--font-weight-normal);
  opacity: 0.9;
}

/* ── 交易规则匹配 ── */
.rule-match {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md);
  background: var(--color-bg-card);
}
.rule-reason {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  line-height: 1.6;
}
.rule-cands {
  display: flex;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.rule-cand {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: 4px 10px;
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
  font-family: var(--font-sans);
  cursor: pointer;
  transition: background var(--transition-fast), color var(--transition-fast),
    border-color var(--transition-fast);
}
.rule-cand:hover {
  border-color: var(--color-accent-dim);
  color: var(--color-accent);
}
/* 当前生效的那条 */
.rule-cand.active {
  background: var(--color-accent-dim);
  border-color: var(--color-accent);
  color: var(--color-accent);
}
/* 「满足」用品牌蓝而不是红/绿 —— 红绿在本项目里表示涨跌 */
.rule-cand.ready .rc-state {
  color: var(--color-accent);
  font-weight: var(--font-weight-semibold);
}
.rule-cand:not(.ready) .rc-state {
  color: var(--color-text-tertiary);
}
.rc-strength {
  color: var(--color-text-tertiary);
  font-size: 11px;
}
.rule-note {
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
  line-height: 1.6;
}
.rule-note b {
  color: var(--color-text-secondary);
  font-weight: var(--font-weight-semibold);
}
.rule-caveat {
  padding: var(--space-2);
  border-left: 2px solid var(--color-border-0);
  border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
  background: var(--color-bg-hover);
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
  line-height: 1.6;
}
.rule-caveat b {
  color: var(--color-text-secondary);
  font-weight: var(--font-weight-semibold);
}
.link-btn {
  border: 0;
  background: transparent;
  color: var(--color-accent);
  font-size: var(--text-xs);
  cursor: pointer;
  padding: 0;
}
.plan {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  border: 1px solid var(--color-border-0);
  border-left: 3px solid var(--color-accent);
  border-radius: var(--radius-md);
  background: var(--color-bg-card);
}
.plan-wait {
  border-left-color: var(--color-warning);
}
.plan-levels {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: 0 var(--space-4);
}
.level {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: var(--space-2);
  padding: 3px 0;
  border-bottom: 1px dashed var(--color-border-0);
}
.level-k {
  flex-shrink: 0;
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}
.level-v {
  font-size: var(--text-sm);
}
.level-v small {
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
}
.plan-waiting {
  padding: var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-warning-bg);
  color: var(--color-warning);
  font-size: var(--text-xs);
  line-height: 1.6;
}
.plan-profile {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  line-height: 1.6;
}
.plan-notes {
  margin: 0;
  padding-left: 16px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.plan-notes li {
  font-size: var(--text-xs);
  color: var(--color-text-tertiary);
  line-height: 1.6;
}
.plan-missing {
  padding: var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-bg-card);
  line-height: 1.6;
}

/* ── 规则回测 ── */
.backtest {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  border: 1px solid var(--color-border-0);
  border-left: 3px solid var(--color-warning);
  border-radius: var(--radius-md);
  background: var(--color-bg-card);
}
.bt-good {
  border-left-color: var(--color-accent);
}
.bt-warn {
  border-left-color: var(--color-warning);
}
.bt-bad {
  border-left-color: var(--color-error);
}
.bt-verdict {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  line-height: 1.6;
}
.bt-caveat {
  font-size: var(--text-xs);
  color: var(--color-warning);
  line-height: 1.6;
}
.bt-note {
  line-height: 1.6;
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
