<script setup lang="ts">
import { computed, reactive, ref, watch, onBeforeUnmount } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { FollowAutomaticPreset, FollowView } from '@/types/modelFollow';
import {
  NAlert, NButton, NCard, NCheckbox, NFormItem, NInput, NInputNumber, NModal,
  NSelect, NSpace, NSwitch, NTabPane, NTabs, NTag, useMessage,
} from 'naive-ui';

type Mode = 'record' | 'confirm' | 'auto';
type Side = 'buy' | 'sell';
interface Target { symbol: string; name: string; rule: string; limit_bps: number }
interface Account {
  managed_by?: 'model_follow' | 'research' | 'manual';
  id: number; name: string; initial_cash: string; current_cash: string; mode: Mode;
  auto_enabled: boolean; manual_source_enabled: boolean; rule_source_enabled: boolean;
  ai_source_enabled: boolean; commission_bps: number; min_commission: string;
  stamp_tax_bps: number; transfer_fee_bps: number; slippage_bps: number;
  targets?: Target[];
}
interface Position { symbol: string; name?: string; quantity: number; available_quantity?: number; cost_price?: string }
interface Order { id: number; symbol: string; name?: string; side: Side; quantity: number; status: string; source?: string; signal_date: string; target_date?: string; price?: string; fee?: string; reject_reason?: string; decision_reason?: string }
interface Run { id: number; run_key?: string; trade_date?: string; phase?: string; status: string; progress?: number; message?: string; created_at?: string }
interface Metrics { win_rate_bps?: number; profit_loss_ratio_bps?: number; expectancy?: string; max_drawdown_bps?: number; total_return_bps?: number; annualized_return_bps?: number; average_holding_days_x100?: number; sample_count?: number; benchmark_return_bps?: number | null }
interface SourceStats { source: string; orders: number; filled: number; rejected: number; realized_profit: string }
interface CapitalAdjustment { id: number; old_initial_cash: string; new_initial_cash: string; delta: string; cash_before: string; cash_after: string; created_at: string }
interface Detail { capital_adjustments?: CapitalAdjustment[]; performance_note?: string; account: Account; targets: Target[]; positions: Position[]; orders: Order[]; recent_runs: Run[]; metrics: Metrics; source_stats: SourceStats[] }

const props = defineProps<{ show: boolean; preferredAccountId?: number }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();
const visible = computed({ get: () => props.show, set: value => emit('update:show', value) });
const message = useMessage();
const accounts = ref<Account[]>([]);
const selectedId = ref<number | null>(null);
const detail = ref<Detail | null>(null);
const busy = ref(false);
const loading = ref(false);
const explicitRefresh = ref(false);
const error = ref('');
const modelError = ref('');
const preset = ref<FollowAutomaticPreset | null>(null);
const presetCash = ref('');
const presetDirty = ref(false);
const presetError = ref('');
const presetExpanded = ref(false);
const followAccounts = ref<FollowView[]>([]);
const follow = computed(() => followAccounts.value.find(view => view.account_id === selectedId.value) ?? null);

const signalSchedule = computed(() => {
  const view = follow.value;
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
const maximum = ref<number | null>(null);
const maximumDirty = ref(false);
const validMaximum = computed(() => maximum.value !== null && Number.isInteger(maximum.value) && maximum.value >= 1 && maximum.value <= 10);
let detailRequest = 0;
let disposed = false;
const live=ref<{engine:string;message:string;updated_at:string|null;plans:Array<{symbol:string;buy_low:number;buy_high:number;stop:number;take:number}>;executions:unknown[]}|null>(null);
const targetsText = ref('');
const activeTab = ref<'overview' | 'account' | 'manual'>('overview');
const form = reactive({
  id: undefined as number | undefined,
  name: '模拟账户', initialCash: '100000.00', mode: 'record' as Mode, autoEnabled: false,
  manualSourceEnabled: true, ruleSourceEnabled: true,
  commissionBps: 3, minCommission: '5.00', stampTaxBps: 5,
  transferFeeBps: 0, slippageBps: 0,
});
const executionMode=ref('realtime');
const capitalCash=ref('100000.00');
const capitalDirty=ref(false);
const order = reactive({ symbol: '', name: '', side: 'buy' as Side, quantity: 100, signalDate: '', limitBps: 1000,limitPrice:'',stopPrice:'',takePrice:'' });

const accountOptions = computed(() => [
  { type: 'group' as const, label: '独立手动账户', key: 'manual', children: accounts.value.filter(a => !a.managed_by || a.managed_by === 'manual').map(a => ({ label: '#' + a.id + ' ' + a.name, value: a.id })) },
  { type: 'group' as const, label: '自动模型专用账户', key: 'model', children: accounts.value.filter(a => a.managed_by === 'model_follow').map(a => ({ label: '#' + a.id + ' ' + a.name + ' · 自动专用', value: a.id })) },
  { type: 'group' as const, label: '冻结研究 · 只读', key: 'research', children: accounts.value.filter(a => a.managed_by === 'research').map(a => ({ label: '#' + a.id + ' ' + a.name + ' · 只读', value: a.id })) },
].filter(group => group.children.length));
const selected = computed(() => accounts.value.find(account => account.id === selectedId.value));
const modelManaged = computed(() => selected.value?.managed_by === 'model_follow');
const managed = computed(() => modelManaged.value || selected.value?.managed_by === 'research');
const nextCheck = computed(() => modelManaged.value ? follow.value?.effective_enabled === false ? '全局已暂停，账户和持仓保留，当前不执行自动买卖' : '由原模型在交易时段自动检查；可在本账户配置中暂停买卖' : selected.value?.auto_enabled ? 'A 股连续竞价时段，上一轮完成约 3 秒后再次检查' : '未启用自动监听，可点击立即运行');

function initialCashCny(raw: string): number {
  if (!/^\d+(?:\.\d{1,2})?$/.test(raw.trim())) throw new Error('初始资金最多两位小数');
  const value=Number(raw);
  if (!Number.isFinite(value) || value < 1000 || value > 100000000) throw new Error('初始资金应为1000至1亿元');
  return value;
}

function decimalToScaled(raw: string): string {
  const match = raw.trim().match(/^(\d+)(?:\.(\d{0,4}))?$/);
  if (!match) throw new Error('金额需为非负数字，最多 4 位小数');
  return (BigInt(match[1]) * 10000n + BigInt((match[2] ?? '').padEnd(4, '0'))).toString();
}

function scaledToDecimal(raw?: string): string {
  if (!raw || !/^-?\d+$/.test(raw)) return '-';
  const signed = BigInt(raw); const value = signed < 0n ? -signed : signed; const whole = value / 10000n; const fraction = (value % 10000n).toString().padStart(4, '0').replace(/0+$/, '');
  return (signed < 0n ? '-' : '') + (fraction ? `${whole}.${fraction}` : whole.toString());
}

function bps(value?: number | null): string { return value == null || !Number.isFinite(value) ? '-' : `${(value / 100).toFixed(2)}%`; }
function statusType(status: string): 'success' | 'warning' | 'error' | 'default' {
  if (status === 'filled' || status === 'completed') return 'success';
  if (status === 'rejected' || status === 'failed') return 'error';
  if (status === 'pending' || status === 'awaiting_confirmation' || status === 'running') return 'warning';
  return 'default';
}

function parseTargets(): Target[] {
  const rows = targetsText.value.split(/[\n,，]+/).map(value => value.trim()).filter(Boolean);
  const unique = [...new Set(rows)];
  if (unique.length > 10) throw new Error('自动/手动标的最多 10 只，请先精简');
  return unique.map(symbol => ({ symbol: symbol.toLowerCase(), name: symbol, rule: 'trend_follow', limit_bps: boardLimitBps(symbol) }));
}

/** 板块涨跌幅（基点）的界面提示值；最终以交易所规则为准（主板 ST 自 2026-07-06 起也是 10%）。 */
function boardLimitBps(symbol: string): number {
  const value = symbol.trim().toLowerCase();
  if (value.startsWith('bj')) return 3000;
  if (/^sh68|^sz30/.test(value)) return 2000;
  return 1000;
}

/** 各板块买入申报单位：科创板 200 股起、北交所 100 股起，均可 1 股递增。 */
function buyLot(symbol: string): { min: number; step: number } {
  const value = symbol.trim().toLowerCase();
  if (value.startsWith('sh68')) return { min: 200, step: 1 };
  if (value.startsWith('bj')) return { min: 100, step: 1 };
  return { min: 100, step: 100 };
}

const orderLot = computed(() => buyLot(order.symbol));
const orderLimitBps = computed(() => boardLimitBps(order.symbol));

// 涨跌幅随代码自动判定，不再让用户填一个可能和板块不符的数字。
watch(orderLimitBps, value => { order.limitBps = value; }, { immediate: true });
watch(orderLot, value => { if (order.side === 'buy' && order.quantity < value.min) order.quantity = value.min; }, { immediate: true });

function resetForm() {
  Object.assign(form, { id: undefined, name: '模拟账户', initialCash: '100000.00', mode: 'record', autoEnabled: false, manualSourceEnabled: true, ruleSourceEnabled: true, commissionBps: 3, minCommission: '5.00', stampTaxBps: 5, transferFeeBps: 0, slippageBps: 0 });
  targetsText.value = '';executionMode.value='realtime';capitalDirty.value=false;capitalCash.value='100000.00';
}

function beginNewAccount() {
  selectedId.value = null;
  detail.value = null;
  error.value = '';
  resetForm();
  activeTab.value = 'account';
}

function editAccount(account: Account, targets: Target[] = account.targets ?? []) {
  Object.assign(form, {
    id: account.id, name: account.name, initialCash: scaledToDecimal(account.initial_cash), mode: account.mode,
    autoEnabled: account.auto_enabled, manualSourceEnabled: account.manual_source_enabled,
    ruleSourceEnabled: account.rule_source_enabled, commissionBps: account.commission_bps,
    minCommission: scaledToDecimal(account.min_commission), stampTaxBps: account.stamp_tax_bps,
    transferFeeBps: 0, slippageBps: 0,
  });
  targetsText.value = targets.map(target => target.symbol).join('\n');
}

async function readFollowAccounts() {
  try {
    const views = await invoke<FollowView[]>('research_follow_accounts');
    if (!disposed && props.show) { followAccounts.value = views; modelError.value = ''; }
  } catch (cause) { if (!disposed && props.show) modelError.value = '自动模型状态暂不可用：' + String(cause); }
}

async function readPreset() {
  try {
    const next = await invoke<FollowAutomaticPreset>('simulation_auto_preset');
    if (disposed || !props.show) return;
    preset.value = next; presetError.value = '';
    if (!presetDirty.value) presetCash.value = next.initial_cash_cny.toFixed(2);
  } catch (cause) { if (!disposed && props.show) presetError.value = '资金预设暂不可用：' + String(cause); }
}
function savePreset() {
  if (!preset.value) return;
  void act(async () => {
    const next = await invoke<FollowAutomaticPreset>('simulation_save_auto_preset', { input: { initial_cash_cny: initialCashCny(presetCash.value) } });
    preset.value = next; presetCash.value = next.initial_cash_cny.toFixed(2); presetDirty.value = false; presetError.value = '';
    message.success('新自动账户资金预设已保存，现有账户资金和持仓保留');
  });
}
async function loadAccounts(preferred?: number) {
  error.value = '';
  const [rows] = await Promise.all([invoke<Account[]>('simulation_list_accounts'), readFollowAccounts(), readPreset()]);
  if (disposed || !props.show) return;
  accounts.value = rows;
  const id = preferred ?? selectedId.value ?? props.preferredAccountId;
  if (id != null && !rows.some(row => row.id === id)) {
    selectedId.value = null; detail.value = null; live.value = null;
    error.value = '指定账户 #' + id + ' 已不存在，请主动选择其他账户。'; return;
  }
  selectedId.value = id ?? rows.find(row => row.managed_by === 'model_follow')?.id ?? rows[0]?.id ?? null;
  if (selectedId.value) await loadDetail(); else { detail.value = null; live.value = null; }
}

async function loadDetail() {
  if (!selectedId.value) return;
  const id = selectedId.value, request = ++detailRequest;
  const result = await Promise.all([
    invoke<Detail>('simulation_get_detail', { accountId: id }),
    invoke<NonNullable<typeof live.value>>('simulation_live_status', { accountId: id }),
    ...(modelManaged.value ? [readFollowAccounts()] : []),
  ]).catch(cause => {
    if (disposed || request !== detailRequest || selectedId.value !== id || !props.show) return null;
    throw cause;
  });
  if (!result || disposed || request !== detailRequest || selectedId.value !== id || !props.show) return;
  const [nextDetail, nextLive] = result;
  nextDetail.account.managed_by = nextDetail.account.managed_by ?? accounts.value.find(a => a.id === id)?.managed_by ?? 'manual';
  detail.value = nextDetail; live.value = nextLive; executionMode.value = nextLive.engine === 'realtime_a_share' ? 'realtime' : 'daily';
  const account = nextDetail.account;
  const index = accounts.value.findIndex(item => item.id === account.id);
  if (index >= 0) accounts.value[index] = account;
  if (!capitalDirty.value) capitalCash.value = scaledToDecimal(account.initial_cash);
  if (!maximumDirty.value) maximum.value = follow.value?.max_positions ?? null;
  if (managed.value && activeTab.value === 'manual') activeTab.value = 'overview';
  editAccount(account, nextDetail.targets);
}

const money = (value: number | null | undefined) => typeof value === 'number' && Number.isFinite(value) ? '¥ ' + value.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) : '—';
function followFor(id: number) { return followAccounts.value.find(view => view.account_id === id); }
function saveMaximum() {
  const view = follow.value;
  if (!view || !modelManaged.value || !validMaximum.value || maximum.value === view.max_positions) return;
  const accountId = view.account_id, maxPositions = maximum.value;
  void act(async () => {
    const updated = await invoke<FollowView>('research_follow_update', { accountId, maxPositions, enabled: null });
    followAccounts.value = followAccounts.value.map(row => row.account_id === accountId ? updated : row);
    if (selectedId.value === accountId) maximumDirty.value = false;
    await loadDetail(); message.success('数量已保存，仓位与执行规则由本模型重新匹配');
  });
}
function toggleModelTrading() {
  const view = follow.value;
  if (!view || !modelManaged.value) return;
  void act(async () => {
    const updated = await invoke<FollowView>('research_follow_update', { accountId: view.account_id, maxPositions: null, enabled: !view.enabled });
    followAccounts.value = followAccounts.value.map(row => row.account_id === view.account_id ? updated : row);
    await loadDetail(); message.success(updated.enabled ? '此账户已恢复自动买卖' : '此账户已暂停，持仓保留、未成交委托已撤销');
  });
}

async function act(task: () => Promise<void>, quiet = false) {
  if (busy.value || disposed || !props.show) return;
  const accountId = selectedId.value;
  busy.value = true; loading.value = !quiet;
  if (!quiet) error.value = '';
  try {
    await task();
    if (quiet && !disposed && props.show && selectedId.value === accountId) error.value = '';
  } catch (cause) {
    if (!disposed && props.show && (!quiet || selectedId.value === accountId)) error.value = String(cause);
  } finally {
    busy.value = false; loading.value = false;
    if (needsDetailRefresh) {
      needsDetailRefresh = false;
      if (props.show && selectedId.value && detail.value?.account.id !== selectedId.value) void act(() => loadDetail());
    }
  }
}

async function refreshAccounts() {
  if (busy.value) return;
  explicitRefresh.value = true;
  try { await act(() => loadAccounts()); } finally { explicitRefresh.value = false; }
}

function saveAccount() {
  void act(async () => {
    initialCashCny(form.initialCash);
    const account = await invoke<Account>('simulation_save_account', { executionMode:executionMode.value,input: {
      id: form.id, name: form.name.trim(), initial_cash: decimalToScaled(form.initialCash), mode: form.mode,
      auto_enabled: form.autoEnabled, manual_source_enabled: form.manualSourceEnabled,
      rule_source_enabled: form.ruleSourceEnabled, ai_source_enabled: false,
      commission_bps: Math.round(form.commissionBps), min_commission: decimalToScaled(form.minCommission),
      stamp_tax_bps: Math.round(form.stampTaxBps), transfer_fee_bps: 0,
      slippage_bps: 0, targets: parseTargets(),
    } });
    await loadAccounts(account.id); message.success('模拟账户已保存');
  });
}

function adjustCapital() {
  if (!selectedId.value || selected.value?.managed_by==='research') return;
  const accountId=selectedId.value;
  void act(async () => {
    await invoke<Account>('simulation_adjust_capital', { input: { accountId, initialCash: initialCashCny(capitalCash.value) } });
    capitalDirty.value=false;
    await loadAccounts(selectedId.value ?? undefined);
    message.success('资金已按差额调整，已有持仓和交易记录保留');
  });
}

function runNow() {
  if (!selectedId.value) return;
  void act(async () => { await invoke('simulation_run', { accountId: selectedId.value }); await loadDetail(); message.success('本次模拟运行完成'); });
}

function submitOrder() {
  if (!selectedId.value) return;
  void act(async () => {
    const key = `manual:${selectedId.value}:${Date.now()}:${order.symbol}:${order.side}`;
    await invoke('simulation_submit_order', { input: {
      account_id: selectedId.value, idempotency_key: key, symbol: order.symbol.trim().toLowerCase(),
      name: order.name.trim() || order.symbol.trim(), side: order.side, quantity: order.quantity,
      signal_date: order.signalDate, source: 'manual', rule: null, stop_bps: 0, take_bps: 0,
      limit_bps: order.limitBps, max_hold_days: 0,
    }, limitPrice:executionMode.value==='realtime'?decimalToScaled(order.limitPrice):null,stopPrice:executionMode.value==='realtime'&&order.side==='buy'?decimalToScaled(order.stopPrice):null,takePrice:executionMode.value==='realtime'&&order.side==='buy'?decimalToScaled(order.takePrice):null });
    await loadDetail(); message.success('指令已按账户模式记录');
  });
}

function confirmOrder(id: number) {
  void act(async () => { await invoke('simulation_confirm_order', { orderId: id }); await loadDetail(); });
}

function deleteAccount() {
  if (!selectedId.value || !confirm(`删除模拟账户“${selected.value?.name ?? ''}”及全部模拟账本？`)) return;
  void act(async () => { await invoke('simulation_delete_account', { accountId: selectedId.value }); selectedId.value = null; resetForm(); await loadAccounts(); });
}

let needsDetailRefresh=false;
let refreshTimer:ReturnType<typeof setInterval>|undefined;
watch(() => props.show, open => {
  if (refreshTimer) clearInterval(refreshTimer);
  ++detailRequest;
  if (open) {
    activeTab.value = 'overview';
    void act(() => loadAccounts(props.preferredAccountId));
    refreshTimer = setInterval(() => { if (!busy.value && selectedId.value && activeTab.value === 'overview') void act(() => loadDetail(), true); }, 3000);
  }
}, { immediate: true });
watch(() => props.preferredAccountId, id => { if (props.show && id && accounts.value.some(row => row.id === id)) selectedId.value = id; });
onBeforeUnmount(() => { disposed = true; ++detailRequest; if (refreshTimer) clearInterval(refreshTimer); });
watch(selectedId, id => {
  ++detailRequest; capitalDirty.value = false; maximumDirty.value = false; maximum.value = followFor(id ?? 0)?.max_positions ?? null;
  detail.value = null; live.value = null;
  if (!id) return;
  if (busy.value) { needsDetailRefresh = true; loading.value = true; } else void act(() => loadDetail());
}, { flush: 'sync' });
</script>

<template>
  <NModal v-model:show="visible">
    <NCard title="模拟账户" class="dialog" closable :bordered="false" @close="visible = false">
      <NAlert type="warning" :show-icon="false" class="notice">模拟口径独立：实时账户仅按委托后的新盘口撮合，执行 A 股 T+1；日线账户采用下一交易日开盘价回放。不会把两种结果合并。</NAlert>
      <NAlert v-if="error" type="error" class="notice">{{ error }}</NAlert>
      <div class="toolbar">
        <NSelect :to="true" v-model:value="selectedId" :options="accountOptions" placeholder="选择模拟账户" style="width: 240px" />
        <NButton :disabled="busy" @click="beginNewAccount">新建手动账户</NButton>
        <NButton v-if="!managed" :disabled="!selectedId" :loading="loading" @click="runNow">立即运行</NButton>
        <NButton class="refresh-button simulation-refresh" size="small" tertiary :disabled="busy" :loading="explicitRefresh" @click="refreshAccounts">刷新</NButton>
      </div>

      <p v-if="loading && selectedId && !detail" class="hint" role="status" data-simulation-detail-loading>正在加载账户 #{{selectedId}} 的明细…</p>
      <p class="hint">自动模型由后台筛选并建立专属账户；在这里统一管理资金、持股数量及账本。新建手动账户用于独立手动指令或通用规则测试。</p>
      <NAlert v-if="modelManaged" type="info" :show-icon="false" class="notice" data-account-ownership="model_follow">自动模型专用账户：绑定原模型独立买卖。手动指令、手动观察和其他策略不能使用此账户；资金与持股配置在下方“账户配置”管理。</NAlert>
      <NAlert v-if="modelError" type="warning" :show-icon="false" class="notice">{{modelError}}</NAlert>
      <nav v-if="accounts.some(row=>row.managed_by==='model_follow')" class="simulation-account-cards" aria-label="自动账户快速切换">
        <button v-for="row in accounts.filter(item=>item.managed_by==='model_follow')" :key="row.id" type="button" :data-account-id="row.id" :class="{selected:selectedId===row.id}" :aria-pressed="selectedId===row.id" @click="selectedId=row.id">
          <span><b>{{followFor(row.id)?.model_name || row.name}}</b><small>#{{row.id}} · {{followFor(row.id)?followFor(row.id)?.enabled?followFor(row.id)?.effective_enabled===false?'全局已暂停':'自动运行':'买卖已暂停':'状态读取中'}}</small></span>
          <span>净值 {{money(followFor(row.id)?.equity_cny)}}<small>现金 ¥ {{scaledToDecimal(row.current_cash)}} · 持仓 {{followFor(row.id)?.positions.length ?? '—'}} 只</small></span>
          <small>{{followFor(row.id)?followFor(row.id)?.positions.length?followFor(row.id)?.positions.slice(0,3).map(p=>(p.name||p.symbol)+' '+p.quantity+'股').join(' · '):'当前空仓':'持仓状态待读取'}}</small>
        </button>
      </nav>
      <details class="automatic-capital-preset" :open="presetExpanded" @toggle="presetExpanded=($event.target as HTMLDetailsElement).open"><summary>自动账户预设初始资金 <b>{{preset?money(preset.initial_cash_cny):'读取中…'}}</b><span>仅用于以后新建的自动账户</span></summary><form @submit.prevent="savePreset"><label>每个新自动账户初始金额（元）<input v-model="presetCash" @input="presetDirty=true" data-auto-preset-input type="text" inputmode="decimal" :disabled="busy || !preset" /></label><button data-auto-preset-save type="submit" :disabled="busy || !preset || !presetDirty">保存预设</button><p>每个模型有独立账户；新账户沿用此金额，持股数量与执行策略仍按各模型推荐。现有账户金额请在“账户配置”中调整，并保留调整记录。</p></form><p v-if="presetError" class="preset-error" role="alert">{{presetError}}</p></details>
      <NTabs v-model:value="activeTab" type="line" animated>
        <NTabPane name="overview" tab="概览与账本">
          <div v-if="modelManaged && follow" class="model-overview metrics">
            <div><small>账户净值</small><b>{{money(follow.equity_cny)}}</b></div><div><small>可用现金</small><b>{{money(follow.available_cash_cny ?? follow.cash_cny)}}</b></div><div><small>累计收益</small><b>{{follow.net_return_pct==null?'—':follow.net_return_pct.toFixed(2)+'%'}}</b></div><div><small>持仓 / 上限</small><b>{{follow.positions.length}} / {{follow.max_positions}} 只</b></div><div><small>买卖状态</small><b>{{follow.enabled?follow.effective_enabled===false?'全局已暂停':'自动运行':'已暂停'}}</b></div>
          </div>
          <p v-if="modelManaged && signalSchedule" class="hint simulation-signal-schedule"><b>{{signalSchedule.label}}</b> · {{signalSchedule.note}}</p>
          <details v-if="modelManaged && detail" class="model-statistics"><summary>历史执行统计（可选）</summary>
          <div class="metrics"><div><small>成交样本</small><b>{{detail.metrics.sample_count ?? 0}}</b></div><div><small>最大回撤</small><b>{{bps(detail.metrics.max_drawdown_bps)}}</b></div><div><small>扣费胜率</small><b>{{bps(detail.metrics.win_rate_bps)}}</b></div><div><small>平均持仓</small><b>{{detail.metrics.average_holding_days_x100==null?'—':(detail.metrics.average_holding_days_x100/100).toFixed(1)+' 天'}}</b></div></div>
          </details>
          <div v-if="detail && !modelManaged" class="metrics">
            <div><small>现金</small><b>¥ {{ scaledToDecimal(detail.account.current_cash) }}</b></div>
            <div><small>累计收益</small><b>{{ bps(detail.metrics.total_return_bps) }}</b></div>
            <div><small>最大回撤</small><b>{{ bps(detail.metrics.max_drawdown_bps) }}</b></div>
            <div><small>扣费胜率</small><b>{{ bps(detail.metrics.win_rate_bps) }}</b></div>
            <div><small>盈亏比</small><b>{{ detail.metrics.profit_loss_ratio_bps == null ? '-' : (detail.metrics.profit_loss_ratio_bps / 10000).toFixed(2) }}</b></div>
            <div><small>期望</small><b>¥ {{ scaledToDecimal(detail.metrics.expectancy) }}</b></div>
            <div><small>年化</small><b>{{ bps(detail.metrics.annualized_return_bps) }}</b></div>
            <div><small>基准</small><b>{{ bps(detail.metrics.benchmark_return_bps) }}</b></div>
            <div><small>平均持仓</small><b>{{ detail.metrics.average_holding_days_x100 == null ? '-' : (detail.metrics.average_holding_days_x100 / 100).toFixed(1) }} 天</b></div>
            <div><small>成交样本</small><b>{{ detail.metrics.sample_count ?? 0 }}</b></div>
          </div>
          <p v-if="detail?.performance_note" class="hint capital-note">{{detail.performance_note}}</p>
          <div class="runs"><span v-for="row in detail?.source_stats ?? []" :key="row.source"><NTag size="small">{{ row.source }}</NTag>订单 {{ row.orders }} · 成交 {{ row.filled }} · 拒绝 {{ row.rejected }} · 已实现 ¥{{ scaledToDecimal(row.realized_profit) }}</span></div>
          <h4>持仓</h4>
          <div class="table-wrap"><table><thead><tr><th>标的</th><th>持仓</th><th>可卖</th><th>含费成本</th><th v-if="modelManaged">估值 / 数据时间</th></tr></thead><tbody>
            <tr v-for="row in detail?.positions ?? []" :key="row.symbol"><td>{{ row.name || row.symbol }}<small>{{ row.symbol }}</small></td><td>{{ row.quantity }}</td><td>{{ row.available_quantity ?? '-' }}</td><td>{{ scaledToDecimal(row.cost_price) }}</td><td v-if="modelManaged">{{money(follow?.positions.find(p=>p.symbol===row.symbol)?.mark_cny)}}<small>{{follow?.positions.find(p=>p.symbol===row.symbol)?.mark_at || '暂无估值时间'}}</small></td></tr>
            <tr v-if="!detail?.positions.length"><td :colspan="modelManaged?5:4" class="empty">暂无持仓</td></tr>
          </tbody></table></div>
          <h4>最近订单</h4>
          <div class="table-wrap"><table><thead><tr><th>信号日</th><th>标的</th><th>方向/数量</th><th>状态</th><th>成交价/费用</th><th>说明</th></tr></thead><tbody>
            <tr v-for="row in detail?.orders ?? []" :key="row.id"><td>{{ row.signal_date }}</td><td>{{ row.name || row.symbol }}<small>{{ row.symbol }} · {{ row.source }}</small></td><td>{{ row.side === 'buy' ? '买' : '卖' }} {{ row.quantity }}</td><td><NTag size="small" :type="statusType(row.status)">{{ row.status }}</NTag><NButton v-if="!managed && row.status === 'awaiting_confirmation'" text type="primary" @click="confirmOrder(row.id)">确认</NButton></td><td>{{ scaledToDecimal(row.price) }} / {{ scaledToDecimal(row.fee) }}</td><td>{{ row.decision_reason || '-' }}<small v-if="row.reject_reason">{{ row.reject_reason }}</small></td></tr>
            <tr v-if="!detail?.orders.length"><td colspan="6" class="empty">暂无订单</td></tr>
          </tbody></table></div>
          <details v-if="detail?.capital_adjustments?.length" class="capital-history"><summary>资金调整记录（{{detail.capital_adjustments.length}}笔）</summary><div class="table-wrap"><table><thead><tr><th>时间</th><th>初始资金：原 → 新</th><th>增减现金</th><th>调整后现金</th></tr></thead><tbody><tr v-for="record in detail.capital_adjustments" :key="record.id"><td>{{new Date(record.created_at).toLocaleString()}}</td><td>{{scaledToDecimal(record.old_initial_cash)}} → {{scaledToDecimal(record.new_initial_cash)}}</td><td>{{scaledToDecimal(record.delta)}}</td><td>{{scaledToDecimal(record.cash_after)}}</td></tr></tbody></table></div></details>
          <h4>最近运行</h4>
          <NAlert v-if="live && !modelManaged" :type="live.engine==='realtime_a_share'?'info':'warning'" :show-icon="false">{{live.message}}<small v-if="live.updated_at"> · {{new Date(live.updated_at).toLocaleString()}}</small></NAlert>
          <div v-if="!modelManaged && live?.plans.length" class="table-wrap"><table><thead><tr><th>实时计划</th><th>买入区间</th><th>止损</th><th>止盈</th></tr></thead><tbody><tr v-for="p in live.plans" :key="p.symbol"><td>{{p.symbol}}</td><td>{{(p.buy_low/10000).toFixed(2)}}–{{(p.buy_high/10000).toFixed(2)}}</td><td>{{(p.stop/10000).toFixed(2)}}</td><td>{{(p.take/10000).toFixed(2)}}</td></tr></tbody></table></div>
          <details v-if="live?.executions.length"><summary>实时成交报价证据（最近 20 笔）</summary><pre style="max-height:240px;overflow:auto;white-space:pre-wrap">{{JSON.stringify(live.executions,null,2)}}</pre></details>
          <p class="hint">下次自动检查：{{ nextCheck }}<template v-if="!modelManaged">（实时账户约 3 秒一轮，日线账户约 5 分钟检查数据）</template></p>
          <div class="runs"><span v-for="run in detail?.recent_runs ?? []" :key="run.id"><NTag size="small" :type="statusType(run.status)">{{ run.trade_date || run.created_at }} · {{ run.phase || run.run_key || '自动' }} · {{ run.status }}<template v-if="run.status === 'running'"> {{ run.progress ?? 0 }}%</template></NTag>{{ run.message }}</span><span v-if="!detail?.recent_runs.length" class="empty"><template v-if="modelManaged">尚无成交；等待原模型信号与新盘口</template><template v-else>尚未运行；全自动默认关闭</template></span></div>
        </NTabPane>

        <NTabPane name="account" tab="账户配置">
          <p v-if="managed" class="hint">{{selected?.managed_by==='model_follow'?'此账户绑定原模型。资金、持股数量与买卖开关在这里管理，手动来源和其他策略不可接入。':'此账户用于冻结研究对照。请在研究中心查看结果；这里仅展示账本。'}}</p>
          <div v-if="modelManaged" class="capital-editor">
            <NFormItem label="初始资金（元）"><NInput data-capital-input v-model:value="capitalCash" :disabled="busy" @update:value="capitalDirty=true" /></NFormItem>
            <p class="hint">按新金额与原金额的差额增减现金；持仓、成交及未成交委托保留。减少后须留足买单资金。收益按新的初始资金基准计算，增减资金不计作盈利。</p>
            <NButton data-capital-save type="primary" :loading="loading" @click="adjustCapital">调整资金</NButton>
          </div>
          <section v-if="modelManaged && follow" class="model-account-config" aria-label="自动账户模型配置">
            <header><div><h4>{{follow.model_name}} · 独立配置</h4><p class="hint">默认沿用本模型的推荐数量、仓位与退出规则。修改数量后由后端重新匹配适用配置。</p></div><NButton :loading="loading" @click="toggleModelTrading">{{follow.enabled?'暂停此账户买卖':'恢复此账户买卖'}}</NButton></header>
            <div class="model-recommendation">当前最多 {{follow.max_positions}} 只 · 单票入场目标 {{follow.execution.position_pct}}%<template v-if="follow.execution.total_entry_pct!==undefined"> · 总入场预算 {{follow.execution.total_entry_pct}}%</template><small>{{follow.execution.entry_note || follow.execution.entry_policy}} · {{follow.execution.exit_note || follow.execution.exit_policy}}</small></div>
            <details class="model-maximum-settings"><summary>调整持股数量与查看执行规则（可选）</summary>
              <div class="maximum-editor"><NFormItem label="最大同时持股数量（1–10只）"><NInputNumber v-model:value="maximum" :min="1" :max="10" :precision="0" :disabled="busy" data-model-maximum @update:value="maximumDirty=true" /></NFormItem><NButton data-model-maximum-save type="primary" :loading="loading" :disabled="!validMaximum || maximum===follow.max_positions" @click="saveMaximum">保存数量</NButton></div>
              <p class="hint">调低数量不能少于持仓和待买委托已占的名额；保持本账户资金、持仓和模型绑定。修改配置后的收益以真实模拟账本为准。</p>
              <dl class="model-execution"><div><dt>入场</dt><dd>{{follow.execution.entry_note || follow.execution.entry_policy || '按原模型合格条件'}}</dd></div><div><dt>退出</dt><dd>{{follow.execution.exit_note || follow.execution.exit_policy || '按原模型退出规则'}} · 计划持有上限 {{follow.execution.holding_days}} 个交易日</dd></div><div><dt>仓位</dt><dd>{{follow.execution.allocation_note || '由本模型的已验证配置匹配'}}</dd></div><div><dt>执行时段</dt><dd>买入 {{follow.execution.buy_window}} · 卖出 {{follow.execution.sell_window || '按原模型有效窗口'}}</dd></div><div><dt>费用</dt><dd>{{follow.execution.fee_note}}</dd></div><div><dt>固定模型来源</dt><dd>{{follow.model_id}} · 来源 #{{follow.source_run_id}}</dd></div></dl>
              <p v-if="follow.comparison_note" class="hint">{{follow.comparison_note}}</p>
            </details>
          </section>
          <div v-if="!managed">
          <div class="form-grid">
            <NFormItem label="成交口径"><NSelect :to="true" v-model:value="executionMode" :disabled="!!form.id" :options="[{label:'实时盘口前向',value:'realtime'},{label:'日线前向（次日开盘）',value:'daily'}]" /></NFormItem><NFormItem label="账户名称"><NInput v-model:value="form.name" maxlength="30" /></NFormItem>
            <NFormItem label="初始资金（元）"><NInput v-model:value="form.initialCash" :disabled="busy" data-manual-capital-input /></NFormItem>
            <NFormItem label="运行模式（处理指令）"><NSelect :to="true" v-model:value="form.mode" :options="[{label:'仅记录',value:'record'},{label:'需确认',value:'confirm'},{label:'全自动',value:'auto'}]" /></NFormItem>
            <NFormItem label="全自动（默认关闭）"><NSwitch v-model:value="form.autoEnabled" :disabled="form.mode !== 'auto'" /></NFormItem>
            <NFormItem label="佣金（基点）"><NInputNumber v-model:value="form.commissionBps" :min="0" :max="1000" /></NFormItem>
            <NFormItem label="最低佣金（元）"><NInput v-model:value="form.minCommission" /></NFormItem>
            <NFormItem label="卖出印花税（基点）"><NInputNumber v-model:value="form.stampTaxBps" :min="0" :max="1000" /></NFormItem>

            <NFormItem label="来源开关（接收哪些指令）"><NSpace><NCheckbox v-model:checked="form.manualSourceEnabled">手动</NCheckbox><NCheckbox v-model:checked="form.ruleSourceEnabled">现有规则</NCheckbox><NCheckbox disabled>AI 候选请到“研究中心”启动验证</NCheckbox></NSpace></NFormItem>
          </div>
          <p v-if="form.id" class="hint">初始资金可以修改：按差额增减现金，保留已有持仓、成交与委托，收益改用新的初始资金基准；减少后须留足未成交买单资金。</p>
           <NFormItem label="标的代码（每行一个，最多 10 只；规则默认趋势跟随）"><NInput v-model:value="targetsText" type="textarea" :rows="4" placeholder="sh600519&#10;sz000001&#10;bj920000" /></NFormItem>
          <p class="hint">过户费与滑点均为 0，仅按本账户佣金和卖出印花税扣费。历史已成交费用保留原记录。</p>
          <div class="simulation-explanation"><b>来源开关与运行模式分别控制两件事</b><p>手动来源：接收你在“手动指令”提交的订单；现有规则：对填写的标的运行通用技术规则。勾选来源仅允许接收信号，不等于会成交。</p><p>仅记录：保存信号和计划，不成交；需确认：等待你确认后再撮合；全自动：程序接收合格信号后自行撮合。“全自动”监听开关开启后才持续检查，关闭时只能手动“立即运行”。</p><p>此页的“现有规则”是通用技术规则。要沿用已研究的五个冻结模型，后台自动模型交易会独立建账，由原模型决定买卖、股数与退出。</p></div>
          <NSpace><NButton type="primary" :loading="loading" @click="saveAccount">保存账户</NButton><NButton v-if="selectedId" type="error" secondary @click="deleteAccount">删除账户</NButton></NSpace>
          </div>
        </NTabPane>

        <NTabPane v-if="!managed" name="manual" tab="独立手动指令">
          <div class="form-grid">
            <NFormItem label="股票代码"><NInput v-model:value="order.symbol" placeholder="sh600519" /></NFormItem>
            <NFormItem label="名称"><NInput v-model:value="order.name" placeholder="可留空" /></NFormItem>
            <NFormItem label="方向"><NSelect :to="true" v-model:value="order.side" :options="[{label:'买入',value:'buy'},{label:'卖出',value:'sell'}]" /></NFormItem>
            <NFormItem label="数量"><NInputNumber v-model:value="order.quantity" :min="order.side === 'buy' ? orderLot.min : 1" :step="order.side === 'buy' ? orderLot.step : 1" :precision="0" /></NFormItem>
            <NFormItem v-if="executionMode==='realtime'" label="实时委托限价（元）"><NInput v-model:value="order.limitPrice" placeholder="例如 34" /></NFormItem><NFormItem v-if="executionMode==='realtime' &amp;&amp; order.side==='buy'" label="止盈卖点（元）"><NInput v-model:value="order.takePrice" placeholder="例如 35" /></NFormItem><NFormItem v-if="executionMode==='realtime' &amp;&amp; order.side==='buy'" label="止损价（元）"><NInput v-model:value="order.stopPrice" /></NFormItem>
            <NFormItem label="涨跌幅限制（按板块自动判定）"><NInput :value="`${(orderLimitBps / 100).toFixed(0)}%（${orderLimitBps} 基点）`" disabled /></NFormItem>
          </div>
          <p class="hint">涨跌幅与申报单位按代码所属板块自动判定：主板 ±10%（主板 ST/*ST 自 2026-07-06 起同为 ±10%）、创业板/科创板 ±20%、北交所 ±30%；科创板买入至少 200 股、北交所至少 100 股（超出部分 1 股递增），其余 100 股整手。以涨停/跌停价开盘但盘中开板（有振幅）的仍可成交，一字板才拒单。退市整理期与上市未满 5 个交易日的股票不参与模拟。</p>
          <NButton type="primary" :disabled="!selectedId || !order.symbol" :loading="loading" @click="submitOrder">提交模拟指令</NButton>
          <p class="hint">实时限价单仅使用委托建立后的新盘口；买入参考卖一、卖出参考买一，滑点为0且不突破限价。今日买入受 T+1 限制。仅记录不成交，确认模式需确认。</p>
        </NTabPane>
      </NTabs>
    </NCard>
  </NModal>
</template>

<style scoped>
.dialog{width:min(1080px,calc(100vw - 24px));max-height:calc(100vh - 24px);display:flex;flex-direction:column;overflow:hidden}.dialog :deep(.n-card__content){min-height:0;overflow:auto}.notice{margin-bottom:10px}.toolbar{display:flex;gap:8px;align-items:center;flex-wrap:wrap;position:sticky;top:0;z-index:2;background:var(--color-surface-1);padding:8px 0}.metrics{display:grid;grid-template-columns:repeat(5,minmax(110px,1fr));gap:8px;margin:10px 0}.metrics div{padding:10px;background:var(--color-surface-2);border-radius:var(--radius-sm);display:flex;flex-direction:column}.metrics small,td small{display:block;color:var(--color-text-tertiary)}.form-grid{display:grid;grid-template-columns:repeat(2,minmax(220px,1fr));gap:0 14px}.date-input{width:100%;height:34px;padding:0 10px;border:1px solid var(--color-border-0);border-radius:var(--radius-sm);background:var(--color-surface-1);color:var(--color-text-primary)}.table-wrap{overflow:auto;border:1px solid var(--color-border-0);border-radius:var(--radius-sm)}table{width:100%;border-collapse:collapse;font-size:var(--text-sm)}th,td{padding:7px 9px;text-align:left;white-space:nowrap;border-bottom:1px solid var(--color-border-0);font-variant-numeric:tabular-nums}.empty{color:var(--color-text-tertiary);text-align:center}.runs{display:flex;flex-direction:column;gap:6px}.runs span{display:flex;gap:8px;align-items:center}.hint{font-size:var(--text-xs);color:var(--color-text-tertiary)}h4{margin:14px 0 6px}@media(max-width:720px){.metrics{grid-template-columns:repeat(2,1fr)}.form-grid{grid-template-columns:1fr}}
.simulation-explanation{padding:12px;border:1px solid var(--color-border-0);border-radius:8px;font-size:12px;line-height:1.7;margin:10px 0}.simulation-explanation p{margin:6px 0}

.simulation-account-cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,220px),1fr));gap:8px;margin:12px 0}.simulation-account-cards button{display:grid;gap:8px;padding:12px;text-align:left;border:1px solid var(--color-border-0);border-radius:10px;background:var(--color-surface-1);color:var(--color-text-primary);cursor:pointer;min-width:0}.simulation-account-cards button.selected{border-color:var(--color-accent);background:var(--color-accent-dim);box-shadow:inset 0 0 0 1px var(--color-accent)}.simulation-account-cards small{display:block;margin-top:4px;color:var(--color-text-secondary);font-size:12px;overflow-wrap:anywhere}.model-statistics{margin:10px 0;font-size:12px;color:var(--color-text-secondary)}summary{cursor:pointer}.model-account-config{padding:14px;border:1px solid var(--color-border-0);border-radius:10px;margin:12px 0}.model-account-config>header{display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap}.model-account-config h4{margin:0}.model-recommendation{padding:10px;background:var(--color-surface-2);font-size:13px;border-radius:8px;margin:10px 0}.model-recommendation small{display:block;margin-top:6px;color:var(--color-text-secondary);font-size:12px;line-height:1.7}.model-maximum-settings{font-size:12px;line-height:1.7}.maximum-editor{display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin-top:12px}.maximum-editor .n-form-item{flex:1;min-width:150px}.model-execution>div{display:grid;grid-template-columns:85px 1fr;gap:10px;margin:8px 0}.model-execution dt{color:var(--color-text-secondary)}.model-execution dd{margin:0;overflow-wrap:anywhere}.capital-editor{border:1px solid var(--color-border-0);border-radius:10px;padding:14px;margin:12px 0}.capital-history{margin-top:12px;font-size:12px}.metrics{grid-template-columns:repeat(auto-fit,minmax(min(100%,140px),1fr))}.notice{font-size:12px}@media(max-width:500px){.model-execution>div{grid-template-columns:1fr;gap:2px}.maximum-editor>.n-button{width:100%}.model-account-config{padding:10px}}

.automatic-capital-preset{margin:12px 0;padding:12px 14px;border:1px solid var(--color-border-0);border-radius:10px;background:var(--color-surface-2);font-size:12px}.automatic-capital-preset summary{display:flex;align-items:center;gap:8px;flex-wrap:wrap;color:var(--color-text-primary)}.automatic-capital-preset summary span{margin-left:auto;color:var(--color-text-secondary);font-size:12px}.automatic-capital-preset form{display:flex;gap:10px;align-items:end;flex-wrap:wrap;margin-top:12px}.automatic-capital-preset label{display:grid;gap:6px;flex:1;min-width:180px}.automatic-capital-preset input{min-width:0;width:100%;box-sizing:border-box;border:1px solid var(--color-border-0);border-radius:7px;padding:8px 10px;background:var(--color-surface-1);color:var(--color-text-primary)}.automatic-capital-preset button{padding:8px 12px;border:1px solid var(--color-accent);border-radius:7px;background:var(--color-accent);color:#fff;cursor:pointer}.automatic-capital-preset button:disabled{opacity:.5;cursor:default}.automatic-capital-preset form p{flex-basis:100%;margin:0;font-size:12px;line-height:1.7;color:var(--color-text-secondary);overflow-wrap:anywhere}.automatic-capital-preset .preset-error{color:var(--color-error);overflow-wrap:anywhere}@media(max-width:480px){.automatic-capital-preset summary span{margin-left:0;flex-basis:100%}.automatic-capital-preset label{min-width:0;width:100%}.automatic-capital-preset button{width:100%}}
</style>
