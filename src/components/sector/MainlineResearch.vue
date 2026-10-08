<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { NButton, NEmpty, NSpin, NTag } from 'naive-ui';
import type { SectorKind } from '@/types/sector';

interface SourceNews { id: string; title: string; body: string; source: string; published_at?: string | null; published_date?: string | null; publication_precision?: string | null; received_at?: string | number | null; url?: string | null; source_url?: string | null; published_after_market_asof?: boolean | null; source_index_only?: boolean; match_basis?: string; coverage_note?: string }
interface ResearchModel { id: string; name: string; feature_count: number; holding_days: number; signal_status: string; score: number | null; score_as_of?: string | null; observation?: { signal_eligible?: boolean } | null; performance: { train?: Record<string, unknown>; validation?: Record<string, unknown>; test?: Record<string, unknown>; double_cost_return_pct?: number; status?: string } }
interface ModelResearch { schema: string; date_matches?: boolean; requested_as_of?: string; frozen_as_of?: string; evidence_as_of?: string; run_id?: string; snapshot_sha256?: string; screen_sha256?: string; evidence_sha256?: string; production_admission: boolean; models: ResearchModel[]; limitations: string[] }
interface MissingMember { symbol: string; reason: string }
interface ReferencePrices { as_of: string; close: number; ma20: number; atr: number; prior_high20: number }
interface ReferenceRange { buy_low: number; buy_high: number; condition: string }
interface ReferencePlan extends Record<string, unknown> {
  status?: string; reference_prices?: ReferencePrices; trial?: ReferenceRange; confirm?: ReferenceRange;
  profit_trim?: { sell_position_pct: number; atr_above_cost: number; trigger: string };
  weakness_trim?: { sell_position_pct: number; trigger_price: number; condition: string };
  exit?: { atr_from_cost: number; preferred_days: number; max_holding_days: number; review_range_days: number[]; condition?: string };
}
interface Leader { strong?: boolean; symbol: string; name: string; as_of: string | number; score: number; r20: number; r60: number; rs20_vs_hs300?: number; rs60_vs_hs300?: number; ma20: number; ma60: number; atr: number; close: number; plan: ReferencePlan; model_research?: ModelResearch }
interface MainlineResult {
  sector_code: string; sector_name: string; as_of: string | number; sector_source: string;
  member_as_of: string | number; member_source: string; total_members: number; excluded_members: number;
  covered_members: number; missing_members: MissingMember[]; complete: boolean; strong: boolean; status: string;
  metrics: Record<string, unknown>; leaders: Leader[]; news: SourceNews[]; limitations: string[];
  fingerprint: string; generated_at: string; content_sha256?: string; model_research?: ModelResearch;
  trend_breadth?: number; trending_members?: number; benchmark?: { symbol: string; source: string; as_of: string };
  membership_observed_at?: string; membership_source_is_current_snapshot?: boolean;
}
interface Claim { text: string; source_ids: string[]; evidence?: string }
interface Summary { overview: string; positives: Claim[]; negatives: Claim[]; uncertainties: string[] }
interface WatchItem { kind: SectorKind; code: string; name: string }
const props = defineProps<{ kind: SectorKind; sectorCode: string; sectorName: string; snapshotFingerprint?: string }>();
const data = ref<MainlineResult | null>(null);
const summary = ref<Summary | null>(null);
const loading = ref(false); const analyzing = ref(false); const savingWatch = ref(false);
const leaderDisplayOptions = [3, 5, 10];
const leaderDisplayStorageKey = 'mainline-leader-display-count-v1';
function readLeaderDisplayCount() {
  try {
    const value = Number(localStorage.getItem(leaderDisplayStorageKey));
    return leaderDisplayOptions.includes(value) ? value : 10;
  } catch { return 10; }
}
const leaderDisplayCount = ref(readLeaderDisplayCount());
watch(leaderDisplayCount, value => { try { localStorage.setItem(leaderDisplayStorageKey, String(value)); } catch {} });
const watched = ref(false); const error = ref(''); const analysisError = ref(''); const watchError = ref('');
const holdingCosts = ref<Record<string, string>>({});
const referenceClock = ref(Date.now());
// Only refresh the age label locally; no quotes, signals or trade monitoring.
const referenceClockTimer = setInterval(() => { referenceClock.value = Date.now(); }, 60_000);
let epoch = 0;
function day(value: string | number | undefined) {
  const valueDay = String(value ?? '').match(/^(\d{8}|\d{4}-\d{2}-\d{2})(?:$|T|\s)/)?.[1].replace(/-/g, '') ?? '';
  if (valueDay.length !== 8) return '';
  const iso = valueDay.slice(0, 4) + '-' + valueDay.slice(4, 6) + '-' + valueDay.slice(6);
  const parsed = new Date(iso + 'T00:00:00Z');
  return Number.isFinite(parsed.getTime()) && parsed.toISOString().slice(0, 10) === iso ? valueDay : '';
}
function date(value: string | number | undefined) { const valueDay = day(value); return valueDay.length === 8 ? `${valueDay.slice(0, 4)}-${valueDay.slice(4, 6)}-${valueDay.slice(6)}` : '--'; }
function number(value: unknown, suffix = '') { return typeof value === 'number' && Number.isFinite(value) ? `${value.toFixed(2)}${suffix}` : '--'; }
const eligibleCount = computed(() => Math.max(0, (data.value?.total_members ?? 0) - (data.value?.excluded_members ?? 0)));
const datesMatch = computed(() => !!data.value && day(data.value.as_of).length === 8
  && day(data.value.as_of) === day(data.value.member_as_of)
  && data.value.benchmark?.symbol === 'sh000300' && day(data.value.benchmark.as_of) === day(data.value.as_of));
const complete = computed(() => !!data.value && data.value.complete && datesMatch.value && eligibleCount.value > 0
  && data.value.covered_members === eligibleCount.value && data.value.missing_members.length === 0
  && data.value.leaders.every(leader => day(leader.as_of) === day(data.value?.as_of)));
function positive(value: unknown) { return typeof value === 'number' && Number.isFinite(value) && value > 0; }
const strong = computed(() => complete.value && data.value?.strong === true
  && positive(data.value.metrics.rs20_vs_hs300) && positive(data.value.metrics.rs60_vs_hs300)
  && typeof data.value.trend_breadth === 'number' && Number.isFinite(data.value.trend_breadth)
  && data.value.trend_breadth >= 0.5 && data.value.trend_breadth <= 1);
const leaders = computed(() => {
  const result = data.value;
  if (!result || !datesMatch.value) return [];
  return result.leaders.filter(leader => day(leader.as_of) === day(result.as_of) && leader.strong !== false
    && positive(leader.rs20_vs_hs300) && positive(leader.rs60_vs_hs300) && positive(leader.r20) && Number.isFinite(leader.r60)
    && typeof result.metrics.r20 === 'number' && leader.r20 > result.metrics.r20
    && positive(leader.atr) && positive(leader.close) && positive(leader.ma20) && positive(leader.ma60)
    && leader.close > leader.ma20 && leader.ma20 > leader.ma60 && (leader.close - leader.ma20) / leader.atr <= 2).slice(0, 10);
});
const leaderEmptyDescription = computed(() => !datesMatch.value
  ? '板块、成分或沪深300日期未通过同日核验，暂不展示个股候选。'
  : (data.value?.leaders.length
    ? '返回个股未通过同日行情、20/60日相对强势或MA20/ATR条件，暂不展示。'
    : '已核验成分中暂无同时满足20/60日相对强势、趋势和MA20/ATR条件的个股候选。')
    + (complete.value ? '' : ' 未覆盖成分尚无结论，缺失不等于弱势。'));
const visibleLeaders = computed(() => leaders.value.slice(0, leaderDisplayCount.value).map(leader => ({ ...leader, reference: referencePoints(leader) })));
const referenceContext = computed(() => {
  if (props.snapshotFingerprint) return { state: 'historical', label: '历史快照 · 仅回看当时价位' };
  if (loading.value) return { state: 'pending', label: '刷新中 · 暂看上次价位' };
  if (error.value) return { state: 'stale', label: '刷新失败 · 仅旧结果参考' };
  const today = new Date(referenceClock.value + 8 * 60 * 60_000).toISOString().slice(0, 10);
  if (day(data.value?.as_of) > day(today)) return { state: 'stale', label: '行情日期在未来 · 请刷新核验' };
  const generated = Date.parse(data.value?.generated_at ?? '');
  if (!Number.isFinite(generated) || generated > referenceClock.value + 60_000) return { state: 'stale', label: '生成时间未通过核验 · 请刷新' };
  if (referenceClock.value - generated > 24 * 60 * 60_000) return { state: 'stale', label: '超过24小时未刷新 · 仅旧结果参考' };
  if (!complete.value) return { state: 'partial', label: '个股形态参考 · 板块待确认' };
  if (!strong.value) return { state: 'weak', label: '主线条件未通过 · 暂不建议新开仓' };
  return { state: 'reference', label: '策略参考 · 等待人工确认' };
});
function referencePoints(leader: Leader) {
  const plan = leader.plan; const prices = plan?.reference_prices; const trial = plan?.trial; const confirm = plan?.confirm;
  const near = (actual: number, expected: number) => Math.abs(actual - expected) <= Math.max(1, Math.abs(expected)) * 1e-8;
  if (plan?.status !== 'unvalidated_observation' || !prices || !trial || !confirm
    || day(prices.as_of) !== day(leader.as_of) || !day(prices.as_of)
    || ![prices.close, prices.ma20, prices.atr, prices.prior_high20, trial.buy_low, trial.buy_high, confirm.buy_low, confirm.buy_high].every(positive)) return null;
  const scale = prices.close / leader.close;
  if (!positive(scale) || !near(prices.ma20, leader.ma20 * scale) || !near(prices.atr, leader.atr * scale)
    || !near(trial.buy_low, prices.ma20 - 0.25 * prices.atr) || !near(trial.buy_high, prices.ma20 + 0.25 * prices.atr)
    || !near(confirm.buy_low, prices.prior_high20) || !near(confirm.buy_high, prices.prior_high20 + 0.5 * prices.atr)
    || !near(plan.weakness_trim?.trigger_price ?? NaN, prices.ma20)
    || plan.profit_trim?.atr_above_cost !== 2 || plan.exit?.atr_from_cost !== 3) return null;
  const input = (holdingCosts.value[leader.symbol] ?? '').trim();
  const costEntered = input !== '';
  const costInvalid = costEntered && (!/^(?:\d+(?:\.\d*)?|\.\d+)$/.test(input) || !positive(Number(input)));
  const cost = costInvalid ? null : costEntered ? Number(input) : prices.close;
  const profit = cost == null ? null : cost + 2 * prices.atr;
  const exit = cost == null ? null : cost - 3 * prices.atr;
  return { prices, trial, confirm, costEntered, costInvalid, cost,
    profit: positive(profit) ? profit : null, exit: positive(exit) ? exit : null };
}
const newsById = computed(() => new Map((data.value?.news ?? []).map(item => [item.id, item])));
const jointLeaders = computed(() => leaders.value.filter(leader => leader.model_research?.production_admission === false
  && leader.model_research.models.some(model => model.signal_status === 'positive_record'
    && recordAtDay(model,leader.as_of,leader.model_research) && positive(model.score) && model.observation?.signal_eligible === true)));
function recordAtDay(model: ResearchModel,asOf: string | number,context?: ModelResearch) {
  const scoreDay=model.score_as_of!=null?day(model.score_as_of):context?.date_matches===true?day(context.frozen_as_of):'';
  return scoreDay.length===8&&scoreDay===day(asOf);
}
function researchStatus(model: ResearchModel,asOf: string | number,context?: ModelResearch) {
  if (model.signal_status === 'date_mismatch' || (['positive_record','nonpositive_record'].includes(model.signal_status) && !recordAtDay(model,asOf,context))) return '冻结日期不同，当前评分未知';
  if (model.signal_status === 'positive_record') return model.observation?.signal_eligible ? '同日正分记录 · 仅联合观察' : '同日正分，但原推断资格未通过';
  if (model.signal_status === 'nonpositive_record') return '同日非正分记录 · 不作为正分观察';
  if (model.signal_status === 'not_in_scored_export') return '不在全分导出，输入资格/评分未知';
  return '未在正分表中提供，不能猜作负分';
}
function modelScore(model: ResearchModel,asOf: string | number,context?: ModelResearch) { return ['positive_record','nonpositive_record'].includes(model.signal_status) && recordAtDay(model,asOf,context) ? number(typeof model.score === 'number' ? model.score * 100 : null, '%') : '--'; }
function newsTime(value: SourceNews['received_at']) { if (value == null) return '未记录'; const parsed = new Date(value); return Number.isNaN(parsed.getTime()) ? '未记录' : parsed.toLocaleString(); }
function precision(value: SourceNews['publication_precision']) { return ({second:'到秒',millisecond:'到毫秒',date:'仅日期，日内时点未知',unknown:'发布时间未知'} as Record<string,string>)[value ?? 'unknown'] ?? '发布时间精度未核实'; }
const groups = computed(() => summary.value ? [
  { name: '支持因素', rows: summary.value.positives }, { name: '反证与风险', rows: summary.value.negatives },
  { name: '待核实', rows: summary.value.uncertainties.map(text => ({ text, source_ids: [], evidence: undefined })) },
] : []);
const metricLabels: Record<string, string> = { r5: '近5日', r20: '近20日', r60: '近60日', rs20_vs_hs300: '相对沪深300 · 20日', rs60_vs_hs300: '相对沪深300 · 60日', persistence: '强势持续度' };
const breadthText = computed(() => typeof data.value?.trend_breadth === 'number' && Number.isFinite(data.value.trend_breadth)
  && data.value.trend_breadth >= 0 && data.value.trend_breadth <= 1 ? number(data.value.trend_breadth * 100, '%') : '--');
function safeUrl(value: string | null | undefined) { try { const parsed = new URL(value ?? ''); return ['http:', 'https:'].includes(parsed.protocol) ? parsed.href : null; } catch { return null; } }
async function openSource(item: SourceNews) {
  const url = safeUrl(item.url ?? item.source_url); if (!url) return;
  try { await openUrl(url); } catch (cause) { error.value = `来源网页打开失败：${cause}`; }
}
async function refresh(reset = false) {
  const token = ++epoch; loading.value = true; error.value = '';
  referenceClock.value = Date.now();
  if (reset) { data.value = null; summary.value = null; watched.value = false; holdingCosts.value = {}; }
  analysisError.value = ''; watchError.value = ''; analyzing.value = false; savingWatch.value = false;
  const args = { kind: props.kind, sectorCode: props.sectorCode, sectorName: props.sectorName, fingerprint: props.snapshotFingerprint ?? null };
  const results = await Promise.allSettled([
    invoke<MainlineResult>('get_sector_mainline', args), invoke<WatchItem[]>('get_mainline_watchlist'),
  ]);
  if (token !== epoch) return;
  const [research, watchlist] = results;
  if (research.status === 'fulfilled') {
    if (research.value.sector_code !== args.sectorCode) error.value = '返回板块与请求不一致，请刷新重试。';
    else {
      if (data.value?.fingerprint !== research.value.fingerprint) summary.value = null;
      data.value = research.value;
    }
  } else error.value = `主线研究读取失败：${research.reason}`;
  if (watchlist.status === 'fulfilled') watched.value = watchlist.value.some(item => item.kind === args.kind && item.code === args.sectorCode);
  else watchError.value = `关注列表读取失败：${watchlist.reason}`;
  if (error.value && props.snapshotFingerprint) { data.value = null; summary.value = null; watched.value = false; }
  loading.value = false;
}
async function toggleWatch() {
  if (savingWatch.value || loading.value || error.value || watchError.value || !data.value) return;
  const token = epoch; savingWatch.value = true; watchError.value = '';
  try {
    await invoke('set_mainline_watch', { kind: props.kind, sectorCode: props.sectorCode, sectorName: props.sectorName, enabled: !watched.value });
    if (token === epoch) watched.value = !watched.value;
  } catch (cause) { if (token === epoch) watchError.value = `关注保存失败：${cause}`; }
  finally { if (token === epoch) savingWatch.value = false; }
}
async function analyze() {
  if (!data.value?.fingerprint || analyzing.value || loading.value || error.value) return;
  const token = epoch; const fingerprint = data.value.fingerprint; analyzing.value = true; analysisError.value = '';
  try {
    const result = await invoke<Summary>('analyze_mainline', { fingerprint });
    if (token === epoch && data.value?.fingerprint === fingerprint) summary.value = result;
  } catch (cause) { if (token === epoch) analysisError.value = `Claude Code 汇总失败：${cause}`; }
  finally { if (token === epoch) analyzing.value = false; }
}
watch(() => [props.kind, props.sectorCode, props.sectorName, props.snapshotFingerprint], () => { void refresh(true); }, { immediate: true });
onBeforeUnmount(() => { epoch++; clearInterval(referenceClockTimer); });
</script>

<template>
  <section class="mainline-research">
    <header class="mainline-toolbar">
      <div><b>{{ sectorName }} · 主线详情</b> <NTag size="small" type="info">手动观察 · 仅提醒</NTag></div>
      <div class="mainline-actions"><NButton size="small" :disabled="loading || !!error || !!watchError || !data" :loading="savingWatch" @click="toggleWatch">{{ watched ? '暂停主线提醒' : '开启主线提醒' }}</NButton><NButton class="mainline-refresh-control" size="small" :loading="loading" @click="refresh()">{{ snapshotFingerprint ? '重读通知快照' : '刷新研究' }}</NButton></div>
    </header>
    <p class="mainline-meta">近期主线趋势直接获取在线行情；多年走势、历史买卖价位研究需另行安装并启用StockDB。</p>
    <p class="mainline-observation">开启主线提醒后，应用运行时会在盘后检查该板块与领涨候选；覆盖、日期和主线条件全部通过后才发提醒。无需账户，只发提醒；不会自动买卖，也不会使用自动模型账户。</p>
    <p v-if="error" class="mainline-error" role="alert">{{ error }}<span v-if="data">；保留上次结果供查看，本次未更新。</span></p>
    <p v-if="watchError" class="mainline-error" role="alert">{{ watchError }}；刷新研究可重试，读取成功前不能更改提醒。</p>
    <p v-if="snapshotFingerprint" class="mainline-meta mainline-archived">正在查看通知留存的那一版证据，行情日期见下方；这是历史快照，不会用当前扫描替换，也不是今日买入提示。开启提醒只针对之后的新交易日。</p>
    <NSpin :show="loading">
      <template v-if="data">
        <div class="mainline-status"><NTag :type="strong ? 'success' : 'warning'" size="small">{{ strong ? '强主线观察' : complete ? '条件未通过' : '覆盖未通过' }}</NTag><span>{{ data.status }}</span><span>行情 {{ date(data.as_of) }} · 成分 {{ date(data.member_as_of) }}</span></div>
        <p class="mainline-cycle">观察周期：20/60日趋势确认，近5日表现作为强弱变化参考。合格成分覆盖 {{ data.covered_members }} / {{ eligibleCount }}，缺失 {{ data.missing_members.length }}。</p>
        <p v-if="!complete" class="mainline-error" role="status">成分覆盖或行情日期未通过核验，暂不能确认强主线；缺行情、请求失败不等于板块走弱。</p>
        <p class="mainline-meta">当前成分名单用于本次观察，不能还原任意历史阶段的“最强主线”，也不代表多年主线策略已经验证。</p>
        <div class="mainline-metrics"><div v-for="(label, key) in metricLabels" :key="key"><span>{{ label }}</span><b>{{ number(data.metrics[key], key.startsWith('rs') ? '个百分点' : key.startsWith('r') ? '%' : key === 'persistence' ? ' / 5日' : '') }}</b></div><div><span>内部趋势广度</span><b>{{ breadthText }}</b></div></div>
        <p v-if="positive(data.metrics.removed_non_trading_rows)" class="mainline-calendar mainline-meta">行情已按真实交易日对齐。</p>
        <p class="mainline-breadth mainline-meta">趋势与相对强势成分 {{ typeof data.trending_members === 'number' && Number.isFinite(data.trending_members) ? data.trending_members : '--' }} / 已覆盖 {{ data.covered_members }}；基准真实沪深300（sh000300），{{ data.benchmark?.source || '来源未返回' }} · {{ date(data.benchmark?.as_of) }}。</p>
        <details v-if="data.missing_members.length" class="mainline-help mainline-missing"><summary>查看 {{ data.missing_members.length }} 只缺失或无法研究的成分及原因</summary><ul><li v-for="item in data.missing_members" :key="item.symbol">{{ item.symbol }}：{{ item.reason }}</li></ul></details>
        <div class="mainline-leaders-toolbar">
          <h4 class="mainline-leaders-heading">{{ strong ? '领涨观察股' : complete ? '成分观察候选' : '部分成分观察候选' }} <small>显示 {{ visibleLeaders.length }} / {{ leaders.length }} 只</small></h4>
          <label class="mainline-leader-count">展示数量
            <select v-model.number="leaderDisplayCount" aria-label="个股展示数量">
              <option v-for="count in leaderDisplayOptions" :key="count" :value="count">{{ count }}只</option>
            </select>
          </label>
        </div>
        <p class="mainline-meta">沿用原有形态分排序，每只须通过同日行情、20/60日相对强势、趋势和MA20/ATR条件。覆盖不全时只展示已核验成分的合格个股，不代表板块已确认强主线；这里是观察候选，不是自动模型下单名单。</p>
        <p v-if="leaders.length" class="mainline-meta mainline-reference-hint">点个股下方“参考买卖策略”查看点位与模型记录；仅供手动参考。</p>
        <NEmpty v-if="!leaders.length" :description="leaderEmptyDescription" size="small" class="mainline-empty" />
        <article v-for="(leader, index) in visibleLeaders" :key="leader.symbol" class="mainline-leader">
          <header><b><span class="mainline-rank">#{{ index + 1 }}</span> {{ leader.name }} · {{ leader.symbol }}</b><small>{{ date(leader.as_of) }} · 形态分 {{ number(leader.score) }}（非概率）</small></header>
          <p>相对沪深300：20日 {{ number(leader.rs20_vs_hs300, '个百分点') }} · 60日 {{ number(leader.rs60_vs_hs300, '个百分点') }}。</p>
          <p class="mainline-meta">前复权指标收盘 {{ number(leader.close, '元') }} · 20日 {{ number(leader.r20, '%') }} · 60日 {{ number(leader.r60, '%') }}</p>
          <details class="mainline-reference-toggle" :aria-label="leader.name + '参考策略详情'">
            <summary>参考买卖策略 <span>{{ leader.reference ? '点位与模型核对' : '暂无有效点位' }}</span></summary>
          <section v-if="leader.reference" class="mainline-reference" :data-reference-state="referenceContext.state" :aria-label="leader.name + '参考买卖策略'">
            <div class="mainline-reference-heading"><span class="mainline-reference-state">{{ referenceContext.label }}</span></div>
            <p class="mainline-price-basis">截止 {{ date(leader.reference.prices.as_of) }} · 下方均为未复权人民币参考价。收盘 {{ number(leader.reference.prices.close, '元') }}，MA20 {{ number(leader.reference.prices.ma20, '元') }}，ATR {{ number(leader.reference.prices.atr, '元') }}。</p>
            <div class="mainline-cost-row">
              <label :for="'mainline-cost-' + leader.symbol">我的持仓成本（元，可选）</label>
              <input :id="'mainline-cost-' + leader.symbol" v-model.trim="holdingCosts[leader.symbol]" class="mainline-cost-input" type="text" inputmode="decimal" autocomplete="off" placeholder="未持仓可留空" :aria-invalid="leader.reference.costInvalid" :aria-describedby="'mainline-cost-note-' + leader.symbol" />
              <span :id="'mainline-cost-note-' + leader.symbol" class="mainline-cost-basis" :class="{ 'mainline-cost-error': leader.reference.costInvalid }" role="status">{{ leader.reference.costInvalid ? '请输入大于0的有效成本；退出和保护价暂不计算。' : leader.reference.costEntered ? '按你填写的成本 ' + number(leader.reference.cost, '元') + ' 计算；仅本页参考。' : '未持仓演示：暂用该日收盘 ' + number(leader.reference.prices.close, '元') + ' 计算，未读取任何账户成本。' }}</span>
            </div>
            <div class="mainline-reference-grid">
              <div class="mainline-reference-item"><span>回踩入场参考区</span><b data-reference="pullback">{{ number(leader.reference.trial.buy_low) }}–{{ number(leader.reference.trial.buy_high, '元') }}</b><p>{{ leader.reference.trial.condition }}。</p></div>
              <div class="mainline-reference-item"><span>突破确认 / 加仓参考区</span><b data-reference="breakout">{{ number(leader.reference.confirm.buy_low) }}–{{ number(leader.reference.confirm.buy_high, '元') }}</b><p>{{ leader.reference.confirm.condition }}。突破后回踩不破、重新走强再复核后续加仓；破位取消。</p></div>
              <div class="mainline-reference-item"><span>MA20弱化参考线</span><b data-reference="weakness">{{ number(leader.reference.prices.ma20, '元') }}</b><p>{{ leader.plan.weakness_trim?.condition }}；原计划参考减仓 {{ number(leader.plan.weakness_trim?.sell_position_pct, '%') }}，不是触线立刻卖出。</p></div>
              <div class="mainline-reference-item"><span>盈利保护启动参考线</span><b data-reference="profit">{{ number(leader.reference.profit, '元') }}</b><p>成本 + 2 × ATR。达到后，若完成分钟跌破 VWAP，原计划参考减仓 {{ number(leader.plan.profit_trim?.sell_position_pct, '%') }}；这是保护启动线，不是固定止盈价。</p></div>
              <div class="mainline-reference-item"><span>风险退出参考线</span><b data-reference="exit">{{ number(leader.reference.exit, '元') }}</b><p>{{ leader.reference.costInvalid ? '成本无效，暂不计算。' : leader.reference.exit == null ? '成本 − 3 × ATR 不大于0，无法给出有效退出价，请另行复核风险。' : '成本 − 3 × ATR；触及后复核退出，仍须可卖出、可成交。' }}</p></div>
              <div class="mainline-reference-item"><span>持有期与提前退出</span><b>{{ leader.plan.exit?.review_range_days?.join('–') || '--' }}交易日内复核</b><p>沿用原计划偏好 {{ leader.plan.exit?.preferred_days ?? '--' }} 日、上限 {{ leader.plan.exit?.max_holding_days ?? '--' }} 交易日；趋势失效可更早退出，不必固定等满。期限仅为未验证观察计划。</p></div>
            </div>
          </section>
          <p v-else class="mainline-reference-unavailable" role="status">未复权报价或参考指标缺失、无效或日期不一致，暂无价位参考。请刷新研究；不会用前复权指标直接替代买卖价位。</p>
          <div v-if="leader.model_research?.models.length" class="mainline-stock-models">
            <b>同日模型核对</b><p class="mainline-meta">核对该股是否命中已有模型；不同日期的分数不作为本次信号。</p>
            <p v-for="model in leader.model_research.models" :key="model.id">{{ model.name }}：{{ researchStatus(model,leader.as_of,leader.model_research) }} · {{ modelScore(model,leader.as_of,leader.model_research) }} <small>评分截止 {{ date(model.score_as_of ?? (leader.model_research.date_matches?leader.model_research.frozen_as_of:undefined)) }}</small></p>
          </div>
          </details>
        </article>

        <details class="mainline-help mainline-evidence"><summary>已研究模型证据与主线交集（可选）</summary>
        <section class="mainline-models">
          <h4>真实模型证据与主线交集</h4>
          <p class="mainline-meta">主线是辅助观察维度。把主线条件加到股票模型上尚未证明长期增益，不能当作获利增强或准入条件。当前通过形态条件且有同日、合格正分记录的交集 {{ jointLeaders.length }} 只；仍不自动买入。</p>
          <p class="mainline-meta">冻结推断截止 {{ date(data.model_research?.frozen_as_of) }}；多年证据截止 {{ date(data.model_research?.evidence_as_of) }}；本次行情截止 {{ date(data.as_of) }}。分值是20日收益标签代理，不是上涨概率，组合历史收益不能归给某一只股票。</p>
          <p v-if="!data.model_research?.models.length" class="mainline-error">本次没有通过版本校验的模型证据，评分未知。</p>
          <article v-for="model in data.model_research?.models ?? []" :key="model.id" class="mainline-model">
            <b>{{ model.name }} · {{ model.feature_count }}特征 / {{ model.holding_days }}交易日标签</b>
            <p>形成期净收益 {{ number(model.performance.train?.net_return_pct, '%') }} / 最大回撤 {{ number(model.performance.train?.max_drawdown_pct, '%') }}；选择期净收益 {{ number(model.performance.validation?.net_return_pct, '%') }} / 最大回撤 {{ number(model.performance.validation?.max_drawdown_pct, '%') }}。</p>
            <p>已见探索后段净收益 {{ number(model.performance.test?.net_return_pct, '%') }} / 最大回撤 {{ number(model.performance.test?.max_drawdown_pct, '%') }}；后段双成本 {{ number(model.performance.double_cost_return_pct, '%') }}。状态 {{ model.performance.status || '未准入研究' }}。</p>
          </article>
          <details class="mainline-help"><summary>? 模型版本与缺失分值</summary><p v-for="item in data.model_research?.limitations ?? []" :key="item">{{ item }}</p><code>运行 {{ data.model_research?.run_id || '--' }} · 行情 SHA {{ data.model_research?.snapshot_sha256 || '--' }}</code><code>推断正文 SHA {{ data.model_research?.screen_sha256 || '--' }} · 多年证据 SHA {{ data.model_research?.evidence_sha256 || '--' }}</code></details>
        </section>
        </details>
        <details class="mainline-help mainline-sources"><summary>原文证据与手动汇总（可选）</summary>
        <h4>近期原文与来源</h4>
        <p v-if="!data.news.length" class="mainline-meta">本次没有可核查的相关原文。</p>
        <article v-for="item in data.news" :id="`mainline-source-${item.id}`" :key="item.id" class="mainline-news"><b>{{ item.title }}</b><small>{{ item.source || '来源未标明' }} · 发布 {{ item.published_at || item.published_date || '未核实' }}（{{ precision(item.publication_precision) }}） · 本机接收 {{ newsTime(item.received_at) }} · 证据 {{ item.id }}</small><small v-if="item.published_after_market_asof === true">发布时间晚于行情截点，仅作为后续新信息，不能冒充预测前已知证据。</small><small v-else-if="item.published_after_market_asof == null">发布时间或精度未核实，不能证明行情截点前已可得。</small><small v-if="item.source_index_only">来源只提供公告索引或标题，本机没有完整正文。</small><small v-if="item.match_basis">关联方式 {{ item.match_basis }}；相关性不等于利好利空已验证。</small><p>{{ item.body || '本机未保存完整正文，请打开来源核实。' }}</p><details v-if="item.coverage_note"><summary>资讯覆盖口径</summary><small>{{ item.coverage_note }}</small></details><button v-if="safeUrl(item.url ?? item.source_url)" class="mainline-link" @click="openSource(item)">打开来源原文</button></article>
        <div class="mainline-claude"><NButton size="small" :disabled="!data.fingerprint || loading || !!error" :loading="analyzing" @click="analyze">{{ analyzing ? 'Claude Code 汇总中…' : '手动 Claude Code 汇总' }}</NButton><small>本次调用可能产生模型费用；程序条件与仓位数值不由模型改写。</small></div>
        <p v-if="analysisError" class="mainline-error" role="alert">{{ analysisError }}</p>
        <div v-if="summary" class="mainline-summary"><p>{{ summary.overview }}</p><section v-for="group in groups" :key="group.name"><h4>{{ group.name }}</h4><ul><li v-for="(claim, index) in group.rows" :key="index">{{ claim.text }} <a v-for="id in claim.source_ids.filter(source => newsById.has(source))" :key="id" :href="`#mainline-source-${id}`">[{{ id }}]</a><blockquote v-if="claim.evidence && claim.source_ids.some(id => newsById.has(id))">原文摘录：{{ claim.evidence }}</blockquote><small v-if="!claim.source_ids.some(id => newsById.has(id))">（未引用本次原文，待核实）</small></li></ul></section></div>
        </details>
        <details class="mainline-help mainline-method"><summary>研究机制、数据来源与局限（可选）</summary>
          <p class="mainline-reference-note">参考价位沿用原有 MA20 / ATR / 前高观察计划，尚未通过多年主线策略验证。触价不等于买卖信号；实时行情和完成分钟量价需自行核对。本页不监测参考点位，也不发送买卖点告警。</p>
          <p v-if="leaders.length" class="mainline-reference-execution">普通A股买入批次按 T+1 执行，当日新买入部分不能当天卖出。停牌、封涨跌停或流动性不足可能导致无法成交；参考触发价不保证成交，跳空可能越过退出线。除权除息后请刷新并核对自己的成本。</p>
          <p>板块与个股的20日、60日涨幅都须超过真实沪深300指数；领涨观察距MA20不得超过2倍ATR；满足趋势与相对强势的内部广度至少50%。只有全部合格成分行情覆盖、日期一致并满足机制条件，才能确认强主线并触发提醒。覆盖不足时仍展示已通过个股条件的部分成分观察候选；没有合格个股时保留空名单。研究分数不是上涨概率或胜率。</p>
          <p>ST、北交所及沪深股通、昨日涨停等动态资格集合排除。当前只比较5/20/60日行情，不支持任意历史区间排行、连续历史排名变化或牛熊阶段自动识别。成分使用当前完整名单，不能回填成历史时点股票池。</p>
          <p class="mainline-meta">成分 {{ data.total_members }}，排除 {{ data.excluded_members }}，合格 {{ eligibleCount }}，已覆盖 {{ data.covered_members }}，缺失 {{ data.missing_members.length }}。板块来源 {{ data.sector_source || '--' }}；成分来源 {{ data.member_source || '--' }}。</p>
          <p v-if="data.membership_source_is_current_snapshot" class="mainline-membership mainline-meta">当前发布者名单观察于 {{ data.membership_observed_at || '--' }}；这是本次抓取观察时间，不等于历史名单生效日期或成分股报价时间。上方成分行情日期由截止交易日的实际行情核验。</p>
          <ul v-if="data.limitations.length" class="mainline-limitations"><li v-for="item in data.limitations" :key="item">{{ item }}</li></ul>
          <p>扫描和提醒不会自动调用Claude Code。手动汇总只解释本次证据，不能改变程序生成的数值计划，也不会创建或使用交易账户。</p>
        <details class="mainline-fingerprint"><summary>查看本次证据标识</summary><code>稳定证据 {{ data.fingerprint }}</code><code>完整正文 {{ data.content_sha256 || '旧版完整内容指纹' }}</code><p>生成于 {{ data.generated_at }}；刷新时间不改变相同证据的稳定标识。</p></details>
        </details>
      </template>
      <NEmpty v-else-if="!loading && !error" description="暂无主线研究" />
    </NSpin>
  </section>
</template>

<style scoped>
.mainline-reference-note,.mainline-reference-execution{padding:10px 12px;border:1px solid var(--color-border-0);border-radius:8px;color:var(--color-text-secondary);font-size:13px;line-height:1.7}
.mainline-reference{margin-top:12px;padding-top:12px;border-top:1px solid var(--color-border-0)}.mainline-reference-heading,.mainline-cost-row{display:flex;align-items:center;flex-wrap:wrap;gap:8px}.mainline-reference-heading{justify-content:space-between}.mainline-reference-state{color:var(--color-text-secondary);font-size:13px}.mainline-reference[data-reference-state="weak"] .mainline-reference-state,.mainline-reference[data-reference-state="stale"] .mainline-reference-state,.mainline-reference[data-reference-state="partial"] .mainline-reference-state{color:var(--color-warning)}.mainline-price-basis,.mainline-cost-basis{font-size:13px;color:var(--color-text-secondary);line-height:1.65}.mainline-cost-row{margin:10px 0 12px}.mainline-cost-row label{font-size:13px}.mainline-cost-input{box-sizing:border-box;width:150px;max-width:100%;padding:7px 10px;border:1px solid var(--color-border-0);border-radius:6px;font:inherit;color:var(--color-text-primary);background:var(--color-surface-0)}.mainline-cost-input:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}.mainline-cost-input[aria-invalid="true"]{border-color:var(--color-warning)}.mainline-cost-error,.mainline-reference-unavailable{color:var(--color-warning)}.mainline-reference-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px}.mainline-reference-item{min-width:0;padding:10px 12px;border-radius:7px;background:var(--color-surface-2);line-height:1.6}.mainline-reference-item>span{font-size:13px;color:var(--color-text-secondary)}.mainline-reference-item>b{display:block;margin:4px 0;font-size:17px;font-variant-numeric:tabular-nums}.mainline-reference-item>p{margin:4px 0 0;font-size:13px;color:var(--color-text-secondary);line-height:1.65}@media(max-width:750px){.mainline-reference-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:500px){.mainline-reference-grid{grid-template-columns:1fr}.mainline-cost-basis{flex-basis:100%}}
.mainline-leaders-toolbar{display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:10px}.mainline-leader-count{display:flex;align-items:center;gap:8px;color:var(--color-text-secondary);white-space:nowrap}.mainline-leader-count select{min-width:76px;padding:5px 8px;border:1px solid var(--color-border-0);border-radius:6px;background:var(--color-surface-0);color:var(--color-text-primary);font:inherit;cursor:pointer}.mainline-leader-count select:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
.mainline-refresh-control{min-width:124px}.mainline-models{display:flex;flex-direction:column;gap:8px}.mainline-model,.mainline-stock-models{padding:10px;background:var(--color-surface-2);border-radius:6px;line-height:1.65}.mainline-model p{margin-top:6px}.mainline-research{min-width:0;overflow-wrap:anywhere}.mainline-metrics>div{min-width:0}.mainline-models code{display:block;overflow-wrap:anywhere;font-size:12px}.mainline-summary blockquote{margin:6px 0;padding:8px 12px;border-left:3px solid var(--color-accent);background:var(--color-surface-2);white-space:pre-wrap}.mainline-archived{border-left:3px solid var(--color-warning);padding-left:10px}
.mainline-observation{margin:0;padding:10px 12px;border-left:3px solid var(--color-accent);background:var(--color-surface-2);border-radius:6px;color:var(--color-text-secondary);line-height:1.7}.mainline-cycle{margin:10px 0;color:var(--color-text-secondary);line-height:1.7}.mainline-rank{color:var(--color-accent)}.mainline-research h4 small{color:var(--color-text-secondary);font-weight:400;font-size:12px}.mainline-reference-toggle{margin-top:10px}.mainline-reference-toggle>summary{cursor:pointer;color:var(--color-accent);padding:8px 0;font-weight:600;line-height:1.6}.mainline-reference-toggle>summary span{margin-left:8px;font-size:12px;font-weight:400;color:var(--color-text-secondary)}.mainline-reference-toggle>summary:focus-visible{outline:2px solid var(--color-accent);outline-offset:3px;border-radius:4px}.mainline-sources,.mainline-evidence,.mainline-method{margin-top:12px}.mainline-help>section,.mainline-sources>h4{margin-top:12px}.mainline-stock-models{margin-top:12px}.mainline-leader{margin-top:8px}.mainline-research{display:flex;flex-direction:column;gap:12px;font-size:13px}.mainline-toolbar,.mainline-status,.mainline-actions{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.mainline-toolbar{justify-content:space-between}.mainline-help,.mainline-fingerprint{padding:10px 12px;border:1px solid var(--color-border-0);border-radius:8px}.mainline-help summary,.mainline-fingerprint summary{cursor:pointer}.mainline-help p,.mainline-meta,.mainline-limitations,.mainline-leader>small,.mainline-claude small{color:var(--color-text-secondary);line-height:1.65}.mainline-status{font-size:12px}.mainline-error{color:var(--color-warning);white-space:pre-wrap}.mainline-metrics{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px}.mainline-metrics>div{padding:10px;background:var(--color-surface-2);border-radius:6px}.mainline-metrics span,.mainline-news small,.mainline-leader small{display:block;color:var(--color-text-tertiary);font-size:12px}.mainline-metrics b{display:block;font-variant-numeric:tabular-nums;margin-top:4px}h4,p{margin:0}.mainline-leader,.mainline-news,.mainline-summary{border:1px solid var(--color-border-0);padding:12px;border-radius:8px}.mainline-leader header{display:flex;justify-content:space-between;flex-wrap:wrap;gap:8px}.mainline-leader p,.mainline-news p{margin:8px 0;line-height:1.6;white-space:pre-wrap}.mainline-link{border:0;background:transparent;color:var(--color-accent);cursor:pointer;padding:0}.mainline-claude{display:flex;gap:10px;flex-wrap:wrap;align-items:center}.mainline-summary{line-height:1.75}.mainline-summary li{white-space:pre-wrap}.mainline-summary a{color:var(--color-accent)}.mainline-fingerprint code{display:block;overflow-wrap:anywhere;margin-top:10px}.mainline-empty{padding:15px 0}@media(max-width:650px){.mainline-metrics{grid-template-columns:repeat(3,minmax(0,1fr))}}
</style>
