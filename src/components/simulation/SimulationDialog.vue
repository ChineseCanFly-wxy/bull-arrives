<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {
  NAlert, NButton, NCard, NCheckbox, NFormItem, NInput, NInputNumber, NModal,
  NSelect, NSpace, NSwitch, NTabPane, NTabs, NTag, useMessage,
} from 'naive-ui';

type Mode = 'record' | 'confirm' | 'auto';
type Side = 'buy' | 'sell';
interface Target { symbol: string; name: string; rule: string; limit_bps: number }
interface Account {
  id: number; name: string; initial_cash: string; current_cash: string; mode: Mode;
  auto_enabled: boolean; manual_source_enabled: boolean; rule_source_enabled: boolean;
  ai_source_enabled: boolean; commission_bps: number; min_commission: string;
  stamp_tax_bps: number; transfer_fee_bps: number; slippage_bps: number;
  targets?: Target[];
}
interface Position { symbol: string; name?: string; quantity: number; available_quantity?: number; cost_price?: string }
interface Order { id: number; symbol: string; name?: string; side: Side; quantity: number; status: string; source?: string; signal_date: string; target_date?: string; price?: string; fee?: string; reject_reason?: string }
interface Run { id: number; run_key?: string; trade_date?: string; phase?: string; status: string; progress?: number; message?: string; created_at?: string }
interface Metrics { win_rate_bps?: number; profit_loss_ratio_bps?: number; expectancy?: string; max_drawdown_bps?: number; total_return_bps?: number; annualized_return_bps?: number; average_holding_days_x100?: number; sample_count?: number; benchmark_return_bps?: number | null }
interface SourceStats { source: string; orders: number; filled: number; rejected: number; realized_profit: string }
interface Detail { account: Account; targets: Target[]; positions: Position[]; orders: Order[]; recent_runs: Run[]; metrics: Metrics; source_stats: SourceStats[] }

const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();
const visible = computed({ get: () => props.show, set: value => emit('update:show', value) });
const message = useMessage();
const accounts = ref<Account[]>([]);
const selectedId = ref<number | null>(null);
const detail = ref<Detail | null>(null);
const busy = ref(false);
const error = ref('');
const targetsText = ref('');
const activeTab = ref<'overview' | 'account' | 'manual'>('overview');
const form = reactive({
  id: undefined as number | undefined,
  name: '模拟账户', initialCash: '100000.00', mode: 'record' as Mode, autoEnabled: false,
  manualSourceEnabled: true, ruleSourceEnabled: true,
  commissionBps: 3, minCommission: '5.00', stampTaxBps: 5,
  transferFeeBps: 1, slippageBps: 5,
});
const order = reactive({ symbol: '', name: '', side: 'buy' as Side, quantity: 100, signalDate: new Date().toISOString().slice(0, 10), limitBps: 1000 });

const accountOptions = computed(() => accounts.value.map(account => ({ label: account.name, value: account.id })));
const selected = computed(() => accounts.value.find(account => account.id === selectedId.value));
const nextCheck = computed(() => selected.value?.auto_enabled ? new Date(Date.now() + 5 * 60_000).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }) : '未启用');

function decimalToScaled(raw: string): string {
  const match = raw.trim().match(/^(\d+)(?:\.(\d{0,4}))?$/);
  if (!match) throw new Error('金额需为非负数字，最多 4 位小数');
  return (BigInt(match[1]) * 10000n + BigInt((match[2] ?? '').padEnd(4, '0'))).toString();
}

function scaledToDecimal(raw?: string): string {
  if (!raw || !/^\d+$/.test(raw)) return '-';
  const value = BigInt(raw); const whole = value / 10000n; const fraction = (value % 10000n).toString().padStart(4, '0').replace(/0+$/, '');
  return fraction ? `${whole}.${fraction}` : whole.toString();
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
  return unique.map(symbol => ({ symbol: symbol.toLowerCase(), name: symbol, rule: 'trend_follow', limit_bps: symbol.toLowerCase().startsWith('bj') ? 3000 : /^(sh688|sz30)/i.test(symbol) ? 2000 : 1000 }));
}

function resetForm() {
  Object.assign(form, { id: undefined, name: '模拟账户', initialCash: '100000.00', mode: 'record', autoEnabled: false, manualSourceEnabled: true, ruleSourceEnabled: true, commissionBps: 3, minCommission: '5.00', stampTaxBps: 5, transferFeeBps: 1, slippageBps: 5 });
  targetsText.value = '';
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
    transferFeeBps: account.transfer_fee_bps, slippageBps: account.slippage_bps,
  });
  targetsText.value = targets.map(target => target.symbol).join('\n');
}

async function loadAccounts(preferred?: number) {
  error.value = '';
  accounts.value = await invoke<Account[]>('simulation_list_accounts');
  selectedId.value = preferred ?? selectedId.value ?? accounts.value[0]?.id ?? null;
  if (selectedId.value) await loadDetail(); else detail.value = null;
}

async function loadDetail() {
  if (!selectedId.value) return;
  detail.value = await invoke<Detail>('simulation_get_detail', { accountId: selectedId.value });
  const account = detail.value.account;
  const index = accounts.value.findIndex(item => item.id === account.id);
  if (index >= 0) accounts.value[index] = account;
  editAccount(account, detail.value.targets);
}

async function act(task: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true; error.value = '';
  try { await task(); } catch (cause) { error.value = String(cause); } finally { busy.value = false; }
}

function saveAccount() {
  void act(async () => {
    const account = await invoke<Account>('simulation_save_account', { input: {
      id: form.id, name: form.name.trim(), initial_cash: decimalToScaled(form.initialCash), mode: form.mode,
      auto_enabled: form.autoEnabled, manual_source_enabled: form.manualSourceEnabled,
      rule_source_enabled: form.ruleSourceEnabled, ai_source_enabled: false,
      commission_bps: Math.round(form.commissionBps), min_commission: decimalToScaled(form.minCommission),
      stamp_tax_bps: Math.round(form.stampTaxBps), transfer_fee_bps: Math.round(form.transferFeeBps),
      slippage_bps: Math.round(form.slippageBps), targets: parseTargets(),
    } });
    await loadAccounts(account.id); message.success('模拟账户已保存');
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
    } });
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

watch(() => props.show, open => { if (open) void act(() => loadAccounts()); }, { immediate: true });
watch(selectedId, id => { if (id) void act(() => loadDetail()); });
</script>

<template>
  <NModal v-model:show="visible">
    <NCard title="模拟账户 · 前向验证" class="dialog" closable :bordered="false" @close="visible = false">
      <NAlert type="warning" :show-icon="false" class="notice">仅供研究学习，不连接券商、不产生真实委托。信号用前复权收盘价，成交固定用未复权下一交易日开盘价。</NAlert>
      <NAlert v-if="error" type="error" class="notice">{{ error }}</NAlert>
      <div class="toolbar">
        <NSelect v-model:value="selectedId" :options="accountOptions" placeholder="选择模拟账户" style="width: 240px" />
        <NButton :disabled="busy" @click="beginNewAccount">新建账户</NButton>
        <NButton :disabled="!selectedId" :loading="busy" @click="runNow">立即运行</NButton>
        <NButton size="small" tertiary :loading="busy" @click="loadAccounts()">刷新</NButton>
      </div>

      <NTabs v-model:value="activeTab" type="line" animated>
        <NTabPane name="overview" tab="概览与账本">
          <div v-if="detail" class="metrics">
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
          <div class="runs"><span v-for="row in detail?.source_stats ?? []" :key="row.source"><NTag size="small">{{ row.source }}</NTag>订单 {{ row.orders }} · 成交 {{ row.filled }} · 拒绝 {{ row.rejected }} · 已实现 ¥{{ scaledToDecimal(row.realized_profit) }}</span></div>
          <h4>持仓</h4>
          <div class="table-wrap"><table><thead><tr><th>标的</th><th>持仓</th><th>可卖</th><th>含费成本</th></tr></thead><tbody>
            <tr v-for="row in detail?.positions ?? []" :key="row.symbol"><td>{{ row.name || row.symbol }}<small>{{ row.symbol }}</small></td><td>{{ row.quantity }}</td><td>{{ row.available_quantity ?? '-' }}</td><td>{{ scaledToDecimal(row.cost_price) }}</td></tr>
            <tr v-if="!detail?.positions.length"><td colspan="4" class="empty">暂无持仓</td></tr>
          </tbody></table></div>
          <h4>最近订单</h4>
          <div class="table-wrap"><table><thead><tr><th>信号日</th><th>标的</th><th>方向/数量</th><th>状态</th><th>成交价/费用</th><th>说明</th></tr></thead><tbody>
            <tr v-for="row in detail?.orders ?? []" :key="row.id"><td>{{ row.signal_date }}</td><td>{{ row.name || row.symbol }}<small>{{ row.symbol }} · {{ row.source }}</small></td><td>{{ row.side === 'buy' ? '买' : '卖' }} {{ row.quantity }}</td><td><NTag size="small" :type="statusType(row.status)">{{ row.status }}</NTag><NButton v-if="row.status === 'awaiting_confirmation'" text type="primary" @click="confirmOrder(row.id)">确认</NButton></td><td>{{ scaledToDecimal(row.price) }} / {{ scaledToDecimal(row.fee) }}</td><td>{{ row.reject_reason || '-' }}</td></tr>
            <tr v-if="!detail?.orders.length"><td colspan="6" class="empty">暂无订单</td></tr>
          </tbody></table></div>
          <h4>最近运行</h4>
          <p class="hint">下次自动检查：{{ nextCheck }}（每 5 分钟检查本地历史是否出现新交易日）</p>
          <div class="runs"><span v-for="run in detail?.recent_runs ?? []" :key="run.id"><NTag size="small" :type="statusType(run.status)">{{ run.trade_date || run.created_at }} · {{ run.phase || run.run_key || '自动' }} · {{ run.status }}<template v-if="run.status === 'running'"> {{ run.progress ?? 0 }}%</template></NTag>{{ run.message }}</span><span v-if="!detail?.recent_runs.length" class="empty">尚未运行；全自动默认关闭</span></div>
        </NTabPane>

        <NTabPane name="account" tab="账户设置">
          <div class="form-grid">
            <NFormItem label="账户名称"><NInput v-model:value="form.name" maxlength="30" /></NFormItem>
            <NFormItem label="初始资金（元）"><NInput v-model:value="form.initialCash" :disabled="!!form.id" /></NFormItem>
            <NFormItem label="运行模式"><NSelect v-model:value="form.mode" :options="[{label:'仅记录',value:'record'},{label:'需确认',value:'confirm'},{label:'全自动',value:'auto'}]" /></NFormItem>
            <NFormItem label="全自动（默认关闭）"><NSwitch v-model:value="form.autoEnabled" :disabled="form.mode !== 'auto'" /></NFormItem>
            <NFormItem label="佣金（基点）"><NInputNumber v-model:value="form.commissionBps" :min="0" :max="1000" /></NFormItem>
            <NFormItem label="最低佣金（元）"><NInput v-model:value="form.minCommission" /></NFormItem>
            <NFormItem label="卖出印花税（基点）"><NInputNumber v-model:value="form.stampTaxBps" :min="0" :max="1000" /></NFormItem>
            <NFormItem label="过户费（基点）"><NInputNumber v-model:value="form.transferFeeBps" :min="0" :max="1000" :precision="0" /></NFormItem>
            <NFormItem label="滑点（基点）"><NInputNumber v-model:value="form.slippageBps" :min="0" :max="1000" /></NFormItem>
            <NFormItem label="来源开关"><NSpace><NCheckbox v-model:checked="form.manualSourceEnabled">手动</NCheckbox><NCheckbox v-model:checked="form.ruleSourceEnabled">现有规则</NCheckbox><NCheckbox disabled>AI / Agent（后续排名）</NCheckbox></NSpace></NFormItem>
          </div>
          <NFormItem label="标的代码（每行一个，最多 10 只；规则默认趋势跟随）"><NInput v-model:value="targetsText" type="textarea" :rows="4" placeholder="sh600519&#10;sz000001&#10;bj920000" /></NFormItem>
          <NSpace><NButton type="primary" :loading="busy" @click="saveAccount">保存账户</NButton><NButton v-if="selectedId" type="error" secondary @click="deleteAccount">删除账户</NButton></NSpace>
        </NTabPane>

        <NTabPane name="manual" tab="手动指令">
          <div class="form-grid">
            <NFormItem label="股票代码"><NInput v-model:value="order.symbol" placeholder="sh600519" /></NFormItem>
            <NFormItem label="名称"><NInput v-model:value="order.name" placeholder="可留空" /></NFormItem>
            <NFormItem label="方向"><NSelect v-model:value="order.side" :options="[{label:'买入',value:'buy'},{label:'卖出',value:'sell'}]" /></NFormItem>
            <NFormItem label="数量"><NInputNumber v-model:value="order.quantity" :min="100" :step="100" :precision="0" /></NFormItem>
            <NFormItem label="信号日期"><input v-model="order.signalDate" class="date-input" type="date" /></NFormItem>
            <NFormItem label="涨跌幅限制（基点）"><NInputNumber v-model:value="order.limitBps" :min="100" :max="3000" :step="100" /></NFormItem>
          </div>
          <NButton type="primary" :disabled="!selectedId || !order.symbol" :loading="busy" @click="submitOrder">提交模拟指令</NButton>
          <p class="hint">仅记录模式不会成交；确认模式需先确认；全自动模式也必须等到信号日后的未复权交易日 K 线出现才会撮合。</p>
        </NTabPane>
      </NTabs>
    </NCard>
  </NModal>
</template>

<style scoped>
.dialog{width:min(1080px,calc(100vw - 24px));max-height:calc(100vh - 24px);overflow:auto}.notice{margin-bottom:10px}.toolbar{display:flex;gap:8px;align-items:center;flex-wrap:wrap}.metrics{display:grid;grid-template-columns:repeat(5,minmax(110px,1fr));gap:8px;margin:10px 0}.metrics div{padding:10px;background:var(--color-surface-2);border-radius:var(--radius-sm);display:flex;flex-direction:column}.metrics small,td small{display:block;color:var(--color-text-tertiary)}.form-grid{display:grid;grid-template-columns:repeat(2,minmax(220px,1fr));gap:0 14px}.date-input{width:100%;height:34px;padding:0 10px;border:1px solid var(--color-border-0);border-radius:var(--radius-sm);background:var(--color-surface-1);color:var(--color-text-primary)}.table-wrap{overflow:auto;border:1px solid var(--color-border-0);border-radius:var(--radius-sm)}table{width:100%;border-collapse:collapse;font-size:var(--text-sm)}th,td{padding:7px 9px;text-align:left;white-space:nowrap;border-bottom:1px solid var(--color-border-0);font-variant-numeric:tabular-nums}.empty{color:var(--color-text-tertiary);text-align:center}.runs{display:flex;flex-direction:column;gap:6px}.runs span{display:flex;gap:8px;align-items:center}.hint{font-size:var(--text-xs);color:var(--color-text-tertiary)}h4{margin:14px 0 6px}@media(max-width:720px){.metrics{grid-template-columns:repeat(2,1fr)}.form-grid{grid-template-columns:1fr}}
</style>
