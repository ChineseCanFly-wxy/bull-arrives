<script setup lang="ts">
// src/components/analysis/AnalysisDialog.vue
// 个股技术分析对话框：展示多因子评分、结论与关键指标快照。

import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { NModal, NTag, NSpin } from 'naive-ui';
import { useAnalysisStore } from '@/stores/analysis';
import { useSettingsStore } from '@/stores/settings';
import { evaluateBacktest, tradeRuleFocus, tradeRuleLabel, verdictTone } from '@/types/analysis';
import PriceLevelChart from '@/components/analysis/PriceLevelChart.vue';
import AgentWorkbench from '@/components/analysis/AgentWorkbench.vue';
import HelpTooltip from '@/components/common/HelpTooltip.vue';

const props = defineProps<{
  show: boolean;
  symbol: string;
  name: string;
  /** 技术规则对照；默认按当前形态匹配，不代表模型或策略已获准入。 */
  rule?: string;
}>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();

const store = useAnalysisStore();
const settings = useSettingsStore();
const importedTeamTask = computed(() => store.interactiveTasks.find(task =>
  task.id === store.importedTeamTaskId && task.team
  && task.context_fingerprint === store.analysis?.agent_context_fingerprint
  && task.context_fingerprint === store.agentAnalysis?.context_fingerprint,
));
const importedTeamActivity = computed(() => importedTeamTask.value
  ? store.interactiveActivities[importedTeamTask.value.id] : undefined);
const liveMessages = computed(() => store.liveStatus?.events.reduce<{ seq: number; kind: string; text: string }[]>((messages, event) => {
  const last = messages.at(-1);
  if (event.kind === 'text_delta' && last?.kind === 'text_delta') last.text += event.text;
  else messages.push({ seq: event.seq, kind: event.kind, text: event.text });
  return messages;
}, []) ?? []);
const storeStyleWidth = computed(() => settings.visualStyle === 'classic' ? 760 : 900);
const storeStyleHeight = computed(() => settings.visualStyle === 'classic' ? 120 : 110);
const selectedTaskId = ref<string | null>(null);
const liveQuestion = ref('');
const aiExpanded = ref(false);
const evidenceExpanded = ref(false);
const technicalExpanded = ref(false);
const factorsExpanded = ref(false);
const statisticsExpanded = ref(false);
let taskTimer: ReturnType<typeof setInterval> | null = null;

watch(() => [props.show, props.symbol, selectedTaskId.value] as const, ([visible, , taskId]) => {
  if (taskTimer) { clearInterval(taskTimer); taskTimer = null; }
  if (visible && taskId) {
    void store.inspectInteractiveTask(taskId);
    taskTimer = setInterval(() => void store.inspectInteractiveTask(taskId), 3000);
  }
});
onBeforeUnmount(() => { if (taskTimer) clearInterval(taskTimer); });

async function sendLiveQuestion() {
  const question = liveQuestion.value.trim();
  if (!question) return;
  await store.askLiveAnalysis(question);
  if (!store.liveError) liveQuestion.value = '';
}

function confirmDeleteInteractiveTask(taskId: string) {
  const task = store.interactiveTasks.find(item => item.id === taskId);
  if (!task) return;
  const launched = task.state !== 'prepared';
  const message = launched
    ? task.live
      ? '请先结束应用内会话。确认清理此任务目录中的文件？应用数据库内的分析快照不会删除。'
      : '请先在 Claude Code 中结束该会话。确认会话已结束并清理此任务目录中的文件？应用数据库内的分析快照不会删除。'
    : '确认清理此任务目录中的文件？应用数据库内的分析快照不会删除。';
  if (window.confirm(message)) {
    if (selectedTaskId.value === taskId) selectedTaskId.value = null;
    void store.deleteInteractiveAnalysis(taskId, launched);
  }
}

const visible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
});

// 打开、换股票、换规则时都要重新分析 —— 规则会改变买点/止损/止盈与回测结果
watch(
  () => [props.show, props.symbol, props.rule] as const,
  ([open, sym, rule]) => {
    if (open && sym) {
      aiExpanded.value = false; evidenceExpanded.value = false;
      technicalExpanded.value = false; factorsExpanded.value = false; statisticsExpanded.value = false;
      void store.analyze(sym, rule);
      void store.loadAgentStatus();
    } else if (!open && (store.agentLoading || store.teamLoading)) {
      void store.cancelAgentAnalysis();
    }
  },
  // 自选股的分析弹窗是 `v-if="analysisRow"` 按需挂载的：首次挂载时
  // `show` 已经是 true，普通 watch 不会触发，于是直接落到「暂无分析结果」。
  // 统一在公共弹窗设为 immediate，任何当前或未来的按需挂载入口都不会再漏掉首次请求。
  { immediate: true },
);

const agentFieldLabel: Record<string, string> = {
  total_score: '综合评分', close: '收盘价', ma20: 'MA20', ma60: 'MA60', rsi12: 'RSI(12)',
  momentum20: '20 日动量', momentum60: '60 日动量', volume_ratio: '量比',
  buy_low: '买入下沿', buy_high: '买入上沿', stop_loss: '止损', take_profit: '止盈',
  risk_reward: '盈亏比', position_pct: '仓位上限', backtest_expectancy_pct: '回测每笔期望',
  backtest_max_drawdown_pct: '回测最大回撤', backtest_win_rate: '回测胜率',
  oos_expectancy_pct: '样本外期望', profit_probability: '盈利概率',
  research_status: '多年研究准入状态',
};
for (const [prefix,label] of [['breadth22','形态广度模型'],['index26','形态与指数模型']]) {
  for (const [suffix,description] of [['signal','该股观察身份'],['label_score','收益标签预测'],['train_net','前段组合净收益'],['validation_net','中段组合净收益'],['test_net','后段组合净收益'],['test_drawdown','后段最大回撤'],['double_cost_net','双成本后段净收益']]) {
    agentFieldLabel[`${prefix}_${suffix}`]=`${label} · ${description}`;
  }
}
for(let i=1;i<=12;i++) agentFieldLabel[`news_${String(i).padStart(2,'0')}`]=`近期原文 ${i}`;
const researchContext = computed(() => store.analysis?.research_context);
const modelOverview = computed(() => researchContext.value?.model_research.models.map(model => ({
  id: model.id, name: model.name,
  opinion: model.signal_status === 'positive_record' ? '正分观察' :
    model.signal_status === 'nonpositive_record' ? '零/负分，偏谨慎' :
    model.signal_status === 'date_mismatch' ? '日期不符，方向未知' : '无评分记录，方向未知',
})) ?? []);
const aiState = computed(() => store.agentLoading || store.teamLoading ? '正在分析' :
  store.agentAnalysis?.status === 'ready' ? '已有解读，点击查看' : '手动生成 · 点击展开');
function safeSourceUrl(value?: string | null): string | undefined {
  if(!value) return undefined;
  try {const url=new URL(value);return ['http:','https:'].includes(url.protocol)&&!url.username&&!url.password ? url.href : undefined;}catch{return undefined;}
}
function newsTime(value: number | string | null | undefined): string {
  if(value==null) return '未知';
  const date=new Date(value);return Number.isNaN(date.getTime())?String(value):date.toLocaleString('zh-CN');
}

const agentOperatorLabel: Record<string, string> = {
  lt: '低于', lte: '不高于', gt: '高于', gte: '不低于',
  cross_below: '下穿', cross_above: '上穿',
};
const agentConclusionLabel: Record<string, string> = { bullish: '偏强', neutral: '中性', bearish: '偏弱', cautious: '谨慎' };
const agentRoleLabel: Record<string, string> = { technical: '技术', bull: '多方', bear: '空方', risk: '风控' };

function invalidationLabel(item: { field: string; operator: string; reference_field: string }): string {
  return `${agentFieldLabel[item.field] ?? item.field} ${agentOperatorLabel[item.operator] ?? item.operator} ${agentFieldLabel[item.reference_field] ?? item.reference_field}`;
}

const plan = computed(() => store.analysis?.trade_plan ?? null);
const backtest = computed(() => store.analysis?.backtest ?? null);
const backtestVerdict = computed(() => (backtest.value ? evaluateBacktest(backtest.value) : null));

/** 支撑/压力位（后端已按当前规则加权排序） */
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
    title="个股模型与形态研究"
    :style="{ width: `min(${storeStyleWidth}px, calc(100vw - 24px))` }"
    :content-style="{ maxHeight: `calc(100dvh - ${storeStyleHeight}px)`, overflow: 'auto' }"
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
              <span class="muted">量化技术状态分 · 0–100（非胜率）</span>
            </div>
          </div>

          <div class="section-title plan-title">
            <span>模型证据与近期原文</span>
            <HelpTooltip label="模型研究使用说明">先看多年、多股组合证据，再看个股同日评分和近期原文。正负分是冻结模型的收益标签预测，不是上涨概率；缺少评分记录时方向未知。目前模型仍为研究观察。技术规则和单股回放用于对照。</HelpTooltip>
          </div>
          <div v-if="researchContext" class="model-overview">
            <span v-for="model in modelOverview" :key="model.id"><b>{{ model.name }}</b><small>{{ model.opinion }}</small></span>
            <p v-if="!modelOverview.length">当前没有可核对的模型评分。</p>
            <small>行情 {{ store.analysis.history?.end_date || '未知' }} · 研究观察，尚未生产准入 · 近期原文 {{ researchContext.recent_news.length }} 条</small>
          </div>
          <details v-if="researchContext" class="analysis-disclosure evidence-disclosure" :open="evidenceExpanded" @toggle="evidenceExpanded = ($event.target as HTMLDetailsElement).open">
            <summary><span>模型证据与近期原文详情</span><small>{{ evidenceExpanded ? '收起' : '点击展开' }}</small></summary>
          <div class="research-card">
            <p><b>探索观察 · 未生产准入</b> · 冻结 {{ researchContext.model_research.frozen_as_of || '未知' }} · 个股 {{ store.analysis.history?.end_date || '未知' }}</p>
            <p v-if="researchContext.model_research.error" class="error-line">{{ researchContext.model_research.error }}</p>
            <p v-if="researchContext.mainline_error" class="error-line">主线关联读取失败：{{ researchContext.mainline_error }}</p>
            <p v-if="researchContext.news_error" class="error-line">近期资讯读取失败：{{ researchContext.news_error }}</p>
            <div v-for="model in researchContext.model_research.models" :key="model.id" class="research-row">
              <b>{{ model.name }} · {{ model.holding_days }} 交易日标签</b>
              <small v-if="model.score_source">{{model.score_source}} · 评分截止 {{model.score_as_of||'未知'}} · 账户 #{{model.model_run_id}}</small>
              <span>{{ model.signal_status==='positive_record' ? '已登记正分观察' : model.signal_status==='nonpositive_record' ? '真实标签预测为零或负，方向偏谨慎' : model.signal_status==='date_mismatch' ? '日期不符，当前评分未知' : '无对应评分记录，当前方向未知' }}<template v-if="model.score!==null"> · 标签预测 {{ signed(model.score*100,4) }}%（非胜率）</template></span>
              <small>前段 {{ signed(model.performance.train.net_return_pct) }}% · 中段 {{ signed(model.performance.validation.net_return_pct) }}% · 后段 {{ signed(model.performance.test.net_return_pct) }}% · 后段回撤 {{ model.performance.test.max_drawdown_pct.toFixed(2) }}% · 双成本 {{ signed(model.performance.double_cost_return_pct) }}%</small>
              <small>{{ model.performance.unique_stocks }} 只成交股票 · {{ model.performance.closed_cycles }} 个平仓周期；这些是组合证据，不是该股预测胜率。</small>
            </div>
            <details v-if="researchContext.model_research.latest_model_scan?.models.length"><summary>本次筛选模型证据 · {{researchContext.model_research.latest_model_scan.as_of}}</summary><div v-for="model in researchContext.model_research.latest_model_scan.models" :key="model.model_id" class="research-row"><b>{{model.name}} · {{model.signal_status==='positive_record'?'满足条件':model.signal_status==='nonpositive_record'?'未过阈值':'该股无评分'}}</b><p>任务 #{{model.job_id}} · 原始分数 {{model.score ?? '未知'}} · 固定阈值 &gt; {{model.threshold}}</p><small>{{model.score_semantic}}。{{model.score_source}}</small></div></details>
            <p class="muted">{{ researchContext.legacy_rule_note }}</p>
            <details v-if="researchContext.financial_research"><summary>公告时点基本面 · {{researchContext.financial_research.available?'保守版本可读':'未知'}}</summary><p>{{researchContext.financial_research.message}}</p><p v-if="researchContext.financial_research.fields">报告期 {{researchContext.financial_research.fields.report_date}} · 公告 {{researchContext.financial_research.fields.first_notice_date}} · 修订 {{researchContext.financial_research.fields.revision_date}} · 可用日 {{researchContext.financial_research.fields.available_signal_date}}</p><p v-if="researchContext.financial_research.fields">营收累计同比 {{researchContext.financial_research.fields.revenue_yoy_pct??'未知'}}% · 利润累计同比 {{researchContext.financial_research.fields.profit_yoy_pct??'未知'}}% · 累计ROE {{researchContext.financial_research.fields.roe_pct??'未知'}}% · 经营现金流每股 {{researchContext.financial_research.fields.cash_per_share_cny??'未知'}}元 · 负债资产比例 {{researchContext.financial_research.fields.debt_assets_pct??'未知'}}%</p><p v-for="item in researchContext.financial_research.limitations" :key="item">{{item}}</p></details>
            <details><summary>近期相关原文（{{ researchContext.recent_news.length }} 条）</summary>
              <p class="muted">{{ researchContext.news_scope }}</p>
              <p v-if="!researchContext.recent_news.length">未采集到相关原文，利好利空仍未知。</p>
              <article v-for="item in researchContext.recent_news" :key="item.id" class="research-row">
                <b>{{ item.title }}</b><small>{{ item.source }} · 发布 {{ item.published_at ? newsTime(item.published_at) : item.published_date ? `${item.published_date}（仅日期，具体公开时点未知）` : '未知' }} · 采集 {{ newsTime(item.received_at) }}</small>
                <small v-if="item.published_after_market_asof===true">行情截止后的新信息，用于补充当前风险，不能回填截止日模型。</small>
                <small v-if="item.source_index_only">标题索引；原文正文尚未采集。</small>
                <p>{{ item.body || '只有标题索引，尚未读取全文。' }}</p>
                <a v-if="safeSourceUrl(item.url)" :href="safeSourceUrl(item.url)" target="_blank" rel="noopener noreferrer">核对来源原文</a>
              </article>
            </details>
            <details v-if="researchContext.model_research.limitations?.length"><summary>研究局限</summary><p v-for="item in researchContext.model_research.limitations" :key="item">{{ item }}</p></details>
          </div>
          </details>
          <details class="analysis-disclosure ai-disclosure" :open="aiExpanded" @toggle="aiExpanded = ($event.target as HTMLDetailsElement).open">
            <summary><span>AI 解读 · 模型、原文与反证</span><small>{{ aiState }}</small></summary>
          <div v-if="aiExpanded" class="agent-card">
            <p class="muted">手动生成后，先看当前结论，再看模型为何支持或反对、近期原文影响、等待条件和最强反证。每条判断会附本次快照的数值与日期。</p>
            <div class="agent-actions">
              <button
                class="agent-btn"
                :disabled="!store.agentStatus?.installed || !store.analysis.agent_context_fingerprint || store.agentLoading || store.teamLoading"
                :title="store.agentStatus?.installed ? '生成结构化解读' : store.agentStatus?.guidance"
                @click="store.analyzeWithAgent()"
              >
                {{ store.agentLoading ? '分析中…' : '生成 AI 解读' }}
              </button>
              <button class="agent-btn" :disabled="!store.agentStatus?.installed || !store.analysis.agent_context_fingerprint || store.agentLoading || store.teamLoading" @click="store.analyzeWithTeam">
                {{ store.teamLoading ? '多角色研判中…' : '多角度交叉研判' }}
              </button>
              <button v-if="store.agentLoading || store.teamLoading" class="link-btn" @click="store.cancelAgentAnalysis">中止</button>
              <span v-if="store.agentAnalysis?.cached" class="muted">同一数据时点已复用</span>
            </div>

            <div v-if="store.agentStatus && !store.agentStatus.installed" class="agent-unavailable">
              <b>{{ store.agentStatus.message }}</b>
              <span>{{ store.agentStatus.guidance }}</span>
            </div>
            <template v-if="store.agentAnalysis?.status === 'ready'">
              <details v-if="importedTeamTask" :key="importedTeamTask.id" class="team-discussion">
                <summary>Team 团队讨论记录 · {{ importedTeamTask.symbol }}</summary>
                <p class="muted">负责人整理的过程摘要，可查看角色分工、交叉质疑、分歧与风控收口；原始对话可在对应终端复核。下方最终结论使用同一行情快照，可能与单个 Agent 一致。</p>
                <pre v-if="importedTeamActivity?.progress" class="team-progress">{{ importedTeamActivity.progress }}</pre>
                <p v-else class="muted">暂未读到讨论记录，可点击刷新；如果仍为空，请让负责人将各方观点、相互质疑和最终取舍写入本次任务的 progress.md。</p>
                <button class="link-btn" @click="store.inspectInteractiveTask(importedTeamTask.id)">刷新讨论记录</button>
              </details>
              <p v-if="importedTeamTask" class="muted">Team 最终结论（已通过结构与证据字段校验）</p>
              <p class="agent-conclusion"><b>{{ agentConclusionLabel[store.agentAnalysis.conclusion || ''] || store.agentAnalysis.conclusion }}</b></p>
              <p v-if="store.agentAnalysis.summary" class="agent-summary">{{ store.agentAnalysis.summary }}</p>
              <div class="agent-confidence">模型主观确定性 {{ store.agentAnalysis.confidence }}%（非上涨概率） · {{ store.agentAnalysis.generated_at }}</div>
              <div v-for="(claim, index) in store.agentAnalysis.claims" :key="index" class="agent-claim">
                <b>{{ { support: '支持因素', risk: '主要风险', watch: '继续观察' }[claim.kind] }}</b>
                <p>{{ claim.text }}</p>
                <small v-for="item in claim.evidence" :key="item.field">{{ agentFieldLabel[item.field] ?? item.field }} = {{ item.value }} · {{ item.source }} · {{ item.as_of }}</small>
              </div>
              <div class="agent-evidence">
                <span v-for="item in store.agentAnalysis.evidence" :key="item.field">
                  <b>{{ agentFieldLabel[item.field] ?? item.field }}</b> {{ item.value }}
                  <small>{{ item.source }} · {{ item.as_of }}</small>
                </span>
              </div>
              <div class="agent-invalid"><b>失效条件</b><span v-for="item in store.agentAnalysis.invalidation_conditions" :key="`${item.field}:${item.operator}:${item.reference_field}`">· {{ invalidationLabel(item) }}</span></div>
            </template>
            <div v-else-if="store.agentAnalysis" class="agent-unavailable">
              <b>Agent 未完成，已降级为纯量化结果</b>
              <span>{{ store.agentAnalysis.error }}</span><span>{{ store.agentAnalysis.guidance }}</span>
            </div>
            <div v-if="store.teamError" class="agent-unavailable">多角色研判失败：{{ store.teamError }}</div>
            <div v-if="store.teamAnalysis" class="agent-team">
              <p v-if="store.teamAnalysis.status === 'cancelled'">多角色研判已中止，未保存预测。</p>
              <p><b>风控收口：{{ store.teamAnalysis.final_conclusion ? ({ bullish: '偏强', bearish: '偏弱', neutral: '中性', cautious: '谨慎' }[store.teamAnalysis.final_conclusion] ?? store.teamAnalysis.final_conclusion) : '无结论' }}</b><span v-if="store.teamAnalysis.confidence !== null"> · 主观确定性 {{ store.teamAnalysis.confidence }}%（非上涨概率）</span></p>
              <div v-for="role in store.teamAnalysis.roles" :key="`${role.round}:${role.role}`" class="agent-role" :class="{ failed: role.status === 'failed' }">
                <b>第 {{ role.round }} 轮 · {{ agentRoleLabel[role.role] ?? role.role }}</b>
                <span>{{ role.argument ?? role.error }}</span>
                <small v-if="role.evidence.length">{{ role.evidence.map(item => `${agentFieldLabel[item.field] ?? item.field}=${item.value} @ ${item.as_of}`).join('；') }}</small>
              </div>
              <small>真实校准：{{ store.teamAnalysis.calibration.verified }}/{{ store.teamAnalysis.calibration.required_verified }} 条，跨度 {{ store.teamAnalysis.calibration.span_days }}/{{ store.teamAnalysis.calibration.required_span_days }} 天 · {{ store.teamAnalysis.calibration.status === 'ready' ? '已就绪' : '观察中' }}</small>
            </div>
            <details class="ai-workbench"><summary>针对性提问、提示词与调用记录</summary>
            <AgentWorkbench v-if="store.analysis.agent_context_fingerprint" :fingerprint="store.analysis.agent_context_fingerprint" :busy="store.agentLoading || store.teamLoading" :installed="!!store.agentStatus?.installed" :visible="props.show" @ask="store.analyzeWithAgent($event)" />
            </details>
            <details class="advanced-ai"><summary>连续对话与 Claude 终端（高级）</summary>
            <section class="live-agent" aria-label="应用内 Claude Code 会话">
              <div class="interactive-intro"><b>应用内连续对话</b><span>发送后下方会持续显示 Claude Code 回复与工具名称，状态会提示本轮是否结束；这里不显示工具内容。每轮使用设置中的单次预算（默认 $10，最高 $50），账户额度另计。普通 Claude Code 设置会加载，需人工批准的工具请用可见终端。文本回复不会自动变成已校验的结构化分析。</span></div>
              <button class="agent-btn" :disabled="!store.agentStatus?.installed || store.liveBusy || store.liveStatus?.running" @click="store.startLiveAnalysis">新建应用内会话</button>
              <p v-if="store.liveError" class="error-line" role="alert">{{ store.liveError }}</p>
              <template v-if="store.liveStatus">
                <p class="muted">{{ store.liveStatus.running ? 'Claude Code 本轮正在运行' : '本轮未运行' }} · {{ store.liveStatus.can_resume ? '会话已由 CLI 确认' : '会话尚未确认' }}</p>
                <div class="live-feed" role="log" aria-live="polite" aria-relevant="additions">
                  <div v-for="event in liveMessages" :key="event.seq" :class="['live-line', event.kind]">{{ event.text }}</div>
                </div>
                <textarea v-model="liveQuestion" aria-label="向应用内 Claude Code 追问" placeholder="输入要追问的快照问题（最多 1000 字）" maxlength="1000" :disabled="store.liveStatus.running || store.liveBusy || !store.liveStatus.can_resume" />
                <div class="live-actions">
                  <button class="agent-btn" :disabled="!liveQuestion.trim() || store.liveStatus.running || store.liveBusy || !store.liveStatus.can_resume" @click="sendLiveQuestion">发送追问</button>
                  <button v-if="store.liveStatus.running" class="link-btn" @click="store.cancelLiveAnalysis">中断本轮</button>
                </div>
              </template>
            </section>
            <div class="interactive-agent">
              <div class="interactive-intro"><b>可见 Claude Code 终端</b><span>“Claude 个股对话”终端用于围绕当前快照追问；“Agent Team 风控负责人”终端负责召集技术、多方、空方队友并汇总结论。展开 Team 任务的“查看状态”可核对原生队友是否真的加入。每个终端都有股票和用途提示；首次打开时 Claude Code 可能要求信任应用任务根目录，请核对路径后确认。完成后回到对应任务点击“导入结果”；终端使用 Claude Code 自身的模型、权限和账户额度。</span></div>
              <button class="agent-btn" :disabled="!store.agentStatus?.installed || !store.analysis.agent_context_fingerprint || store.interactiveBusy" :title="store.agentStatus?.installed ? '启动可见的 Claude Code 交互会话' : store.agentStatus?.guidance" @click="store.startInteractiveAnalysis">
                {{ store.interactiveBusy ? '处理中…' : '打开 Claude Code 终端' }}
              </button>
              <button class="agent-btn" :disabled="!store.agentStatus?.installed || !store.analysis.agent_context_fingerprint || store.interactiveBusy" title="打开风控负责人终端；技术、多方、空方队友在该会话中协作" @click="store.startInteractiveTeam">打开原生 Agent Team 终端</button>
              <p v-if="store.interactiveNotice" class="muted" role="status">{{ store.interactiveNotice }}</p>
              <p v-if="store.interactiveError" class="error-line" role="alert">{{ store.interactiveError }}</p>
              <div v-if="store.historicalInteractiveResult" class="historical-result">
                <b>历史快照分析 · {{ store.historicalInteractiveResult.generated_at }}</b>
                <p>{{ store.historicalInteractiveResult.summary }}</p>
                <small>仅用于回顾，不代表当前行情；请使用当前快照重新分析。</small>
              </div>
              <div v-for="task in store.interactiveTasks" :key="task.id" class="interactive-task">
                <span><b>{{ task.team ? 'Agent Team' : task.live ? '应用内对话' : 'Claude 终端' }}</b> · {{ task.symbol }} · 数据 {{ task.as_of }} · {{ task.context_fingerprint === store.analysis?.agent_context_fingerprint ? '当前快照' : '历史快照' }} · {{ task.state === 'validated' ? '结果已校验' : task.live ? '应用内会话' : task.state === 'opened' ? '终端脚本已启动' : '任务文件已准备' }}</span>
                <button class="link-btn" :aria-expanded="selectedTaskId === task.id" @click="selectedTaskId = selectedTaskId === task.id ? null : task.id">{{ selectedTaskId === task.id ? '收起状态' : '查看状态' }}</button>
                <button v-if="task.live && task.context_fingerprint === store.analysis?.agent_context_fingerprint" class="link-btn" :disabled="store.liveBusy" @click="store.restoreLiveAnalysis(task.id)">查看应用内会话</button>
                <button v-if="task.state !== 'prepared' && !task.live" class="link-btn" :disabled="store.interactiveBusy" @click="store.resumeInteractiveAnalysis(task.id)">终端继续</button>
                <button class="link-btn" :disabled="store.interactiveBusy" @click="store.importInteractiveAnalysis(task.id)">导入结果</button>
                <button class="link-btn" :disabled="store.interactiveBusy" @click="confirmDeleteInteractiveTask(task.id)">清理任务文件</button>
                <div v-if="selectedTaskId === task.id" class="task-activity" role="status">
                  <template v-if="store.interactiveActivities[task.id]">
                    <b>{{ store.interactiveActivities[task.id].session_status || (task.live ? '应用内会话状态见上方' : '尚未确认终端进程已启动') }}</b>
                    <span v-if="store.interactiveActivities[task.id].team_status">{{ store.interactiveActivities[task.id].team_status }}</span>
                    <span>{{ store.interactiveActivities[task.id].output_present ? '已检测到待导入文件（内容尚未校验）' : '尚未检测到可导入文件' }}</span>
                    <span v-if="store.interactiveActivities[task.id].output_bytes !== null">{{ store.interactiveActivities[task.id].output_bytes }} 字节 · 修改于 {{ store.interactiveActivities[task.id].output_modified_at || '未知时间' }}</span>
                    <pre v-if="store.interactiveActivities[task.id].progress" class="team-progress">{{ store.interactiveActivities[task.id].progress }}</pre>
                    <span>上次检查：{{ store.interactiveActivities[task.id].last_checked_at }} · 实时运行和队友消息以终端为准。</span>
                  </template>
                  <span v-else>正在读取任务文件状态…</span>
                </div>
                <small>任务目录：{{ task.directory }}。任务文件会保留到你主动清理；{{ task.live ? '应用内会话的消息只在本次运行的应用进程内保存，重启后可继续提问，但历史消息不会恢复。' : '如果终端未启动，可在该目录手动运行 Claude Code。' }}清理任务文件不会删除应用数据库中的分析快照。</small>
              </div>
            </div>
            </details>
          </div>

          </details>

          <details class="analysis-disclosure statistics-disclosure" :open="statisticsExpanded" @toggle="statisticsExpanded = ($event.target as HTMLDetailsElement).open">
            <summary><span>因子与形态历史统计（可选）</span><small>辅助理解本股，不参与自动买卖</small></summary>
            <p class="statistics-description">统计本股历史因子和形态出现后 5 个交易日的价格变化。结果为全样本描述，未做滚动样本外检验，也未计交易成本；不代表当前上涨概率或可执行组合收益。模型自动交易请使用研究中心 → 模拟跟踪。</p>
          <div class="section-title plan-title">
            <span>本股历史 5 日统计</span>
            <button class="agent-btn" :disabled="store.researchLoading" @click="store.runResearch">
              {{ store.researchLoading ? '计算中…' : '计算历史统计' }}
            </button>
          </div>
          <div v-if="store.researchError" class="error-line">{{ store.researchError }}</div>
          <div v-if="store.research" class="stock-statistics-card">
            <div class="research-meta">
              <b>{{ store.research.bars }} 根日 K · 观察后续 {{ store.research.horizon_days }} 个交易日</b>
              <span :class="store.research.causal_audit_passed ? 'up' : 'down'">历史特征一致性检查{{ store.research.causal_audit_passed ? '通过' : '失败' }}</span>
              <span>{{ store.research.runtime }}</span>
            </div>
            <div v-for="factor in store.research.factors" :key="factor.id" class="research-row">
              <b>{{ factor.label }}</b>
              <span>Rank IC {{ factor.rank_ic === null ? '-' : signed(factor.rank_ic, 3) }} · {{ factor.samples }} 样本</span>
              <small>因子分组的后续收盘平均变化：{{ factor.quantiles.map(item => `Q${item.quantile} ${signed(item.avg_return_pct)}%`).join(' / ') }}</small>
            </div>
            <div v-for="pattern in store.research.patterns" :key="pattern.id" class="research-row">
              <b>{{ pattern.label }}</b>
              <span>{{ pattern.samples }} 次 · 历史上涨占比 {{ pattern.positive_probability === null ? '-' : rate(pattern.positive_probability) }}</span>
              <small>后续收盘平均变化 {{ pattern.avg_return_pct === null ? '-' : `${signed(pattern.avg_return_pct)}%` }} · 信号后最低价最大跌幅 {{ pattern.max_drawdown_pct === null ? '-' : `${pattern.max_drawdown_pct.toFixed(2)}%` }}</small>
            </div>
            <p class="statistics-description">特征一致性检查只核对历史特征是否读取未来数据；上面的跌幅按信号日收盘到后续最低价计算，不是交易账户回撤。</p>
            <div class="research-note">板块/市值分层：{{ store.research.stratification_note }}</div>
            <div class="research-note">盘中截止匹配：{{ store.research.intraday_note }}</div>
          </div>

          </details>

          <details class="analysis-disclosure technical-disclosure" :open="technicalExpanded" @toggle="technicalExpanded = ($event.target as HTMLDetailsElement).open">
            <summary><span>技术形态与规则对照</span><small>{{ tradeRuleLabel(store.ruleUsed) }} · {{ plan?.ready ? '规则已触发' : '规则待触发' }} · 点击查看</small></summary>
            <p class="rule-validation"><b>多股多年盈利验证：未准入</b><span>{{ backtest ? backtest.trust.eligible && backtest.causal_audit.passed ? '本股回放门禁已通过，仍需多股多年组合验证' : '本股回放门禁未通过，展开查看具体原因' : '本次未取得单股回放结果' }}</span></p>
          <!-- 交易规则：按当前市场状态自动匹配，也可以手动切换 -->
          <template v-if="ruleMatch">
            <div class="section-title plan-title">
              <span>当前形态匹配</span>
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
                三条规则用于描述当前的趋势、超跌或突破形态，并生成可核对的价格情景。
                自动匹配依据当前形态条件；本股回放只供对照，不代表规则已通过多年、多股的盈利验证。
              </div>
            </div>
          </template>

          <!-- 操作计划：把「选出来」变成「照着做」 -->
          <div class="section-title plan-title">
            <span>规则情景价位</span>
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

          <!-- 价格位：支撑/压力位，权重随当前策略变（v1.5.1 起不再画筹码分布） -->
          <template v-if="levels.length">
            <div class="section-title plan-title">
              <span>价格位</span>
              <span class="rule-tag">{{ tradeRuleLabel(store.ruleUsed) }}</span>
              <span class="muted focus-hint">{{ ruleFocus }}</span>
            </div>
            <PriceLevelChart :close="store.analysis.close" :levels="levels" />
          </template>

          <!-- 规则回测：让「胜率」落到这只股票自己的历史上 -->
          <template v-if="backtest">
            <div class="section-title plan-title">
              <span>本股规则历史回放</span>
              <span class="rule-tag">{{ backtest.rule_label }}</span>
              <span class="muted">{{ store.analysis?.history?.source_label || '在线历史' }} · {{ store.analysis?.history?.start_date || '—' }} → {{ store.analysis?.history?.end_date || '—' }} · {{ store.analysis?.history?.bars ?? backtest.bars }} 根 · 前复权</span>
            </div>

            <div v-if="store.analysis?.history?.warning" class="bt-history-warning">{{ store.analysis.history.warning }}</div>
            <div class="bt-audit" :class="{ failed: !backtest.causal_audit.passed }">
              {{ backtest.causal_audit.passed ? '因果审计通过' : '因果审计未通过' }} ·
              静态 {{ backtest.causal_audit.static_checks }} 项 · 动态 {{ backtest.causal_audit.dynamic_checks }} 个截点
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
              <div class="bt-trust-grid">
                <span>样本外 {{ backtest.trust.oos_trades }} 笔 / {{ signed(backtest.trust.oos_expectancy_pct) }}%</span>
                <span>盈利概率 {{ rate(backtest.trust.profit_probability) }}</span>
                <span>双倍成本 {{ signed(backtest.trust.doubled_cost_expectancy_pct) }}%</span>
                <span>基准超额 {{ backtest.trust.excess_return_pct == null ? '—' : `${signed(backtest.trust.excess_return_pct)}%` }}</span>
              </div>
              <div v-if="backtest.trust.cost_flip" class="bt-history-warning">成本翻倍后每笔期望由正转负，已自动拦截。</div>
              <details class="bt-details">
                <summary>本股回放门禁（{{ backtest.trust.gates.filter(gate => gate.passed).length }}/{{ backtest.trust.gates.length }} 通过）</summary>
                <div v-for="gate in backtest.trust.gates" :key="gate.key" class="bt-gate" :class="{ failed: !gate.passed }">
                  {{ gate.passed ? '✓' : '✕' }} {{ gate.label }}：{{ gate.detail }}
                </div>
              </details>
              <details class="bt-details">
                <summary>口径披露</summary>
                <div v-for="item in backtest.trust.methodology" :key="item" class="bt-method">· {{ item }}</div>
              </details>
              <div class="bt-caveat">
                只看胜率会误判：高胜率配低盈亏比照样亏钱。上面这几项要一起看，尤其是「每笔期望」。
              </div>
              <div class="bt-note muted">{{ backtest.note }}</div>
            </div>
          </template>

          </details>
          <details class="analysis-disclosure factor-disclosure" :open="factorsExpanded" @toggle="factorsExpanded = ($event.target as HTMLDetailsElement).open"><summary><span>技术评分明细与指标</span><small>点击查看因子、权重与指标快照</small></summary>
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
          </details>
        </template>

        <div v-else-if="!store.loading" class="muted">暂无分析结果</div>
      </n-spin>
    </div>
  </n-modal>
</template>

<style scoped>
.analysis-disclosure { margin: 12px 0; border: 1px solid var(--color-border-0); border-radius: 10px; background: var(--color-surface-1); padding: 0 14px; }
.analysis-disclosure > summary { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 6px; min-height: 48px; padding: 12px 0; font-size: 13px; font-weight: 600; color: var(--color-text-primary); cursor: pointer; list-style: none; }
.analysis-disclosure > summary::-webkit-details-marker { display: none; }
.analysis-disclosure > summary::before { content: '›'; font-size: 20px; line-height: 18px; color: var(--color-text-secondary); transition: transform 120ms; }
.analysis-disclosure[open] > summary::before { transform: rotate(90deg); }
.analysis-disclosure > summary > span { flex: 1; }
.analysis-disclosure > summary small { font-size: var(--text-xs); font-weight: 400; color: var(--color-text-secondary); }
.analysis-disclosure[open] { padding-bottom: 12px; }
.analysis-disclosure[open] > summary { border-bottom: 1px solid var(--color-border-0); margin-bottom: 12px; }
.ai-disclosure > summary > span { color: var(--color-accent); }
.ai-disclosure .agent-card { border: 0; padding: 0; }
.model-overview { display: flex; flex-wrap: wrap; gap: 8px; border: 1px solid var(--color-border-0); border-radius: 10px; padding: 12px 14px; background: var(--color-surface-1); }
.model-overview > span { flex: 1; min-width: 220px; padding: 9px 11px; background: var(--color-surface-2); border-radius: 7px; }
.model-overview b,.model-overview span small { display: block; }
.model-overview b { font-size: 12px; font-weight: 550; }.model-overview small { font-size: var(--text-xs); color: var(--color-text-secondary); }.model-overview > small { width: 100%; }.model-overview p { margin: 0; font-size: 12px; }
.rule-validation { display: grid; gap: 4px; padding: 10px 12px; border-radius: 7px; background: var(--color-warning-bg); font-size: 12px; color: var(--color-warning); }.rule-validation span { font-size: var(--text-xs); color: var(--color-text-secondary); }
.ai-workbench > summary,.advanced-ai > summary { cursor: pointer; color: var(--color-text-secondary); font-size: 12px; padding: 8px 0; }.advanced-ai > section,.advanced-ai > div { margin: 10px 0; }

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
  padding: var(--panel-padding);
  border: 1px solid var(--color-border-0);
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
  font-size: var(--text-xs);
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
.bt-history-warning { margin: -2px 0 8px; padding: 7px 10px; border-radius: var(--radius-sm); background: color-mix(in srgb, #d29922 12%, transparent); color: #d29922; font-size: var(--text-xs); }
.bt-audit { margin: -2px 0 8px; color: var(--color-down); font-size: var(--text-xs); }
.bt-audit.failed { color: var(--color-error); }
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
.agent-card { display: flex; flex-direction: column; gap: var(--space-3); border: 1px solid var(--color-border-0); border-radius: var(--radius-md); padding: var(--panel-padding); background: var(--color-surface-1); }
.interactive-agent { display: flex; flex-direction: column; align-items: flex-start; gap: var(--space-2); padding: var(--panel-padding); border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-2); }
.live-agent { display: grid; gap: var(--space-2); min-width: 0; padding: var(--panel-padding); border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-1); }
.live-agent > .agent-btn { justify-self: start; }
.live-feed { min-height: 80px; max-height: 260px; overflow: auto; padding: var(--space-2); background: var(--color-surface-0); font-size: var(--text-xs); line-height: 1.6; }
.live-line { white-space: pre-wrap; overflow-wrap: anywhere; }
.live-line + .live-line:not(.text_delta) { margin-top: var(--space-2); }
.live-line.user { color: var(--color-accent); }
.live-line.tool { color: var(--color-text-tertiary); }
.live-line.error { color: var(--color-error); }
.live-agent textarea { width: 100%; min-height: 64px; padding: var(--space-2); border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-0); color: var(--color-text-primary); resize: vertical; }
.live-actions { display: flex; gap: var(--space-2); align-items: center; }
html[data-style="classic"] .score-hero { border: 0; padding: var(--space-3); }
html[data-style="classic"] .agent-card { display: block; border-radius: var(--radius-sm); padding: 10px; background: transparent; }
html[data-style="classic"] .interactive-agent { gap: 8px; margin: 10px 0; padding: 12px; border-radius: var(--radius-sm); background: var(--color-bg-2); }
.interactive-intro { display: flex; flex-direction: column; gap: 4px; color: var(--color-text-secondary); font-size: var(--text-xs); line-height: 1.6; }
.interactive-intro b { color: var(--color-text-primary); font-size: var(--text-sm); }
.historical-result { padding: 10px; border-left: 2px solid var(--color-warning); background: var(--color-bg-card); color: var(--color-text-secondary); font-size: var(--text-xs); }
.historical-result p { margin: 6px 0; white-space: pre-wrap; }
.historical-result small { color: var(--color-warning); }
.interactive-task { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; width: 100%; padding-top: 8px; border-top: 1px solid var(--color-border-0); font-size: var(--text-xs); }
.interactive-task span { flex: 1; min-width: 160px; }
.task-activity { display: flex; flex-direction: column; gap: 4px; width: 100%; padding: 8px; border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-secondary); }
.task-activity span { min-width: 0; overflow-wrap: anywhere; }
.team-progress { width: 100%; max-height: 220px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; margin: 4px 0; font: inherit; }
.team-discussion { margin-top: 12px; padding: 10px; border: 1px solid var(--color-border-0); border-radius: var(--radius-sm); }
.team-discussion summary { cursor: pointer; font-weight: 600; }
.team-discussion .team-progress { max-height: 420px; line-height: 1.75; }
.interactive-task small { width: 100%; overflow-wrap: anywhere; color: var(--color-text-tertiary); }
.agent-actions { display: flex; align-items: center; gap: var(--space-2); }
html[data-style="modern"] .agent-actions { flex-wrap: wrap; }
@media (max-width: 620px) {
  .score-num { font-size: 36px; }
  .interactive-task span { min-width: 100%; }
  .agent-card { padding: var(--space-2); }
}
.agent-btn { border: 0; border-radius: var(--radius-sm); padding: 7px 12px; background: var(--color-accent); color: white; cursor: pointer; }
.agent-btn:disabled { opacity: .45; cursor: not-allowed; }
.agent-unavailable, .agent-invalid { display: flex; flex-direction: column; gap: 4px; margin-top: 8px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.agent-conclusion { margin: 10px 0 4px; line-height: 1.65; }
.agent-summary { line-height: 1.8; white-space: pre-wrap; }
.agent-claim { border-left: 3px solid var(--color-border-0); margin: 10px 0; padding: 6px 10px; }
.agent-claim p { margin: 5px 0; line-height: 1.7; }
.agent-claim small { display: block; color: var(--color-text-secondary); line-height: 1.7; }
.agent-confidence { color: var(--color-text-tertiary); font-size: var(--text-xs); }
.agent-evidence { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; margin-top: 8px; }
.agent-evidence span { display: flex; flex-direction: column; padding: 6px; border-radius: var(--radius-xs); background: var(--color-bg-2); }
.agent-evidence small { color: var(--color-text-tertiary); }
.agent-team{display:flex;flex-direction:column;gap:6px;margin-top:10px;padding-top:10px;border-top:1px solid var(--color-border-0);font-size:var(--text-xs)}.agent-team p{margin:0}.agent-role{display:grid;grid-template-columns:90px 1fr;gap:4px 8px;padding:6px;background:var(--color-bg-2);border-radius:var(--radius-xs)}.agent-role small{grid-column:2;color:var(--color-text-tertiary)}.agent-role.failed{opacity:.65}
.research-card,.stock-statistics-card{display:flex;flex-direction:column;gap:6px;padding:10px;border:1px solid var(--color-border-0);border-radius:var(--radius-sm);background:var(--color-bg-card)}.statistics-description{font-size:var(--text-xs);color:var(--color-text-secondary);line-height:1.7;margin:6px 0 10px;overflow-wrap:anywhere}.stock-statistics-card .research-row{grid-template-columns:minmax(0,120px) minmax(0,1fr)}.research-meta{display:flex;flex-wrap:wrap;gap:6px 14px;font-size:var(--text-xs)}.research-row{display:grid;grid-template-columns:120px 1fr;gap:3px 8px;padding:6px;background:var(--color-bg-2);border-radius:var(--radius-xs);font-size:var(--text-xs)}.research-row small{grid-column:2;color:var(--color-text-tertiary)}.research-note{font-size:var(--text-xs);color:var(--color-warning);line-height:1.5}
.bt-trust-grid { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: var(--text-xs); color: var(--color-text-secondary); }
.bt-details { font-size: var(--text-xs); color: var(--color-text-secondary); }
.bt-details summary { cursor: pointer; color: var(--color-text-primary); }
.bt-gate, .bt-method { margin-top: 5px; line-height: 1.5; }
.bt-gate { color: var(--color-down); }
.bt-gate.failed { color: var(--color-error); }

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
