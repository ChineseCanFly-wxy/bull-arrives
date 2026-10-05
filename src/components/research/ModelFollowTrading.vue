<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { MODEL_CATALOG, modelScore } from '@/types/research';
import type { FollowAutomaticModel, FollowAutomaticStatus, FollowOrder, FollowSourceRun, FollowView } from '@/types/modelFollow';

const SimulationDialog = defineAsyncComponent(() => import('../simulation/SimulationDialog.vue'));
const showSimulation = ref(false);
const simulationAccountId = ref<number>();
const emit = defineEmits<{ prepareSource: [string]; refreshSources: [] }>();
const props = withDefaults(defineProps<{ sourceRuns: FollowSourceRun[]; active?: boolean; sourceBusy?: boolean; preferredAccountId?: number }>(), { active: true, sourceBusy: false });
const accounts = ref<FollowView[]>([]);
const selectedId = ref<number | ''>('');
const removalId = ref<number | null>(null);
const removalPanel = ref<HTMLElement | null>(null);
const account = computed(() => accounts.value.find(view => view.account_id === selectedId.value) ?? null);

const signalSchedule = computed(() => {
  const view = account.value;
  if (!view) return null;
  const execution = view.execution;
  const daily = ['breadth22_h20', 'index26_h20', 'breadth22_excess_csi20', 'breadth22_rank20', 'breadth22_open_downside20'].includes(view.model_id)
    && (execution.signal_basis ?? 'completed_daily') === 'completed_daily'
    && (execution.entry_schedule ?? 'next_session_open') === 'next_session_open'
    && (execution.execution_basis ?? 'live_depth') === 'live_depth';
  return {
    label: daily ? (execution.signal_label || '完成日日线选股') + ' · 次日开盘实时盘口买入' : '信号或执行模式未登记',
    note: daily ? execution.schedule_note || execution.intraday_note || '前一交易日完整日线确定候选，下一交易日 ' + execution.buy_window + ' 用实时盘口买入；盘中实时检查委托与卖出。' : '当前模式不能套用已登记的日线自动买入入口，请核对账户执行配置。',
  };
});
const timingResearch = computed(() => account.value?.timing_research ?? null);
const timingPolicyName = (id: string) => timingResearch.value?.policies.find(policy => policy.id === id)?.name || id || '--';
const reading = ref(false);
const action = ref('');
const busy = computed(() => reading.value || action.value !== '');
const loaded = ref(false);
const readError = ref('');
const automatic = ref<FollowAutomaticStatus | null>(null);
const automaticError = ref('');
const autoStateName = (state: string) => ({paused:'自动运行已暂停',closed:'休市 · 等待交易日',waiting_calendar:'等待交易日核验',waiting_time:'等待定时更新',waiting_update:'等待数据更新',waiting_data:'等待完整日线',waiting_delay:'更新成功 · 等待10分钟',due:'等待自动检查',running:'正在后台检查',complete:'今日已处理 · 等待新交易日',failed:'本日检查失败',retry_wait:'等待后台重试'}[state] ?? state);
const outcomeName = (row: FollowAutomaticModel) => {
  const exists = accounts.value.some(view => view.model_id === row.model_id);
  if (row.state === 'excluded' || (loaded.value && !readError.value && row.account_id && !exists && ['created','following'].includes(row.state))) return exists ? '已停用 · 不自动恢复' : '已删除 · 不再自动建';
  return ({created:'已建账',following:'已复用',no_candidates:'暂无合格候选',paused:'用户已暂停',failed:'等待重试',error:'检查失败',busy:'等待其他任务完成',waiting_research:'等待其他研究完成',waiting_source:'等待模型来源'}[row.state] ?? row.state);
};
const modelResult = (id: string) => automatic.value?.models.find(row => row.model_id === id);
const modelName = (id: string) => MODEL_CATALOG.find(model => model.id === id)?.name || id;
const accountState = (view: FollowView) => !view.enabled ? '已暂停买卖' : (view.effective_enabled === false || automatic.value?.effective_enabled === false) ? '全局已暂停' : stateName(view.state);
const error = ref('');
const notice = ref('');
const newOrderIds = ref<Record<number, number[]>>({});
const knownOrders = new Map<number, Set<number>>();
let timer: ReturnType<typeof setInterval> | undefined;
let mounted = false;
let disposed = false;
let inFlight = false;

const candidates = computed(() => [...(account.value?.candidates ?? [])].sort((left, right) => left.rank - right.rank));
const currentNewOrderIds = computed(() => account.value ? newOrderIds.value[account.value.account_id] ?? [] : []);
const activeOrder = (order: FollowOrder) => order.status === 'awaiting_confirmation' || order.status === 'pending';
const latestFirst = (left: FollowOrder, right: FollowOrder) =>
  (right.filled_at || right.created_at).localeCompare(left.filled_at || left.created_at) || right.id - left.id;
const orderGroups = computed(() => {
  const orders = account.value?.orders ?? [];
  const filled = orders.filter(order => order.status === 'filled').sort(latestFirst);
  const closed = orders.filter(order => !activeOrder(order) && order.status !== 'filled').sort(latestFirst);
  return [
    { key: 'active', title: '自动委托与待成交记录', rows: orders.filter(activeOrder).sort(latestFirst), total: orders.filter(activeOrder).length },
    { key: 'filled', title: '最近成交', rows: filled.slice(0, 10), total: filled.length },
    ...(closed.length ? [{ key: 'closed', title: '最近撤销 / 到期 / 拒绝记录', rows: closed.slice(0, 10), total: closed.length }] : []),
  ];
});
const money = (value: number | null | undefined) => typeof value === 'number' && Number.isFinite(value)
  ? `¥${value.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}` : '--';
const percent = (value: number | null | undefined) => typeof value === 'number' && Number.isFinite(value) ? `${value.toFixed(2)}%` : '--';
const beijingTime = new Intl.DateTimeFormat('zh-CN', {
  timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit',
  hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23',
});
function timestamp(value: string | null | undefined): string {
  if (!value) return '--';
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return value;
  if (/^\d{8}$/.test(value)) return value.replace(/^(\d{4})(\d{2})(\d{2})$/, '$1-$2-$3');
  const instant = new Date(value);
  if (!Number.isFinite(instant.getTime())) return '--';
  const parts = beijingTime.formatToParts(instant);
  const part = (type: Intl.DateTimeFormatPartTypes) => parts.find(item => item.type === type)?.value ?? '--';
  return `${part('year')}-${part('month')}-${part('day')} ${part('hour')}:${part('minute')}:${part('second')}（北京时间）`;
}
const statusName = (value: string) => ({
  awaiting_confirmation: '旧人工计划 · 待升级撤销', pending: '程序已提交 · 待撮合', filled: '已成交', rejected: '已拒绝',
  cancelled: '已撤销', canceled: '已撤销', expired: '已到期',
}[value] ?? value);
const stateName = (value: string) => ({
  waiting_session: '等待交易时段', listening: '监听新盘口', waiting_model_data: '等待原模型数据',
  data_unavailable: '行情数据暂不可用', company_action_review: '等待公司行为核验（分红 / 送转等）',
  active: '跟随中', running: '跟随中', paused: '已暂停', automatic_paused: '全局已暂停', waiting_data: '等待数据',
  waiting_new_data: '等待新数据', waiting_market: '等待交易时段', awaiting_confirmation: '等待自动执行升级',
}[value] ?? value);

watch(selectedId, () => { removalId.value = null; });
watch(() => props.active, active => { if (!active) removalId.value = null; });
watch(() => props.preferredAccountId, id => { if (id && accounts.value.some(view => view.account_id === id)) selectedId.value = id; });
function rememberOrders(view: FollowView) {
  const previous = knownOrders.get(view.account_id);
  const present = new Set(view.orders.map(order => order.id));
  const arrived = view.orders.filter(order => previous ? !previous.has(order.id) : activeOrder(order)).map(order => order.id);
  knownOrders.set(view.account_id, new Set([...(previous ?? []), ...present]));
  newOrderIds.value = {
    ...newOrderIds.value,
    [view.account_id]: [...new Set([...(newOrderIds.value[view.account_id] ?? []).filter(id => present.has(id)), ...arrived])],
  };
}

function applyAccount(view: FollowView) {
  rememberOrders(view);
  loaded.value = true;
  const index = accounts.value.findIndex(item => item.account_id === view.account_id);
  if (index === -1) accounts.value = [...accounts.value, view];
  else accounts.value = accounts.value.map(item => item.account_id === view.account_id ? view : item);
}

async function readAccounts(quiet = false) {
  if (disposed || !mounted || !props.active || inFlight) return;
  inFlight = true;
  reading.value = !quiet || !loaded.value;
  try {
    const [views, status] = await Promise.allSettled([
      invoke<FollowView[]>('research_follow_accounts'),
      invoke<FollowAutomaticStatus>('research_auto_status', { enabled: null }),
    ]);
    if (disposed || !props.active) return;
    if (views.status === 'fulfilled') {
      views.value.forEach(rememberOrders); accounts.value = views.value;
      if (!loaded.value && props.preferredAccountId && views.value.some(view => view.account_id === props.preferredAccountId)) selectedId.value = props.preferredAccountId;
      if (!views.value.some(view => view.account_id === selectedId.value)) {
        selectedId.value = !loaded.value && props.preferredAccountId ? '' : views.value[0]?.account_id ?? '';
      }
      loaded.value = true; readError.value = '';
    } else readError.value = String(views.reason);
    if (status.status === 'fulfilled') { automatic.value = status.value; automaticError.value = ''; }
    else automaticError.value = '自动运行状态暂不可用：' + String(status.reason);
  } finally {
    inFlight = false;
    if (!disposed) reading.value = false;
  }
}

async function perform(label: string, request: () => Promise<FollowView>, success: (view: FollowView) => string) {
  if (disposed || !props.active || inFlight) return;
  inFlight = true;
  action.value = label;
  error.value = '';
  notice.value = '';
  try {
    const view = await request();
    if (disposed) return;
    applyAccount(view);
    notice.value = success(view);
  } catch (cause) {
    if (!disposed) error.value = String(cause);
  } finally {
    inFlight = false;
    if (!disposed) action.value = '';
  }
}

async function toggle() {
  const view = account.value;
  if (!view) return;
  await perform(view.enabled ? '正在暂停自动买卖' : '正在恢复自动买卖', () => invoke<FollowView>('research_follow_update', {
    accountId: view.account_id, maxPositions: null, enabled: !view.enabled,
  }), updated => updated.enabled ? '全自动模拟已开启，程序会自行下单、撮合和记录成交，无需逐笔确认。' : '自动买卖已暂停，已有持仓与成交保留，未成交委托已撤销。提醒开关没有改变。');
}

async function showRemoval() {
  if (!account.value || busy.value || inFlight || !props.active || disposed) return;
  removalId.value = account.value.account_id;
  await nextTick();
  removalPanel.value?.querySelector<HTMLButtonElement>('.follow-delete-cancel')?.focus();
}
async function removeModel() {
  const view = account.value;
  if (!view || removalId.value !== view.account_id || busy.value || inFlight || !props.active || disposed) return;
  const id = view.account_id;
  inFlight = true; action.value = '正在停止并删除模型账户'; error.value = ''; notice.value = '';
  let deleted = false;
  try {
    await invoke('research_follow_delete', { accountId: id });
    deleted = true;
    if (disposed) return;
    accounts.value = accounts.value.filter(item => item.account_id !== id);
    knownOrders.delete(id); delete newOrderIds.value[id]; removalId.value = null;
    if (selectedId.value === id) selectedId.value = accounts.value[0]?.account_id ?? '';
    notice.value = view.model_name + '的自动账户已删除，程序不再运行此账户；此模型不会因下次自动筛选而重新建账。';
    emit('refreshSources');
    const views = await invoke<FollowView[]>('research_follow_accounts');
    if (disposed) return;
    views.forEach(rememberOrders); accounts.value = views;
    if (!views.some(item => item.account_id === selectedId.value)) selectedId.value = views[0]?.account_id ?? '';
    readError.value = '';
  } catch (cause) {
    if (!disposed) error.value = deleted ? '模型账户已删除，其他账户状态暂时读取失败：' + String(cause) : '删除失败：' + String(cause);
  } finally { inFlight = false; if (!disposed) action.value = ''; }
}

async function refreshContext() {
  const view = account.value;
  if (!view) return;
  await perform('正在刷新原模型与委托', () => invoke<FollowView>('research_follow_refresh', { accountId: view.account_id }),
    () => '原模型上下文与委托已刷新，请核对数据日期、报价时间和委托状态。');
}

async function toggleAutomatic() {
  if (!automatic.value || disposed || !props.active || inFlight) return;
  const enabled = !automatic.value.enabled;
  inFlight = true; action.value = enabled ? '正在启用自动运行' : '正在暂停自动运行'; error.value = ''; notice.value = '';
  try {
    const status = await invoke<FollowAutomaticStatus>('research_auto_status', { enabled });
    if (disposed) return;
    automatic.value = status; automaticError.value = '';
    notice.value = enabled ? '已启用后台自动检查，复用现有账户；用户暂停或删除的模型不会自动恢复。' : '自动发现与买卖已统一暂停，未成交委托撤销，账户和持仓保留。提醒设置独立。';
  } catch (cause) { if (!disposed) error.value = String(cause); }
  finally { inFlight = false; if (!disposed) { action.value = ''; void readAccounts(true); } }
}
function openAccount(id?: number) { simulationAccountId.value = id ?? account.value?.account_id; showSimulation.value = true; }
watch(showSimulation, open => { if (!open) void readAccounts(true); });
const totals = computed(() => ({
  running: automatic.value?.effective_enabled === false ? 0 : accounts.value.filter(view => view.enabled && view.effective_enabled !== false).length,
  positions: accounts.value.reduce((sum, view) => sum + view.positions.length, 0),
  cash: accounts.value.reduce((sum, view) => sum + view.cash_cny, 0),
  equity: accounts.value.length && accounts.value.every(view => typeof view.equity_cny === 'number' && Number.isFinite(view.equity_cny))
    ? accounts.value.reduce((sum, view) => sum + view.equity_cny!, 0) : null,
}));
function syncPolling() {
  if (timer) clearInterval(timer);
  timer = undefined;
  if (!mounted || disposed || !props.active) return;
  void readAccounts(true);
  timer = setInterval(() => void readAccounts(true), 10000);
}
watch(() => props.active, syncPolling);
onMounted(() => { mounted = true; syncPolling(); });
onBeforeUnmount(() => {
  disposed = true;
  mounted = false;
  if (timer) clearInterval(timer);
  timer = undefined;
  knownOrders.clear();
});
defineExpose({ refresh: readAccounts });
</script>

<template>
  <section class="model-follow" aria-label="自动模型模拟交易" :aria-busy="busy">
    <header class="follow-header"><div><h2>自动模型交易</h2><p>后台按已验证模型独立建账与交易，账户、资金和持仓集中查看。</p></div><button type="button" class="follow-open-accounts" @click="openAccount()">账户与资金设置</button></header>
    <p v-if="error || readError" class="follow-error" role="alert">{{ error || readError }}</p>
    <p v-if="action || notice" class="follow-notice" role="status">{{ action || notice }}</p>
    <section class="follow-automatic-control" aria-label="自动运行总开关">
      <div><b>{{automatic ? autoStateName(automatic.state) : '正在读取自动运行状态…'}}</b><p>{{automatic?.message || automaticError || '后台状态读取不会新建账户或重跑模型。'}}</p><small v-if="automatic?.next_check_at">下一次检查 {{timestamp(automatic.next_check_at)}}</small><small v-else-if="automatic?.last_completed_day">上次完成 {{automatic.last_completed_day}} · 本日已处理的模型不会重复初始化</small><small v-if="automatic?.enabled && !automatic.effective_enabled">智能总开关当前关闭；本开关保留启用设置，自动交易仍暂停。</small></div>
      <button type="button" class="follow-automatic-switch" role="switch" :aria-checked="automatic?.enabled ?? false" :disabled="busy || !active || !automatic" @click="toggleAutomatic">{{automatic?.enabled ? '暂停自动运行' : '开启自动运行'}}</button>
    </section>
    <p v-if="automaticError && automatic" class="follow-error" role="alert">{{automaticError}}</p>
    <details v-if="automatic?.models.length" class="follow-model-outcomes"><summary>本轮模型结果 · {{automatic.models.length}} 项</summary><ul><li v-for="row in automatic.models" :key="row.model_id"><b>{{modelName(row.model_id)}}</b><span>{{outcomeName(row)}}</span><small>{{row.message || row.error}}</small></li></ul><p v-if="automatic.last_error" class="follow-error">{{automatic.last_error}}</p></details>
    <dl v-if="accounts.length" class="follow-overview"><div><dt>模型账户</dt><dd>{{accounts.length}} <small>运行 {{totals.running}}</small></dd></div><div><dt>持仓合计</dt><dd>{{totals.positions}} 只</dd></div><div><dt>账户净值合计</dt><dd>{{money(totals.equity)}}</dd></div><div><dt>账户现金合计</dt><dd>{{money(totals.cash)}}</dd></div></dl>
    <p v-if="!loaded && !error && !readError" class="follow-muted" role="status">{{ active ? '正在读取自动账户…' : '页面未激活，账户轮询已暂停。' }}</p>
    <nav v-if="accounts.length" class="follow-account-grid" aria-label="自动模型账户切换">
      <button v-for="view in accounts" :key="view.account_id" type="button" class="follow-account-card" :class="{selected:selectedId===view.account_id}" :aria-pressed="selectedId===view.account_id" :disabled="!!action || !active" :data-account-id="view.account_id" @click="selectedId=view.account_id">
        <span class="follow-card-heading"><b>{{view.model_name}}</b><span class="follow-badge" :class="{paused:!view.enabled}">{{accountState(view)}}</span></span>
        <small>#{{view.account_id}} · 持仓 {{view.positions.length}} / {{view.max_positions}} 只<template v-if="modelResult(view.model_id)"> · {{outcomeName(modelResult(view.model_id)!)}}</template></small>
        <span class="follow-card-money"><span>净值 <b>{{money(view.equity_cny)}}</b></span><span>现金 <b>{{money(view.cash_cny)}}</b></span></span>
        <span class="follow-card-holdings">{{view.positions.length?view.positions.slice(0,3).map(p=>(p.name||p.symbol)+' '+p.quantity+'股').join(' · '):'空仓 · 等待模型与成交条件'}}{{view.positions.length>3?' · 另'+(view.positions.length-3)+'只':''}}</span>
      </button>
    </nav>
    <p v-if="loaded && !accounts.length" class="follow-empty">当前没有自动模型账户。自动运行会检查模型并为符合条件的模型建账；无需逐只加入观察或手动选择来源账户。</p>
    <div v-if="account" :key="account.account_id" class="follow-selected-account" :data-selected-account-id="account.account_id">
      <div class="follow-account-heading"><h3>{{ account.model_name }} <small>#{{account.account_id}}</small></h3><span>数据截至 {{ timestamp(account.as_of) }} · {{accountState(account)}}</span></div>
      <p v-if="account.message" class="follow-muted">{{ account.message }}</p>
      <p v-if="!account.enabled" class="follow-paused" role="status">暂停期间不成交。已有持仓保留，未成交委托已撤销；自动发现不会恢复你暂停的账户。</p>
            <dl class="follow-metrics">
        <div><dt>账户现金</dt><dd>{{ money(account.cash_cny) }}</dd></div>
        <div><dt>账户净值</dt><dd>{{ money(account.equity_cny) }}</dd></div>
        <div><dt>净收益率</dt><dd>{{ percent(account.net_return_pct) }}</dd></div>
        <div><dt>初始现金</dt><dd>{{ money(account.initial_cash_cny) }}</dd></div>
        <div v-if="account.reserved_cash_cny !== undefined"><dt>待买预留</dt><dd>{{ money(account.reserved_cash_cny) }}</dd></div>
        <div v-if="account.available_cash_cny !== undefined"><dt>可用下单现金</dt><dd>{{ money(account.available_cash_cny) }}</dd></div>
      </dl>
      <p v-if="account.valuation_note" class="follow-muted">{{ account.valuation_note }}</p>

      <p v-if="signalSchedule" class="follow-muted follow-signal-schedule"><b>{{signalSchedule.label}}</b> · {{signalSchedule.note}}</p>
      <div class="follow-daily-controls" aria-label="当前账户控制">
        <button type="button" class="follow-trading-switch" :disabled="busy || !active" @click="toggle">{{ account.enabled ? '暂停此账户买卖' : '恢复此账户买卖' }}</button>
        <button type="button" class="follow-account-details" @click="openAccount(account.account_id)">账户明细与配置</button>
        <button type="button" class="follow-delete-trigger" :disabled="busy || !active" @click="showRemoval">删除模型账户</button>
      </div>
      <section v-if="removalId === account.account_id" ref="removalPanel" class="follow-delete-panel" role="alertdialog" aria-labelledby="follow-delete-title" aria-describedby="follow-delete-description" @keydown.esc.stop="removalId=null">
        <div class="follow-delete-heading"><span class="follow-delete-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 10v7m4-7v7" /></svg></span><div><h3 id="follow-delete-title">删除 {{account.model_name}} 的自动账户？</h3><p>专属模拟账户 #{{account.account_id}}</p></div></div>
        <div class="follow-delete-counts"><span>持仓 <b>{{account.positions.length}}</b> 只</span><span>未成交 <b>{{account.orders.filter(activeOrder).length}}</b> 笔</span><span>已成交 <b>{{account.performance.filled_buys + account.performance.filled_sells}}</b> 笔</span></div>
        <p id="follow-delete-description">删除会停止此模型自动建账并清除本账户的模拟持仓、委托与成交记录。下次筛选不会重建此模型账户；原模型和研究结果保留。</p>
        <div class="follow-delete-actions"><button type="button" class="follow-delete-cancel" :disabled="busy" @click="removalId=null">保留账户</button><button type="button" class="follow-delete-confirm" :disabled="busy || !active" @click="removeModel">{{action === '正在停止并删除模型账户' ? '正在删除…' : '删除此模型账户'}}</button></div>
      </section>
      <section class="follow-section" aria-label="当前持仓">
        <h3>当前持仓 <span>{{ account.positions.length }} 只 / 上限 {{ account.max_positions }} 只</span></h3>
        <p v-if="!account.positions.length" class="follow-empty">当前空仓，程序已提交但未成交的买单尚未计入持仓。</p>
        <div class="follow-position-grid">
          <article v-for="position in account.positions" :key="position.symbol" class="follow-position">
            <b>{{ position.name || position.symbol }} <span>{{ position.symbol }}</span></b>
            <dl class="follow-row-data">
              <div><dt>持有数量</dt><dd>{{ position.quantity }} 股</dd></div>
              <div><dt>T+1 可卖量</dt><dd>{{ position.available_quantity }} 股</dd></div>
              <div><dt>成本价 / 股</dt><dd>{{ money(position.cost_cny) }}</dd></div>
              <div><dt>估值价格</dt><dd>{{ money(position.mark_cny) }}</dd></div>
              <div v-if="position.mark_at"><dt>估值数据时间</dt><dd>{{ timestamp(position.mark_at) }}</dd></div>
              <div><dt>入场日期</dt><dd>{{ timestamp(position.entry_date) }}</dd></div>
              <div><dt>已持交易日</dt><dd>{{ position.holding_sessions }} 日</dd></div>
            </dl>
            <p v-if="position.exit_reason" class="follow-muted">计划退出原因：{{ position.exit_reason }}</p>
          </article>
        </div>
      </section>

      <details v-for="group in orderGroups" :key="group.key" class="follow-order-group" :open="group.key!=='closed'"><summary>{{group.title}} · {{group.total}} 笔{{ group.total > group.rows.length ? ' · 显示最近10笔' : '' }}</summary><section class="follow-section" :aria-label="group.title">
        <p v-if="group.key === 'active'" class="follow-muted">信号表示模型候选，委托已提交表示等待成交；只有“已成交”才计入持仓与收益。自动买卖交易提醒在实际成交后发送，显示股数与成交价；关闭提醒仍继续交易。</p>
        <p v-if="!group.rows.length" class="follow-empty">{{ group.key === 'active' ? '当前没有待处理委托。' : '暂无模拟成交。' }}</p>
        <article v-for="order in group.rows" :key="order.id" class="follow-order" :class="{ 'is-new': currentNewOrderIds.includes(order.id) }">
          <header><b>{{ order.side === 'buy' ? '买入' : '卖出' }} {{ order.name || order.symbol }} · {{ order.quantity }} 股</b><span>{{ order.symbol }} · #{{ order.id }}</span><span class="follow-badge" :class="{ 'needs-confirmation': order.status === 'awaiting_confirmation' }">{{ statusName(order.status) }}</span><strong v-if="currentNewOrderIds.includes(order.id)" class="follow-new">新委托</strong></header>
          <p v-if="order.status==='filled'" class="follow-fill-price">实际成交 {{order.quantity}} 股 · 成交价 {{money(order.filled_price_cny)}} / 股 · {{timestamp(order.filled_at)}}</p>
          <p v-else class="follow-muted">计划 {{order.quantity}} 股 · 限价 {{money(order.limit_price_cny)}} · 有效期 {{timestamp(order.valid_until)}}</p>
          <details class="follow-order-detail"><summary>报价与委托详情（可选）</summary>
          <dl class="follow-row-data follow-order-data">
            <div><dt>限价</dt><dd>{{ money(order.limit_price_cny) }}</dd></div>
            <div><dt>预计费用</dt><dd>{{ money(order.estimated_fee_cny) }}</dd></div>
            <div><dt>信号数据日期</dt><dd>{{ timestamp(order.signal_date) }}</dd></div>
            <div><dt>有效期至</dt><dd>{{ timestamp(order.valid_until) }}</dd></div>
            <div><dt>报价时间</dt><dd>{{ timestamp(order.quote_at) }}</dd></div>
            <div><dt>生成时间</dt><dd>{{ timestamp(order.created_at) }}</dd></div>
            <div><dt>{{ order.automatic_submission ? '自动提交时间' : '历史确认时间' }}</dt><dd>{{ timestamp(order.confirmed_at) }}</dd></div>
            <template v-if="order.status === 'filled'">
              <div><dt>成交时间</dt><dd>{{ timestamp(order.filled_at) }}</dd></div>
              <div><dt>成交价格</dt><dd>{{ money(order.filled_price_cny) }}</dd></div>
              <div><dt>成交费用</dt><dd>{{ money(order.fee_cny) }}</dd></div>
            </template>
          </dl>
          </details>
          <p class="follow-muted">{{ order.side === 'buy' ? '买入' : '卖出' }}原因：{{ order.reason || '--' }}</p>
          <p v-if="order.reject_reason" class="follow-error">未执行原因：{{ order.reject_reason }}</p>
        </article>
      </section></details>



      <details class="follow-account-options"><summary>原模型规则与研究证据（可选）</summary><div class="follow-optional-content">
        <p class="follow-muted">资金与持股数量在“账户明细与配置”中管理；各模型推荐配置由后端按已有研究匹配，收益以本账户实际成交为准。</p>
        <p v-if="account.comparison_note" class="follow-muted">{{account.comparison_note}}</p>
      <details class="follow-rules">
        <summary>原模型执行规则与来源</summary>
        <p>本账户每票入场目标 {{ account.execution.position_pct }}% 总资产 · 最长计划持有 {{ account.execution.holding_days }} 个交易日 · 最大跳空 {{ percent(account.execution.max_gap_pct) }} · 买入窗口 {{ account.execution.buy_window || '--' }}</p>
        <p>持有期是计划上限，原模型风险条件可提前触发退出。到期由程序自动提交卖单，等待可成交的新盘口；停牌、无对手盘或权益待核对时，实际持有可能超期。多年及年度收益是反复交易的账户统计。</p>
        <p v-if="account.execution.intraday_note">{{ account.execution.intraday_note }}</p>
        <p v-if="account.execution.sell_window">盘中卖出：{{ account.execution.sell_window }}</p>
        <p v-if="account.execution.entry_note || account.execution.entry_policy">实际入场：{{ account.execution.entry_note || account.execution.entry_policy }}</p>
        <p v-if="account.execution.exit_note || account.execution.exit_policy">实际退出：{{ account.execution.exit_note || account.execution.exit_policy }}</p>
        <p v-if="account.execution.allocation_note">{{ account.execution.allocation_note }}</p>
        <p>{{ account.execution.fee_note || '费用以服务端委托和成交记录为准。' }}</p>
        <p>模型 {{ account.model_id }} · 来源账户 #{{ account.source_run_id }}</p>
        <p class="follow-fingerprint">来源校验 {{ account.source_sha256 || '--' }}</p>
      </details>

      <details v-if="account.allocation_research" class="follow-rules follow-allocation" aria-label="少持股仓位研究">
        <summary>本模型仓位与退出研究 · {{ account.allocation_research.admitted ? '本数量下的新预算通过模拟复验' : '保留原预算基准' }}</summary>
        <p>{{ account.allocation_research.decision }}</p>
        <p class="follow-muted">最多 {{ account.max_positions }} 只 · {{ timestamp(account.allocation_research.start) }} 至 {{ timestamp(account.allocation_research.end) }} 的累计历史结果。先用 {{ account.allocation_research.early_period }} 选择，再查 {{ account.allocation_research.later_period }}。{{ account.allocation_research.selection_note }}</p>
        <div v-if="account.allocation_research.slot_comparison?.length" class="follow-slot-comparison">
          <p><b>持股数量对照 · 原每票8%含费基准</b></p>
          <table><thead><tr><th>最多持股</th><th>总入场目标</th><th>累计净收益</th><th>最大回撤</th></tr></thead>
            <tbody><tr v-for="row in account.allocation_research.slot_comparison" :key="row.max_positions"><td>{{ row.max_positions }} 只</td><td>{{ row.max_positions * 8 }}%</td><td>{{ percent(row.net_return_pct) }}</td><td>{{ percent(row.max_drawdown_pct) }}</td></tr></tbody>
          </table>
          <p class="follow-muted">以下再比较当前数量的不同资金方案。默认选择同时检查回撤、成本和延迟，不能只按累计收益排序。</p>
        </div>
        <div class="follow-allocation-grid">
          <article v-for="policy in account.allocation_research.policies" :key="policy.id" class="follow-allocation-policy">
            <b>{{ policy.name }}</b>
            <dl class="follow-row-data">
              <div><dt>累计净收益</dt><dd>{{ percent(policy.net_return_pct) }}</dd></div>
              <div><dt>最大回撤</dt><dd>{{ percent(policy.max_drawdown_pct) }}</dd></div>
              <div><dt>总入场预算</dt><dd>{{ percent(policy.total_entry_pct) }}</dd></div>
              <div><dt>历史平均投入</dt><dd>{{ percent(policy.average_invested_pct) }}</dd></div>
            </dl>
          </article>
        </div>
        <p v-if="account.allocation_research.recommendation">本模型独立默认 {{ account.allocation_research.recommendation.default_max_positions }} 只：{{ account.allocation_research.recommendation.reason }}</p>
        <p>较早年份选择：{{ account.allocation_research.selected }}。{{ account.allocation_research.reasons.join('；') }}。</p>
        <p>{{ account.allocation_research.exit_decision }}。</p>
        <p class="follow-muted">新配置单票入场上限 20%；实际金额仍受现金、费用、整手和流动性限制，涨跌后暴露会变化。已核对 {{ account.allocation_research.ledger_count }} 个历史账本；费用、迟到和盘口差异会影响实际跟随结果。</p>
      </details>

      <details v-if="timingResearch" class="follow-rules follow-timing" aria-label="本模型入场与提前退出研究">
        <summary>本模型入场与提前退出研究 · <template v-if="!timingResearch.current_configuration_matches">当前配置与该研究配置不一致，本轮时点证据不适用；实际按上方执行规则</template><template v-else>{{ timingResearch.admitted ? '所选方案通过模拟复验' : '保留基准执行' }}</template></summary>
        <p v-if="!timingResearch.current_configuration_matches" class="follow-paused" role="status">当前配置与该研究配置不一致，本轮时点证据不适用；实际按上方执行规则。研究配置最多 {{ timingResearch.studied_max_positions }} 只，当前最多 {{ account.max_positions }} 只。</p>
        <p>{{ timingResearch.decision }}</p>
        <p v-if="timingResearch.characteristics">本模型特点：{{ timingResearch.characteristics }}</p>
        <p class="follow-muted">本模型独立研究，方案不跨模型通用。研究针对最多 {{ timingResearch.studied_max_positions }} 只 · {{ timestamp(timingResearch.start) }} 至 {{ timestamp(timingResearch.end) }}，当前展示的是多年累计历史结果。</p>
        <p class="follow-muted">早段选择：{{ timingResearch.early_period }}；后段检查：{{ timingResearch.later_period }}。两段均为既有历史，不代表全新样本外验证，也不能据此认定未来最优方案。</p>
        <p>15 / 20 个交易日均为最长计划持有上限，可按各模型的退出条件提前退出；提前退出轮次和平均持有天数来自历史账本。</p>
        <p>早段所选方案：<b>{{ timingPolicyName(timingResearch.selected_policy) }}</b>。<template v-if="timingResearch.current_configuration_matches">当前实际执行：<b>{{ timingPolicyName(timingResearch.active_policy) }}</b>。</template><template v-else>本轮实际入场与退出见上方执行规则。</template></p>
        <p v-if="timingResearch.reasons.length">复验依据：{{ timingResearch.reasons.join('；') }}</p>
        <div class="follow-allocation-grid">
          <article v-for="policy in timingResearch.policies" :key="policy.id" class="follow-allocation-policy follow-timing-policy">
            <header class="follow-timing-policy-heading"><b>{{ policy.name }}</b><span v-if="policy.id === timingResearch.selected_policy" class="follow-badge">早段选择</span><span v-if="timingResearch.current_configuration_matches && policy.id === timingResearch.active_policy" class="follow-badge">当前执行</span></header>
            <p>入场：{{ policy.entry_note || '--' }}</p>
            <p>退出：{{ policy.exit_note || '--' }}</p>
            <dl class="follow-row-data">
              <div><dt>最长计划持有</dt><dd>{{ policy.max_holding_sessions }} 个交易日（上限）</dd></div>
              <div><dt>历史平均持有</dt><dd>{{ policy.mean_holding_sessions === null ? '--' : policy.mean_holding_sessions.toFixed(1) + ' 个交易日' }}</dd></div>
              <div><dt>累计历史净收益</dt><dd>{{ percent(policy.net_return_pct) }}</dd></div>
              <div><dt>历史最大回撤</dt><dd>{{ percent(policy.max_drawdown_pct) }}</dd></div>
              <div><dt>提前退出轮次</dt><dd>{{ policy.early_exit_cycles }}</dd></div>
              <div><dt>已完成轮次</dt><dd>{{ policy.completed_cycles }}</dd></div>
            </dl>
          </article>
        </div>
        <template v-if="timingResearch.stress.length">
          <p><b>后段压力复验 · {{ timingResearch.later_period }} 累计历史净收益</b></p>
          <div class="follow-allocation-grid">
            <article v-for="row in timingResearch.stress" :key="row.name" class="follow-allocation-policy follow-timing-policy">
              <b>{{ row.name }}</b>
              <dl class="follow-row-data">
                <div><dt>基准后段累计净收益</dt><dd>{{ percent(row.baseline_later_net_return_pct) }}</dd></div>
                <div><dt>早段所选方案后段累计净收益</dt><dd>{{ percent(row.selected_later_net_return_pct) }}</dd></div>
              </dl>
            </article>
          </div>
        </template>
        <p v-if="timingResearch.selection_note" class="follow-muted">{{ timingResearch.selection_note }}</p>
        <p class="follow-muted">已核对 {{ timingResearch.ledger_count }} 个历史账本。实际入场和退出以本账户执行规则与委托记录为准。</p>
      </details>


        <div class="follow-maintenance"><button type="button" :disabled="busy || !active" @click="refreshContext">重新核验模型与委托</button><p class="follow-muted">日常无需刷新，后台持续检查；缺数据或条件未满足时等待。</p></div>
      </div></details>
      <details class="follow-plan-details"><summary>模型候选、执行偏差与历史统计（可选）</summary><div class="follow-optional-content">
      <section class="follow-section" aria-label="原模型有序候选">
        <h3>原模型有序候选 <span>{{ candidates.length }} 只</span></h3>
        <p class="follow-muted">数据日期 {{ timestamp(account.as_of) }}。沿用原模型排名与评分，参考股数不等于委托；实际计划受剩余名额、现金与执行条件限制。</p>
        <p v-if="!candidates.length" class="follow-empty">当前没有原模型候选，等待下一次数据更新。</p>
        <ol v-else class="follow-candidates">
          <li v-for="candidate in candidates" :key="candidate.symbol"><div><b>#{{ candidate.rank }} {{ candidate.symbol }}</b><span>原模型评分 {{ modelScore(candidate.score, account.model_id) }} · 参考 {{ candidate.reference_quantity }} 股</span></div><p>{{ candidate.reason || '--' }}</p></li>
        </ol>
      </section>
      <section class="follow-section" aria-label="计划执行偏差">
        <h3>计划执行偏差与计数</h3>
        <dl class="follow-performance">
          <div><dt>买入成交</dt><dd>{{ account.performance.filled_buys }} 笔</dd></div>
          <div><dt>卖出成交</dt><dd>{{ account.performance.filled_sells }} 笔</dd></div>
          <div><dt>撤销</dt><dd>{{ account.performance.cancelled }} 笔</dd></div>
          <div><dt>到期</dt><dd>{{ account.performance.expired }} 笔</dd></div>
        </dl>
        <p class="follow-muted">盘口等待、撤销和到期会使执行偏离原模型计划。请以本账户实际成交与净值核对跟随结果；未知净值与收益显示“--”。</p>
      </section>
      </div></details>

    </div>
    <details class="follow-selection-help"><summary>自动流程与提醒怎样使用？</summary><p>保持应用运行。A 股交易日 09:00 自动更新；错过时间会在启动后补做。更新成功后等待 10 分钟，后台逐个检查既有验证模型，按模型推荐配置自动建账、准备计划。账户初始资金沿用模拟账户中的统一预设。</p><p>符合候选不等于成交。完成日日线确定信号，下一交易日有效买入窗口执行；盘中持续检查已有委托与退出条件，受可用现金、名额、T+1 和盘口限制，可以少买或空仓。</p><p>暂停此账户买卖会撤销其未成交委托并保留持仓。删除会清理本账户账本并停止该模型自动建账。研究提醒统一到“提醒与记录”设置，关闭提醒不会暂停买卖；手动观察不使用自动账户。</p></details>
    <SimulationDialog v-if="showSimulation" v-model:show="showSimulation" :preferred-account-id="simulationAccountId" />
  </section>
</template>

<style scoped>
.model-follow{width:100%;min-width:0;display:grid;gap:12px;padding:16px;border:1px solid var(--color-border-0);border-radius:12px;background:var(--color-surface-1);color:var(--color-text-primary);font-size:13px;line-height:1.7;overflow-wrap:anywhere}
.model-follow *, .model-follow *::before, .model-follow *::after{box-sizing:border-box;min-width:0}
.model-follow h2,.model-follow h3,.model-follow p,.model-follow dl{margin:0}
.model-follow h2{font-size:16px}.model-follow h3{font-size:14px}.model-follow h3 span{font-weight:400;font-size:12px;color:var(--color-text-secondary)}
.model-follow button,.model-follow input,.model-follow select{max-width:100%;border:1px solid var(--color-border-0);border-radius:8px;padding:7px 10px;background:var(--color-surface-1);color:var(--color-text-primary);font:inherit;line-height:1.5}
.model-follow input,.model-follow select{width:100%}.model-follow button{cursor:pointer;white-space:normal}.model-follow button:disabled{opacity:.5;cursor:default}.model-follow button.primary{background:var(--color-accent-dim);border-color:var(--color-accent);color:var(--color-accent)}
.model-follow button:focus-visible,.model-follow input:focus-visible,.model-follow select:focus-visible,.model-follow summary:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
.follow-header,.follow-account-heading,.follow-section-heading,.follow-order header,.follow-order-actions,.follow-account-controls{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.follow-header>div,.follow-header p,.follow-muted,.follow-empty,.follow-account-heading>span{color:var(--color-text-secondary);font-size:12px}.follow-note{padding:10px 12px;border-radius:8px;background:var(--color-accent-dim);font-size:12px}.follow-note strong{display:inline-block;margin-left:4px}
.follow-error{color:var(--color-error);white-space:pre-wrap}.follow-notice{color:var(--color-accent)}.follow-paused{padding:8px 10px;border-radius:8px;color:var(--color-warning);background:var(--color-warning-bg)}
.follow-account-heading>span{flex:1}.follow-badge{border-radius:6px;background:var(--color-surface-2);color:var(--color-text-secondary);padding:2px 7px;font-size:12px}.follow-badge.needs-confirmation{background:var(--color-warning-bg);color:var(--color-warning)}
.follow-metrics{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px}.follow-metrics>div{padding:10px;border:1px solid var(--color-border-0);border-radius:8px}.model-follow dt{color:var(--color-text-secondary);font-size:12px}.model-follow dd{margin:0;font-variant-numeric:tabular-nums}.follow-metrics dd{font-size:17px;font-weight:600}.follow-account-controls{align-items:end}.follow-account-controls label{width:160px}.follow-rules,.follow-selection-help{font-size:12px;color:var(--color-text-secondary)}.follow-rules summary,.follow-selection-help summary{cursor:pointer;color:var(--color-text-primary)}.follow-rules p,.follow-selection-help p{margin-top:6px}.follow-fingerprint{word-break:break-all}
.follow-section{display:grid;gap:10px;border-top:1px solid var(--color-border-0);padding-top:14px}.follow-section-heading h3{flex:1}.follow-section-heading button{font-size:12px}.follow-position-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,260px),1fr));gap:10px}.follow-position,.follow-order{padding:12px;border:1px solid var(--color-border-0);border-radius:10px;background:var(--color-surface-2);display:grid;gap:8px}.follow-position b span,.follow-order header>span:not(.follow-badge){font-size:12px;color:var(--color-text-secondary)}.follow-row-data{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px 12px}.follow-order-data{grid-template-columns:repeat(3,minmax(0,1fr))}.follow-order.is-new{border-color:var(--color-accent);box-shadow:inset 3px 0 var(--color-accent)}.follow-new{color:var(--color-accent);font-size:12px}.follow-order-actions{justify-content:flex-end}
.follow-candidates{list-style:none;display:grid;gap:8px;padding:0;margin:0}.follow-candidates li{border-bottom:1px solid var(--color-border-0);padding:8px 0}.follow-candidates li:last-child{border:0}.follow-candidates li>div{display:flex;gap:10px;flex-wrap:wrap;align-items:baseline}.follow-candidates span,.follow-candidates p{font-size:12px;color:var(--color-text-secondary)}.follow-performance{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:10px}
@media(max-width:760px){.model-follow{padding:12px}.follow-metrics,.follow-order-data{grid-template-columns:repeat(2,minmax(0,1fr))}.follow-performance{grid-template-columns:repeat(3,minmax(0,1fr))}.follow-header>div{flex-basis:100%}}
@media(max-width:480px){.follow-create-fields,.follow-row-data,.follow-performance{grid-template-columns:minmax(0,1fr)}.follow-account-controls label{width:100%}.follow-account-controls button{flex:1}.follow-metrics dd{font-size:15px}}
.follow-slot-comparison table{width:100%;table-layout:fixed;border-collapse:collapse;font-variant-numeric:tabular-nums;margin:8px 0}.follow-slot-comparison th,.follow-slot-comparison td{padding:6px 8px;text-align:right;border-bottom:1px solid var(--color-border-0)}.follow-slot-comparison th:first-child,.follow-slot-comparison td:first-child{text-align:left}.follow-slot-comparison th{color:var(--color-text-primary);font-weight:500}
.follow-allocation-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,210px),1fr));gap:10px;margin:12px 0}.follow-allocation-policy{border:1px solid var(--color-border-0);border-radius:8px;padding:12px;min-width:0;background:var(--color-surface-2)}
.follow-timing-policy{display:grid;gap:8px;align-content:start}.follow-timing-policy p{margin:0}.follow-timing-policy-heading{display:flex;align-items:center;gap:6px;flex-wrap:wrap}.follow-timing-policy-heading b{flex:1;color:var(--color-text-primary)}
.follow-account-options,.follow-plan-details,.follow-order-group{min-width:0}.follow-account-options>summary,.follow-plan-details>summary,.follow-order-group>summary{cursor:pointer;font-size:13px;color:var(--color-text-primary);padding:6px 0}.follow-account-options,.follow-plan-details{padding:10px 12px;border:1px solid var(--color-border-0);border-radius:10px}.follow-optional-content{display:grid;gap:14px;margin-top:12px;min-width:0}.follow-daily-controls{display:flex;align-items:center;gap:8px;flex-wrap:wrap;padding:12px;border-radius:10px;background:var(--color-surface-2)}.follow-daily-controls>p{flex-basis:100%;font-size:12px;color:var(--color-text-secondary)}.follow-daily-controls .follow-error{color:var(--color-error)}.follow-daily-controls [aria-checked="true"]{border-color:var(--color-accent);color:var(--color-accent)}.follow-trading-switch{font-weight:600}.follow-maintenance{display:flex;gap:8px;flex-wrap:wrap}.follow-maintenance p{flex-basis:100%}.follow-order-group .follow-section{border-top:0;padding-top:8px}.follow-order-detail{font-size:12px;color:var(--color-text-secondary)}.follow-order-detail>summary{cursor:pointer}.follow-order-detail[open]>.follow-row-data{margin-top:8px}.follow-fill-price{color:var(--color-accent);font-weight:600}@media(max-width:480px){.follow-daily-controls button{flex:1}}
.model-follow .follow-delete-trigger{margin-left:auto;color:var(--color-text-secondary)}.model-follow .follow-delete-trigger:hover:not(:disabled){color:var(--color-error,#c53f50);border-color:var(--color-error,#c53f50)}.follow-delete-panel{border:1px solid color-mix(in srgb,var(--color-error,#c53f50) 32%,var(--color-border-0));border-radius:12px;padding:16px;background:color-mix(in srgb,var(--color-error,#c53f50) 5%,var(--color-surface-1));min-width:0}.follow-delete-heading{display:flex;align-items:center;gap:10px}.follow-delete-heading h3{margin:0;font-size:14px;line-height:1.6;overflow-wrap:anywhere}.follow-delete-heading p{margin:3px 0 0;font-size:12px;color:var(--color-text-secondary)}.follow-delete-icon{display:grid;place-items:center;width:36px;height:36px;border-radius:10px;background:color-mix(in srgb,var(--color-error,#c53f50) 10%,var(--color-surface-1));color:var(--color-error,#c53f50);flex-shrink:0}.follow-delete-icon svg{width:21px;height:21px;fill:none;stroke:currentColor;stroke-width:1.6;stroke-linecap:round;stroke-linejoin:round}.follow-delete-counts{display:flex;gap:8px;flex-wrap:wrap;margin:12px 0}.follow-delete-counts span{padding:5px 9px;border:1px solid var(--color-border-0);border-radius:7px;background:var(--color-surface-1);font-size:12px;color:var(--color-text-secondary)}.follow-delete-counts b{color:var(--color-text-primary);font-variant-numeric:tabular-nums}.follow-delete-panel>p{margin:0;font-size:12px;line-height:1.8;color:var(--color-text-secondary);overflow-wrap:anywhere}.follow-delete-actions{display:flex;justify-content:flex-end;gap:8px;margin-top:14px}.model-follow .follow-delete-confirm{color:#fff;background:var(--color-error,#c53f50);border-color:var(--color-error,#c53f50)}@media(max-width:480px){.follow-delete-panel{padding:12px}.follow-delete-actions button{flex:1}.model-follow .follow-delete-trigger{margin-left:0}}

.follow-header{display:flex;align-items:flex-start;justify-content:space-between;gap:12px;flex-wrap:wrap}.follow-header>div{flex:1;min-width:min(100%,260px)}.follow-header h2{font-size:17px}.follow-overview{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;margin:14px 0}.follow-overview>div{padding:12px;background:var(--color-surface-2);border-radius:10px}.follow-overview dt{font-size:12px;color:var(--color-text-secondary)}.follow-overview dd{margin:6px 0 0;font-size:18px;font-weight:600;overflow-wrap:anywhere;font-variant-numeric:tabular-nums}.follow-overview small{font-size:12px;font-weight:400;color:var(--color-text-secondary)}.follow-account-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,265px),1fr));gap:10px;margin:14px 0}.model-follow .follow-account-card{text-align:left;display:grid;gap:10px;padding:14px;border-radius:12px;min-width:0;white-space:normal}.model-follow .follow-account-card.selected{border-color:var(--color-accent);box-shadow:inset 0 0 0 1px var(--color-accent);background:var(--color-accent-dim)}.follow-card-heading{display:flex;justify-content:space-between;align-items:center;gap:8px;flex-wrap:wrap}.follow-account-card small,.follow-card-holdings{font-size:12px;color:var(--color-text-secondary);overflow-wrap:anywhere}.follow-card-money{display:grid;grid-template-columns:1fr 1fr;gap:10px;font-size:12px;color:var(--color-text-secondary)}.follow-card-money b{display:block;margin-top:4px;font-size:14px;color:var(--color-text-primary);overflow-wrap:anywhere;font-variant-numeric:tabular-nums}.follow-badge.paused{color:var(--color-text-secondary);background:var(--color-surface-2)}.follow-selected-account{display:grid;gap:12px;margin-top:16px;padding-top:16px;border-top:1px solid var(--color-border-0)}.follow-account-heading h3 small{font-size:12px;font-weight:400;color:var(--color-text-secondary)}.follow-daily-controls .follow-delete-trigger{margin-left:auto}@media(max-width:600px){.follow-overview{grid-template-columns:repeat(2,minmax(0,1fr))}.follow-header .follow-open-accounts{width:100%}.follow-daily-controls button{min-width:0}}

.follow-automatic-control{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:12px 14px;margin:14px 0;border:1px solid var(--color-border-0);border-radius:12px;background:var(--color-surface-2)}.follow-automatic-control>div{flex:1;min-width:0}.follow-automatic-control b{font-size:13px}.follow-automatic-control p{font-size:12px;line-height:1.7;margin:5px 0;color:var(--color-text-secondary);overflow-wrap:anywhere}.follow-automatic-control small{display:block;font-size:12px;line-height:1.6;color:var(--color-text-secondary)}.model-follow .follow-automatic-switch{flex-shrink:0;border-color:var(--color-accent);color:var(--color-accent);font-weight:600}.follow-model-outcomes{padding:10px 12px;border:1px solid var(--color-border-0);border-radius:10px;margin:12px 0;font-size:12px}.follow-model-outcomes summary{cursor:pointer;color:var(--color-text-secondary)}.follow-model-outcomes ul{padding:0;list-style:none;display:grid;gap:8px}.follow-model-outcomes li{display:flex;justify-content:space-between;gap:8px;flex-wrap:wrap}.follow-model-outcomes li small{flex-basis:100%;color:var(--color-text-secondary);line-height:1.7;overflow-wrap:anywhere}@media(max-width:480px){.follow-automatic-control{align-items:flex-start;flex-direction:column}.follow-automatic-switch{width:100%}}
</style>
