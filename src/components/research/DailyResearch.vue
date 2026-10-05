<script setup lang="ts">
import {computed,onMounted,onBeforeUnmount,ref} from 'vue';
import {invoke} from '@tauri-apps/api/core';
import {NModal} from 'naive-ui';
import ResearchModelScreener from '@/components/screener/ResearchModelScreener.vue';
import ModelConditionWatches from './ModelConditionWatches.vue';
import ModelFollowTrading from './ModelFollowTrading.vue';
import ResearchMarketData from './ResearchMarketData.vue';
import AnalysisDialog from '@/components/analysis/AnalysisDialog.vue';
import StockDetail from '@/components/detail/StockDetail.vue';
import {MODEL_CATALOG,type ResearchJob} from '@/types/research';
import type {WatchItem} from '@/types';
const props=defineProps<{page:'daily'|'tracking';busy:boolean}>();
const emit=defineEmits<{job:[ResearchJob];records:[id?:number];evidence:[];'open-settings':[string]}>();
interface PaperRun {id:number;model_id:string;model_name:string;mode:string;comparison:string;holding_days:number;as_of:string;enabled:boolean;signal_watch:Array<{symbol:string;score:number;threshold:number;cash_reference_quantity:number;reason:string}>;ledger:{metrics:{net_return_pct:number;max_drawdown_pct:number;open_positions:number};orders:Array<{date:number;code:string;side:string;qty:number;price:number;reason:string}>;unclosed:Array<{code:string;qty:number;bonus_locked:number;mark:number;last_quote_date?:number;overdue?:number}>}}
const runs=ref<PaperRun[]>([]);const selected=ref<number|null>(null);const model=ref('breadth22_h20');const error=ref('');const notice=ref('');const acting=ref(false);const analysis=ref<{symbol:string;name:string}|null>(null);const detail=ref<WatchItem|null>(null);let timer:ReturnType<typeof setInterval>|undefined;let disposed=false;let readTask:Promise<void>|null=null;
const accounts=computed(()=>runs.value.filter(r=>r.mode==='forward'));
const modelSources=computed(()=>accounts.value.filter(r=>MODEL_CATALOG.some(m=>m.id===r.model_id)&&r.comparison==='baseline'&&r.holding_days===20));
async function refresh(preferredId?:number){
  if(readTask){await readTask;if(preferredId&&accounts.value.some(r=>r.id===preferredId))selected.value=preferredId;return;}
  readTask=(async()=>{try{const v=await invoke<{runs:PaperRun[]}>('research_model_runs');if(!disposed){runs.value=v.runs;if(preferredId&&accounts.value.some(r=>r.id===preferredId))selected.value=preferredId;else if(!accounts.value.some(r=>r.id===selected.value))selected.value=accounts.value[0]?.id??null;error.value='';}}catch(e){if(!disposed)error.value=String(e);}})();
  try{await readTask;}finally{readTask=null;}
}
async function prepareSource(modelId:string){model.value=modelId;await start();}
async function start(){if(acting.value||props.busy)return;acting.value=true;error.value='';try{await refresh();const old=modelSources.value.find(r=>r.model_id===model.value);if(old){selected.value=old.id;if(!old.enabled)await invoke('research_model_observe',{id:old.id,enabled:true});await refresh();notice.value='已复用该模型来源账户并开启盘后更新。上方会自动选择可用来源，可直接创建交易账户。';}else{const job=await invoke<ResearchJob>('research_job_start',{request:{kind:'forward',models:[model.value],comparisons:['baseline'],continuation:null,enable_observation:true}});emit('job',job);notice.value='正在从最近完成日准备模型来源，完成后会自动出现在上方来源选择中；可以查看下方任务进度。';}}catch(e){error.value=String(e);}finally{acting.value=false;}}
function showDetail(row:{symbol:string;name:string}){detail.value={id:-1,code:row.symbol,name:row.name,market:'CN',sort_order:0,added_at:''};}
onMounted(()=>{if(props.page==='tracking'){void refresh();timer=setInterval(()=>void refresh(),10000);}});onBeforeUnmount(()=>{disposed=true;if(timer)clearInterval(timer);});
defineExpose({refresh});
</script>
<template>
<section class="daily-research" :class="page==='daily'?'manual-reminder-page':'automatic-trading-page'">
  <p v-if="error" class="daily-error" role="alert">{{error}}</p>
  <p v-if="notice" class="daily-notice" role="status">{{notice}}</p>
  <template v-if="page==='tracking'">
    <ModelFollowTrading :source-runs="runs" :active="true" :source-busy="busy||acting" @prepare-source="prepareSource" @refresh-sources="refresh()" />
    <ResearchMarketData />
  </template>
  <template v-else>
    <header class="manual-reminder-intro" aria-label="手动条件提醒说明">
      <span class="manual-reminder-badge">仅提醒</span>
      <div><h2>手动选股 · 条件提醒</h2><p>手动点击“观察”，只把股票加入提醒清单。无需交易账户，也不会自动买入或卖出。</p></div>
      <p class="manual-reminder-isolation">自动模型交易在独立页面运行，使用专属自动账户。这里的观察、暂停提醒或筛选都不会改动它的候选池、仓位和买卖。</p>
    </header>
    <ModelConditionWatches />
    <details class="manual-screen-help"><summary>怎样添加提醒与检查数据（可选）</summary><p>在下方选择已验证模型并筛选，从候选结果点击“观察”，保存默认条件即可。观察到的是信号和条件变化；需要自动买卖，请到“自动模型交易”主动启用模型一次。</p><p>日线信号使用最近完成交易日的数据；模型各自推荐的条件已预选，专业选项可按需调整。</p><button type="button" @click="emit('open-settings','market')">检查数据源</button></details>
    <ResearchModelScreener centralized :research-busy="busy" @job="emit('job',$event)" @records="emit('records',$event)" @evidence="emit('evidence')" @analyze="analysis=$event" @detail="showDetail" />
  </template>
  <AnalysisDialog v-if="analysis" :show="true" :symbol="analysis.symbol" :name="analysis.name" rule="auto" @update:show="v=>{if(!v)analysis=null}" />
  <NModal :show="!!detail" preset="card" :title="detail?.name||'行情详情'" :style="{width:'min(1120px,94vw)'}" @update:show="v=>{if(!v)detail=null}"><StockDetail v-if="detail" :item="detail" @close="detail=null" /></NModal>
</section>
</template>
<style scoped>
.daily-research{min-width:0;display:grid;gap:12px}.daily-error{color:var(--color-error);white-space:pre-wrap;overflow-wrap:anywhere}.daily-notice{padding:10px;border-radius:8px;background:var(--color-surface-2);font-size:12px;line-height:1.7;overflow-wrap:anywhere}.manual-reminder-intro{display:flex;align-items:center;gap:12px;flex-wrap:wrap;padding:16px;border:1px solid var(--color-border-0);border-left:4px solid var(--color-warning);border-radius:12px;background:var(--color-surface-1)}.manual-reminder-intro>div{flex:1;min-width:min(100%,220px)}.manual-reminder-intro h2{margin:0;font-size:16px;color:var(--color-text-primary)}.manual-reminder-intro p,.manual-screen-help p{margin:6px 0 0;font-size:12px;line-height:1.8;color:var(--color-text-secondary);overflow-wrap:anywhere}.manual-reminder-badge{padding:4px 10px;border-radius:7px;background:var(--color-warning-bg);color:var(--color-warning);font-size:12px;font-weight:600}.manual-reminder-isolation{flex-basis:100%}.manual-screen-help{padding:12px;border:1px solid var(--color-border-0);border-radius:10px;font-size:12px}.manual-screen-help summary{cursor:pointer;color:var(--color-text-primary)}.manual-screen-help button{margin-top:10px;border:1px solid var(--color-border-0);border-radius:8px;padding:7px 10px;color:var(--color-text-primary);background:var(--color-surface-1);cursor:pointer}
</style>
