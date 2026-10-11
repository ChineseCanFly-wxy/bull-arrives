<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { emit as emitAppEvent } from '@tauri-apps/api/event';
import { NModal } from 'naive-ui';
import DailyResearch from './DailyResearch.vue';
import ModelFollowTrading from './ModelFollowTrading.vue';
import ResearchJobPanel from './ResearchJobPanel.vue';
import ResearchMarketData from './ResearchMarketData.vue';
import { jobActive, type ResearchJob } from '@/types/research';
import type { FollowSourceRun } from '@/types/modelFollow';
import approved from '../../../src-tauri/approved-models.json';
const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [boolean]; 'open-settings': [string] }>();
interface ModelRun extends FollowSourceRun {
  as_of: string;
  ledger: { metrics: { net_return_pct: number; max_drawdown_pct: number; completed_holding_cycles: number }; orders: Array<{ date: number; code: string; side: string; qty: number; price: number; reason: string }> };
}
const tab = ref('tracking');
const tabs = [{ id: 'tracking', label: '自动模型交易' }, { id: 'daily', label: '手动条件提醒' }, { id: 'records', label: '运行记录' }];
const runs = ref<ModelRun[]>([]);
const error = ref('');
const jobsBusy = ref(false);
const jobsExpanded = ref(true);
const activeJobId = ref<number | null>(null);
const selectedId = ref<number | null>(null);
const selected = computed(() => runs.value.find(row => row.id === selectedId.value));
const followPanel = ref<InstanceType<typeof ModelFollowTrading> | null>(null);
const dailyPanel = ref<InstanceType<typeof DailyResearch> | null>(null);
async function loadModels() {
  try { const result = await invoke<{ runs: ModelRun[] }>('research_model_runs'); runs.value = result.runs; error.value = ''; }
  catch (e) { error.value = String(e); }
}
async function openSetting(section: string) {
  if (section === 'alerts' || section === 'notifications') await emitAppEvent('notification-open-settings', 'trades');
  else emit('open-settings', section);
}
function openRecords(id?: number) { tab.value = 'records'; if (id) activeJobId.value = id; }
function startedJob(job: ResearchJob) { jobsBusy.value = true; openRecords(job.id); }
async function completedJob() { await loadModels(); await followPanel.value?.refresh(); await dailyPanel.value?.refresh(); }
function jobsUpdated(jobs: ResearchJob[]) { jobsBusy.value = jobs.some(jobActive); }
async function showRun(id: number) { await loadModels(); selectedId.value = id; tab.value = 'records'; }
watch(() => props.show, show => { if (show) void loadModels(); }, { immediate: true });
const percent = (value: number) => Number.isFinite(value) ? value.toFixed(2) + '%' : '--';
</script>

<template>
  <NModal :show="show" @update:show="emit('update:show', $event)" preset="card" class="research-modal" :bordered="false"
    :style="{ width: 'min(1240px, calc(100vw - 24px))', height: 'min(860px, calc(100dvh - 32px))', display: 'flex', flexDirection: 'column', overflow: 'hidden' }"
    :content-style="{ padding: 0, minHeight: 0, display: 'flex', flexDirection: 'column' }">
    <template #header><div class="research-title"><h2>研究中心</h2><p>已确认模型 · 自动模拟交易与运行记录</p></div></template>
    <div class="research-center">
      <nav class="research-nav" aria-label="研究中心导航"><button v-for="item in tabs" :key="item.id" :class="{ active: tab === item.id }" :aria-current="tab === item.id ? 'page' : undefined" @click="tab = item.id">{{ item.label }}</button></nav>
      <div class="research-scroll" tabindex="0" aria-label="研究内容">
        <p v-if="error" class="error" role="alert">{{ error }}</p>
        <ModelFollowTrading v-if="tab === 'tracking'" ref="followPanel" :source-runs="runs" :active="show" @refresh-sources="loadModels" />
        <DailyResearch v-if="tab === 'daily'" ref="dailyPanel" page="daily" :busy="jobsBusy" @job="startedJob" @records="openRecords" @evidence="openRecords()" @open-settings="openSetting" />
        <section v-if="tab === 'records'" class="research-records">
          <h3>运行记录</h3><p>查看数据更新、模型任务及已有账本。模型研究结果随版本更新，你只需确认自动运行设置。</p>
          <ResearchMarketData />
          <details v-if="show" class="research-background-jobs" :open="jobsExpanded" @toggle="jobsExpanded = ($event.target as HTMLDetailsElement).open">
            <summary>模型任务与已核验结果{{ jobsBusy ? ' · 正在运行' : '' }}</summary>
            <ResearchJobPanel :selected-id="activeJobId" @complete="completedJob" @updated="jobsUpdated" @select-run="showRun" />
          </details>
          <details class="approved-model-results"><summary>本版纳入的模型与仓位方案</summary>
            <article v-for="model in approved.models" :key="model.name"><h4>{{ model.name }}</h4><p>{{ model.qualification }}</p><p>{{ model.result }}</p><p class="muted">{{ model.limits }}</p></article>
            <p class="muted">历史研究截至 {{ approved.as_of }}。未通过方案只保留结论和去重记录，不参与自动运行。</p>
          </details>
          <details><summary>已有信号与历史账本 · {{ runs.length }} 项</summary>
            <p class="muted">这里是信号来源及历史回放记录；实际模拟成交和账户收益在“自动模型交易”查看。</p>
            <div class="saved-runs"><button v-for="run in runs" :key="run.id" :class="{ active: selectedId === run.id }" @click="selectedId = run.id">#{{ run.id }} {{ run.model_name }} · {{ run.as_of }}</button></div>
            <section v-if="selected"><h4>{{ selected.model_name }} · {{ selected.as_of }}</h4><p>来源账本累计收益 {{ percent(selected.ledger.metrics.net_return_pct) }} · 最大回撤 {{ percent(selected.ledger.metrics.max_drawdown_pct) }} · 完成 {{ selected.ledger.metrics.completed_holding_cycles }} 轮</p>
              <div class="table-wrap"><table><thead><tr><th>日期</th><th>股票</th><th>方向</th><th>股数</th><th>价格</th><th>原因</th></tr></thead><tbody><tr v-for="(order, index) in selected.ledger.orders.slice(-30).reverse()" :key="index"><td>{{ order.date }}</td><td>{{ order.code }}</td><td>{{ order.side === 'buy' ? '买入' : '卖出' }}</td><td>{{ order.qty }}</td><td>{{ order.price }}</td><td>{{ order.reason }}</td></tr></tbody></table></div>
            </section>
          </details>
        </section>
        <button class="notice-settings" type="button" @click="openSetting('notifications')">提醒设置 · 资讯与提醒</button>
      </div>
    </div>
  </NModal>
</template>

<style scoped>
.research-title h2{margin:0;font-size:19px}.research-title p{margin:4px 0 0;font-size:12px;color:var(--color-text-secondary)}
.research-center{display:flex;flex-direction:column;min-height:0;flex:1;font-size:13px;line-height:1.7;color:var(--color-text-primary)}
.research-nav{display:flex;gap:8px;flex-wrap:wrap;padding:0 0 12px}.research-center button{border:1px solid var(--color-border-0);border-radius:8px;background:var(--color-surface-1);color:inherit;padding:7px 12px;cursor:pointer;font:inherit}.research-center button.active{border-color:var(--color-accent);background:var(--color-accent-dim);color:var(--color-accent)}
.research-scroll{min-height:0;overflow:auto;flex:1;padding-right:4px}.research-records{display:grid;gap:14px}.research-records h3,.research-records p{margin:0}.research-records details{border:1px solid var(--color-border-0);border-radius:10px;padding:12px}.research-records summary{cursor:pointer}.research-records article{padding:12px 0;border-bottom:1px solid var(--color-border-0)}.research-records article h4{margin:0 0 6px}.muted{color:var(--color-text-secondary);font-size:12px}.error{color:var(--color-error);white-space:pre-wrap;overflow-wrap:anywhere}.saved-runs{display:flex;gap:6px;flex-wrap:wrap;margin-top:10px}.table-wrap{overflow:auto}table{width:100%;border-collapse:collapse;text-align:left}th,td{padding:6px;border-bottom:1px solid var(--color-border-0);white-space:nowrap}.notice-settings{margin-top:14px}button:focus-visible,summary:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
</style>