<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NButton, NEmpty, NTag } from 'naive-ui';
import type { MainlineNavigationTarget } from '@/types/navigation';

interface Candidate extends MainlineNavigationTarget {
  as_of: string; source?: string; metrics: Record<string, unknown>;
  entry_ready?: boolean; observation_state?: 'ready' | 'waiting_pullback'; extension_atr?: number;
}
interface Failure { code: string; name?: string; reason: string }
interface DiscoveryStatus {
  enabled: boolean; busy: boolean; as_of?: string; total?: number; processed?: number;
  failed?: Failure[]; candidates?: Candidate[]; finished?: boolean; last_error?: string | null;
  catalog_total?: number; excluded_sectors?: { code: string; name: string }[];
  catalog_errors?: string[]; retry_processed?: number; retry_total?: number;
}
const props = withDefaults(defineProps<{ active?: boolean }>(), { active: true });
const emit = defineEmits<{ open: [candidate: MainlineNavigationTarget] }>();
const state = ref<DiscoveryStatus | null>(null);
const reading = ref(false); const manualReading = ref(false); const starting = ref(false); const pausing = ref(false); const error = ref('');
const intent = ref<boolean | null>(null);
const hasStarted = ref(false);
const rankingPeriod = ref<20 | 60>(20);
let disposed = false; let request = 0; let controlRequest = 0; let timer: ReturnType<typeof setInterval> | undefined;
const failures = computed(() => state.value?.failed ?? []);
const total = computed(() => Math.max(0, Math.floor(state.value?.total ?? 0)));
const processed = computed(() => Math.max(0, Math.min(total.value, Math.floor(state.value?.processed ?? 0))));
const retrying = computed(() => !state.value?.finished && (state.value?.retry_processed ?? 0) < (state.value?.retry_total ?? 0));
const completed = computed(() => !!state.value?.finished && total.value > 0 && processed.value === total.value);
const scopeComplete = computed(() => completed.value && !failures.value.length && !state.value?.catalog_errors?.length && !state.value?.last_error);
const enabled = computed(() => intent.value ?? (state.value?.enabled === true));
const working = computed(() => enabled.value && (starting.value || state.value?.busy === true));
function metric(candidate: Candidate, period: 20 | 60) { const value = candidate.metrics['rs' + period + '_vs_hs300']; return typeof value === 'number' && Number.isFinite(value) ? value : null; }
const candidates = computed(() => [...(state.value?.candidates ?? [])].sort((a, b) => {
  const aScore = current(a) ? metric(a, rankingPeriod.value) : null;
  const bScore = current(b) ? metric(b, rankingPeriod.value) : null;
  if (aScore === null || bScore === null) return aScore === bScore ? a.code.localeCompare(b.code) : aScore === null ? 1 : -1;
  return bScore - aScore || a.code.localeCompare(b.code);
}));
function date(value?: string) { const day = (value ?? '').replace(/\D/g, '').slice(0, 8); return day.length === 8 ? day.slice(0, 4) + '-' + day.slice(4, 6) + '-' + day.slice(6) : '--'; }
function percent(value: unknown) { return typeof value === 'number' && Number.isFinite(value) ? (value >= 0 ? '+' : '') + value.toFixed(2) + '%' : '--'; }
function relative(value: unknown) { return typeof value === 'number' && Number.isFinite(value) ? value.toFixed(2) + '个百分点' : '--'; }
function current(candidate: Candidate) { return !!state.value?.as_of && candidate.as_of === state.value.as_of; }
function observationState(candidate: Candidate) {
  if (!current(candidate)) return 'unknown';
  const extension = candidate.extension_atr;
  if (candidate.observation_state === 'waiting_pullback' || (typeof extension === 'number' && Number.isFinite(extension) && extension > 2)) return 'waiting_pullback';
  return candidate.entry_ready !== false && typeof extension === 'number' && Number.isFinite(extension) && extension <= 2 ? 'ready' : 'unknown';
}
function openCandidate(candidate: Candidate) {
  if (!current(candidate)) return;
  emit('open', { kind: candidate.kind, code: candidate.code, name: candidate.name });
}
async function refresh(manual = true) {
  if (reading.value || disposed) return;
  const token = ++request; reading.value = true; manualReading.value = manual;
  if (manual) error.value = '';
  try {
    const result = await invoke<DiscoveryStatus>('get_mainline_discovery_status');
    if (!disposed && token === request) {
      state.value = result;
      if (result.enabled || result.as_of) hasStarted.value = true;
      if (error.value.startsWith('发现进度读取失败')) error.value = '';
    }
  } catch (cause) { if (!disposed && token === request) error.value = '发现进度读取失败：' + cause + '；以下保留上次读取状态。'; }
  finally { if (!disposed) { reading.value = false; manualReading.value = false; } }
}
async function control(nextEnabled: boolean) {
  if (pausing.value || disposed || (nextEnabled && enabled.value)) return;
  const token = ++controlRequest;
  if (nextEnabled) hasStarted.value = true;
  request++; intent.value = nextEnabled; starting.value = nextEnabled; pausing.value = !nextEnabled; error.value = '';
  try {
    const result = nextEnabled
      ? await invoke<DiscoveryStatus>('run_mainline_discovery')
      : await invoke<DiscoveryStatus>('set_mainline_discovery_enabled', { enabled: false });
    if (!disposed && token === controlRequest) { request++; state.value = result; intent.value = null; }
  } catch (cause) {
    if (!disposed && token === controlRequest) { intent.value = null; error.value = (nextEnabled ? '全市场扫描启动失败：' : '暂停扫描失败：') + cause; }
  } finally {
    if (!disposed && token === controlRequest) { starting.value = false; pausing.value = false; await refresh(false); }
  }
}
watch(() => props.active, active => { if (active) void refresh(false); });
onMounted(() => {
  if (props.active) void refresh();
  timer = setInterval(() => { if (props.active && (working.value || enabled.value || state.value?.busy)) void refresh(false); }, 10000);
});
onBeforeUnmount(() => { disposed = true; request++; controlRequest++; if (timer) clearInterval(timer); });
</script>

<template>
  <section class="mainline-discovery">
    <header class="discovery-toolbar">
      <div><b>寻找当前主线候选</b> <NTag size="small" type="info">手动观察 · 仅提醒</NTag></div>
      <div class="discovery-actions">
        <NButton class="discovery-scan-control" size="small" :type="enabled ? 'warning' : 'primary'" :disabled="pausing" :loading="pausing" :aria-pressed="enabled" @click="control(!enabled)">{{ pausing ? '正在暂停…' : enabled ? '暂停全市场扫描' : hasStarted ? '继续全市场扫描' : '开始全市场扫描' }}</NButton>
        <NButton class="discovery-refresh-control" size="small" :loading="manualReading" :disabled="reading" @click="refresh()">刷新进度</NButton>
      </div>
    </header>
    <p class="discovery-note">点击开始后，应用运行时会在后台持续扫描，关闭本窗口也会继续；暂停会停止在途扫描并保留进度。新交易日收盘后自动扫描更新；继续保留已完成结果，只补查失败或未处理的板块。</p>
    <p class="discovery-boundary">这里无需模拟账户，只提供主线观察与提醒；不会下单，不会使用自动模型账户，也不会改变模型的选股范围。</p>
    <div class="discovery-progress" role="status">
      <NTag size="small" :type="enabled ? 'info' : 'default'">{{ pausing ? '正在暂停' : starting ? '正在启动 · 可暂停' : !enabled ? hasStarted ? '已暂停 · 进度保留' : '尚未开始' : working ? '后台扫描中' : '持续扫描 · 等待最新收盘行情' }}</NTag>
      <span>行情日期 <b>{{ date(state?.as_of) }}</b></span>
      <span>已处理 {{ processed }} / {{ total }} 个板块</span>
      <span v-if="retrying">失败补查 {{ state?.retry_processed ?? 0 }} / {{ state?.retry_total ?? 0 }} · 成功结果保留</span>
      <span>返回有效日线 {{ Math.max(0, processed - failures.length) }} · 失败 {{ failures.length }} · 待处理 {{ Math.max(0, total - processed) }}</span>
      <NTag size="small" :type="scopeComplete ? 'success' : 'warning'">{{ scopeComplete ? '范围完整 · 候选待成分核验' : retrying ? '补查失败板块 · 保留排名' : completed ? '目录已处理 · 范围不完整' : '目录尚未完成' }}</NTag>
      <progress v-if="total" :max="total" :value="processed" :aria-label="'已处理 ' + processed + ' / ' + total + ' 个板块'" />
    </div>
    <p v-if="error" class="discovery-error" role="alert">{{ error }}</p>
    <p v-if="state?.last_error" class="discovery-error" role="alert">后台扫描错误：{{ state.last_error }}</p>
    <p v-for="(issue, index) in state?.catalog_errors ?? []" :key="index" class="discovery-error" role="alert">目录来源失败：{{ issue }}；当前扫描范围不完整。</p>
    <p v-if="failures.length" class="discovery-error" role="status">部分板块数据尚未通过核验；程序会尝试同板块的备用行情，继续时只补查失败部分，已有排名保留。范围未全时不确认全市场最强。</p>
    <details v-if="failures.length" class="discovery-failures"><summary>查看 {{ failures.length }} 个失败板块（不能视为弱势）</summary><ul><li v-for="(failure, index) in failures" :key="failure.code + '-' + index">{{ failure.name || failure.code }} · {{ failure.code }}：{{ failure.reason }}</li></ul></details>
    <div class="discovery-ranking">
      <h4>主线候选排名 <small>{{ candidates.length }} 个 · 尚需成分核验</small></h4>
      <div class="discovery-actions" role="group" aria-label="相对强度排名周期">
        <NButton size="tiny" :type="rankingPeriod === 20 ? 'primary' : 'default'" :aria-pressed="rankingPeriod === 20" @click="rankingPeriod = 20">近20日</NButton>
        <NButton size="tiny" :type="rankingPeriod === 60 ? 'primary' : 'default'" :aria-pressed="rankingPeriod === 60" @click="rankingPeriod = 60">近60日</NButton>
      </div>
    </div>
    <p class="discovery-note">按相对沪深300的{{ rankingPeriod }}日强度排序；仅切换已返回候选的展示顺序，原有20/60日确认条件保持不变。{{ scopeComplete ? '板块条件通过后，还需详情中的完整成分核验。' : '扫描范围未核验完整，排名仍可能变化。' }}</p>
    <p class="discovery-note discovery-entry-note">强度排名不等于可追涨：距MA20超过2倍ATR的行业仍在榜单，标记“趋势强 · 等待回落”，可点开研究，不能作为当前入场提示。距离门槛通过后，提醒仍须完整成分、日期和原有条件核验。</p>
    <div class="discovery-candidates">
      <button v-for="(candidate, index) in candidates" :key="candidate.kind + '-' + candidate.code" type="button" :disabled="!current(candidate)" @click="openCandidate(candidate)">
        <span class="candidate-heading"><b class="candidate-rank">{{ current(candidate) && metric(candidate, rankingPeriod) !== null ? '#' + (index + 1) : '--' }}</b><b>{{ candidate.name }}</b></span>
        <small>{{ candidate.code.startsWith('SW') ? '申万一级行业' : candidate.kind === 'industry' ? '东方财富行业' : '东方财富概念' }} · {{ candidate.code }}</small>
        <NTag class="candidate-observation" size="small" :type="observationState(candidate) === 'waiting_pullback' ? 'warning' : 'info'">{{ observationState(candidate) === 'waiting_pullback' ? '趋势强 · 等待回落' : observationState(candidate) === 'ready' ? '趋势强 · 距离门槛通过' : '入场距离待核验' }}</NTag>
        <span class="candidate-strength">相对沪深300 · {{ rankingPeriod }}日 {{ relative(candidate.metrics['rs' + rankingPeriod + '_vs_hs300']) }}</span>
        <span>强弱变化参考：5日 {{ percent(candidate.metrics.r5) }} · 20日 {{ percent(candidate.metrics.r20) }} · 60日 {{ percent(candidate.metrics.r60) }}</span>
        <small>{{ date(candidate.as_of) }} · {{ current(candidate) ? '查看成分与领涨观察 ›' : '日期与本次扫描不一致，暂不进入' }}</small>
      </button>
    </div>
    <NEmpty v-if="!candidates.length" size="small" :description="scopeComplete ? '本次目录未发现满足条件的板块趋势候选' : completed ? '已返回部分未发现候选；失败来源尚无结论' : '目录尚未完成，候选结果仍可能变化'" />
    <details class="discovery-help"><summary>持续扫描与研究口径（可选）</summary>
      <p>扫描申万发行方一级行业（SW）和东方财富行业、概念（BK）；两套名称和指数各自保留。排除ST、北交所及沪深股通、昨日涨停等动态资格集合；每批处理部分目录，应用运行时在盘后继续余下批次。详情须核查全部合格成分、日期与领涨观察，才能形成提醒。</p>
      <p>板块与个股20日、60日涨幅均须超过真实沪深300；领涨观察距MA20不超过2倍ATR，内部趋势与相对强势广度至少50%。主线是辅助观察维度，加入冻结股票模型尚未证明长期增益，不代表胜率或准入。</p>
      <p>当前只能比较已完成行情的5/20/60日表现，没有任意起止区间的历史主线排行、连续历史排名变化或牛熊阶段自动识别。成分使用当前名单，不能作为历史时点股票池，也不能冒充多年主线策略回测。扫描不会自动调用Claude Code，原文汇总需手动点击。</p>
      <details v-if="state?.excluded_sectors?.length" class="discovery-failures"><summary>目录 {{ state.catalog_total ?? '--' }} 个，排除 {{ state.excluded_sectors.length }} 个动态资格或不研究集合</summary><ul><li v-for="item in state.excluded_sectors" :key="item.code">{{ item.name }} · {{ item.code }}</li></ul></details>
    </details>
  </section>
</template>

<style scoped>
.mainline-discovery { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
.discovery-toolbar, .discovery-actions, .discovery-ranking { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.discovery-toolbar, .discovery-ranking { justify-content: space-between; }
.discovery-note, .discovery-help, .discovery-progress, .discovery-failures, .discovery-boundary { font-size: var(--text-xs); color: var(--color-text-secondary); }
.discovery-note, .discovery-boundary { margin: 0; line-height: 1.7; }
.discovery-boundary { padding: 10px 12px; border-left: 3px solid var(--color-accent); background: var(--color-surface-2); border-radius: var(--radius-sm); }
.discovery-help { border: 1px solid var(--color-border-0); border-radius: var(--radius-md); padding: 12px; }
.discovery-help p { line-height: 1.7; }
.discovery-help summary, .discovery-failures summary { cursor: pointer; }
.discovery-refresh-control { min-width: 108px; }
.discovery-error { margin: 0; color: var(--color-warning); overflow-wrap: anywhere; }
.discovery-progress { display: flex; flex-wrap: wrap; gap: 8px 16px; }
.discovery-progress progress { flex-basis: 100%; width: 100%; height: 8px; accent-color: var(--color-accent); }
.discovery-failures { max-height: 240px; overflow: auto; }
.discovery-failures li { overflow-wrap: anywhere; }
.mainline-discovery h4 { margin: 0; font-size: var(--text-sm); }
.mainline-discovery h4 small { font-weight: 400; color: var(--color-text-tertiary); }
.discovery-candidates { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
.discovery-candidates button { display: flex; flex-direction: column; align-items: flex-start; min-width: 0; gap: 6px; padding: 14px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-1); color: var(--color-text-primary); text-align: left; overflow-wrap: anywhere; cursor: pointer; }
.discovery-candidates button:hover:not(:disabled) { border-color: var(--color-accent); }
.discovery-candidates button:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
.discovery-candidates button:disabled { opacity: .6; cursor: default; }
.discovery-candidates button small { color: var(--color-text-secondary); }
.candidate-heading { display: flex; align-items: baseline; gap: 10px; }
.candidate-rank, .candidate-strength { color: var(--color-accent); font-variant-numeric: tabular-nums; }
@media (max-width: 620px) { .discovery-candidates { grid-template-columns: 1fr; } }
</style>
