<script setup lang="ts">
import ConditionTreeEditor from '@/components/research/ConditionTreeEditor.vue';
import {defaultConditionTree,describeCondition,usesRetiredMinutes,type ConditionTree} from '@/types/conditions';
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NModal } from 'naive-ui';
import ResearchEvidence from '@/components/research/ResearchEvidence.vue';
import ResearchJobPanel from '@/components/research/ResearchJobPanel.vue';
import ModelConditionWatches from '@/components/research/ModelConditionWatches.vue';
import { useWatchlistStore } from '@/stores/watchlist';
import { MODEL_CATALOG, modelScore, jobActive, type CandidateView, type ModelCandidate, type ResearchJob, type ConditionWatchView } from '@/types/research';
const props = defineProps<{ centralized?: boolean; researchBusy?: boolean }>();
const emit = defineEmits<{ analyze: [{ symbol: string; name: string }]; detail: [{ symbol: string; name: string }]; job: [ResearchJob]; records: [id?: number]; evidence: [] }>();
const watchlist = useWatchlistStore();
const choiceKey = 'research-model-choice-v1';
function savedChoice() { try { const v = localStorage.getItem(choiceKey); return v === 'all' || MODEL_CATALOG.some(m => m.id === v) ? v! : 'all'; } catch { return 'all'; } }
const choice = ref(savedChoice()); const resultModel = ref(''); const record = ref<CandidateView | null>(null);
const busy = ref(false); const active = ref(false); const error = ref(''); const notice = ref(''); const activeId = ref<number | null>(null);
watch(() => props.researchBusy, value => { if (props.centralized) active.value = !!value; }, { immediate: true });
const showEvidence = ref(false); const showJobs = ref(false); const showWatches = ref(false); const page = ref(1); const pageSize = 25;
const nameAttempted = new Set<string>();
const labels = ref<Record<string, { name: string; excluded: boolean }>>({});
const adding = ref(''); const jobsPanel = ref<InstanceType<typeof ResearchJobPanel> | null>(null); const watchPanel = ref<InstanceType<typeof ModelConditionWatches> | null>(null);
const observeTarget = ref<ModelCandidate | null>(null); const observePreset = ref('model_hit'); const autoUpdate = ref(true); const observing = ref(false);
const conditionTree=ref<ConditionTree>(defaultConditionTree());
const observeAdvanced = ref(false);
const observationKey = 'research-observation-preferences-v1';
const presets = ref<ConditionWatchView['presets']>([]);
const selectedPreset = computed(() => presets.value.find(p => p.id === observePreset.value));
function rememberedObservation(modelId:string):{preset:string;autoUpdate:boolean;conditionTree?:ConditionTree}|null {
  try { const v=JSON.parse(localStorage.getItem(observationKey)||'{}')[modelId];return v&&typeof v.preset==='string'&&typeof v.autoUpdate==='boolean'?v:null; } catch { return null; }
}
function recommendedObservation(){observePreset.value='model_hit';autoUpdate.value=true;conditionTree.value=defaultConditionTree();}
function rememberObservation(modelId:string){try{const all=JSON.parse(localStorage.getItem(observationKey)||'{}');all[modelId]={preset:observePreset.value,autoUpdate:autoUpdate.value,...(observePreset.value==='custom'?{conditionTree:conditionTree.value}:{})};localStorage.setItem(observationKey,JSON.stringify(all));}catch{/* Storage unavailable does not prevent a real watch from being saved. */}}
 let disposed = false; let readVersion = 0; let labelVersion = 0;
const groups = computed(() => record.value?.fresh && record.value.as_of===record.value.current_as_of ? record.value.groups.filter(g => g.as_of===record.value?.current_as_of && g.production_admission===false && MODEL_CATALOG.some(m=>m.id===g.model_id) && Number.isFinite(g.threshold) && (choice.value === 'all' || g.model_id === choice.value)) : []);
const group = computed(() => groups.value.find(g => g.model_id === resultModel.value) ?? groups.value[0]);
const candidates = computed(() => (group.value?.candidates ?? []).filter(c => c.signal_eligible === true && Number.isFinite(c.score) && c.score>(group.value?.threshold ?? Infinity) && c.as_of === record.value?.current_as_of && /^(sh6|sz[03])\d{5}$/.test(c.symbol) && !labels.value[c.symbol]?.excluded).slice().sort((a,b) => b.score-a.score || a.symbol.localeCompare(b.symbol)));
const pages = computed(() => Math.max(1, Math.ceil(candidates.value.length / pageSize)));
const visible = computed(() => candidates.value.slice((page.value-1)*pageSize, page.value*pageSize));
const modelName = (id: string) => MODEL_CATALOG.find(m => m.id === id)?.name ?? id;
const stockName = (candidate: ModelCandidate) => labels.value[candidate.symbol]?.name || candidate.symbol;
const isAdded = (symbol: string) => watchlist.items.some(w => w.market === 'CN' && (w.code === symbol || w.code === symbol.slice(2)));
watch(choice, value => { try { localStorage.setItem(choiceKey, value); } catch {} page.value=1; });
watch(() => group.value?.model_id, () => { page.value=1; });
watch(visible, () => void loadNames(), { flush: 'post' });
async function loadNames() {
  const symbols=visible.value.map(c=>c.symbol).filter(s=>!labels.value[s]&&!nameAttempted.has(s)); if (!symbols.length) return; symbols.forEach(s=>nameAttempted.add(s));
  ++labelVersion;const jobId=record.value?.job_id;
  try { const result=await invoke<typeof labels.value>('get_model_candidate_labels',{symbols}); if (!disposed && jobId===record.value?.job_id) labels.value={...labels.value,...result}; } catch { /* Score/date evidence remains usable when auxiliary labels are unavailable. */ }
}
async function load() {
  const version=++readVersion;
  try {
    const view=await invoke<CandidateView>('get_model_candidates');
    if(view.schema!=='research-model-candidates-v1'||view.production_admission!==false||!Array.isArray(view.groups))throw Error('模型结果格式或身份不匹配');
    if(!disposed&&version===readVersion){if(record.value?.job_id!==view.job_id){labels.value={};nameAttempted.clear();}record.value=view; if(!groups.value.some(g=>g.model_id===resultModel.value))resultModel.value=groups.value[0]?.model_id??''; page.value=1;}
  } catch(e){if(!disposed&&version===readVersion)error.value=String(e);}
}
async function query() {
  if(busy.value||active.value)return; busy.value=true;error.value='';notice.value='';
  try {
    const job=await invoke<ResearchJob>('research_job_start',{request:{kind:'scan',models:choice.value==='all'?MODEL_CATALOG.map(m=>m.id):[choice.value],comparisons:['baseline'],continuation:null}});
    activeId.value=job.id;active.value=true;if(props.centralized)emit('job',job);else{showJobs.value=true;await jobsPanel.value?.refresh();}
    notice.value='后台更新完成行情后推断模型；可以关闭此页面，重新打开查看进度。';
  }catch(e){error.value=String(e);}finally{busy.value=false;}
}
async function completed(job: ResearchJob) { if(job.kind==='scan'){await load();notice.value='筛选已完成；各模型按自己的分数排序，可直接查看理由或加入观察。';} }
function updateJobs(rows:ResearchJob[]){active.value=rows.some(jobActive);if(!activeId.value){activeId.value=rows.find(j=>j.kind==='scan'&&jobActive(j))?.id??null;}}
async function add(candidate:ModelCandidate){adding.value=candidate.symbol;try{await watchlist.addStock(candidate.symbol,'CN',stockName(candidate));notice.value='已加入自选。';}catch(e){error.value=String(e);}finally{adding.value='';}}
async function openObserve(candidate:ModelCandidate){
  if(!group.value)return;const modelId=group.value.model_id;error.value='';
  try{
    const value=await invoke<ConditionWatchView>('model_condition_watches');
    if(disposed||group.value?.model_id!==modelId)return;
    presets.value=value.presets;recommendedObservation();
    const saved=rememberedObservation(modelId);
    const existing=value.watches.filter(w=>w.config.model_id===modelId&&w.config.symbol===candidate.symbol).sort((a,b)=>b.id-a.id)[0];
    const preset=existing?.config.preset??saved?.preset??'model_hit';
    if(preset==='custom'||value.presets.some(p=>p.id===preset))observePreset.value=preset;
    if(saved)autoUpdate.value=saved.autoUpdate;
    conditionTree.value=JSON.parse(JSON.stringify(existing?.config.condition_tree??saved?.conditionTree??defaultConditionTree()));
    if(usesRetiredMinutes(conditionTree.value)){observePreset.value='model_hit';conditionTree.value=defaultConditionTree();notice.value='旧分钟确认条件已停用；新提醒默认使用原模型命中，原提醒记录仍保留。';}
    if(!value.presets.some(p=>p.id===observePreset.value)&&observePreset.value!=='custom')throw Error('原模型观察条件暂不可用，请刷新后重试。');
    observeAdvanced.value=observePreset.value==='custom';observeTarget.value=candidate;
  }catch(e){error.value=String(e);}
}
async function observe(){
  if(!observeTarget.value||!group.value||!record.value?.job_id)return;observing.value=true;error.value='';
  try{
    const id=await invoke<number>('model_condition_watch',{request:{job_id:record.value.job_id,model_id:group.value.model_id,symbol:observeTarget.value.symbol,preset:observePreset.value,...(observePreset.value==='custom'?{condition_tree:conditionTree.value}:{})}});
    rememberObservation(group.value.model_id);observeTarget.value=null;showWatches.value=true;notice.value='已加入条件观察，当前命中静默登记；新确认或失效时才提醒。';
    await watchPanel.value?.refresh();
    if(autoUpdate.value){
      try{const value=await invoke<{job_id:number|null;message:string}>('model_condition_auto_update',{watchId:id});notice.value+=' '+value.message;if(value.job_id){activeId.value=value.job_id;active.value=true;if(props.centralized)emit('records',value.job_id);else{showJobs.value=true;await jobsPanel.value?.refresh();}}}
      catch(e){notice.value+=' 自动更新尚未开启：'+String(e)+'。观察已保存，可在研究中心开启原模型自动观察。';}
    }
  }catch(e){error.value=String(e);}finally{observing.value=false;}
}
onMounted(()=>void load());
onBeforeUnmount(()=>{disposed=true;++readVersion;++labelVersion;});
</script>
<template>
  <section class="research-model-screen">
    <div class="screen-query">
      <div class="screen-title"><b>模型条件选股</b><HelpTooltip label="模型选股使用说明">选一个模型或全部模型，再点击筛选。使用冻结的模型与阈值计算最近完成日沪深普通股票，排除历史ST和北交所。五项是已注册研究模型，均未实盘准入；分数单位不同，不合并成统一胜率。只读历史记录不会启动计算，切换模型也不会查询。</HelpTooltip><span>先筛选，再分析或观察</span></div>
      <div class="query-actions"><select v-model="choice" aria-label="选择筛选模型"><option value="all">全部已注册模型（5）</option><option v-for="model in MODEL_CATALOG" :key="model.id" :value="model.id">{{ model.name }}</option></select><button class="primary" :disabled="busy || active" @click="query">{{ busy ? '提交中…' : active ? '后台计算中…' : '筛选符合条件的股票' }}</button><button @click="centralized?emit('evidence'):showEvidence=true">多年验证证据</button></div>
      <div class="screen-tools"><button :class="{selected:!centralized&&showJobs}" @click="centralized?emit('records',activeId??undefined):showJobs=!showJobs">{{centralized?'查看研究任务':'任务进度'}}</button><button v-if="!centralized" :class="{selected:showWatches}" @click="showWatches=!showWatches">条件观察管理</button><span>最近完成日 {{ record?.current_as_of || '--' }}</span></div>
    </div>
    <p v-if="error" class="screen-error" role="alert">{{error}}</p><p v-if="notice" class="screen-notice" role="status">{{notice}}</p>
    <div v-if="!centralized" v-show="showJobs" class="screen-section"><ResearchJobPanel ref="jobsPanel" :selected-id="activeId" @complete="completed" @updated="updateJobs" /></div>
    <div v-if="!centralized&&showWatches" class="screen-section"><ModelConditionWatches ref="watchPanel" /></div>
    <p v-if="record?.fresh && record.data_status" class="screen-note">行情来源：{{record.data_status.message}}</p>
    <p class="screen-note">{{record?.message || '选择模型后点击筛选。进入页面只恢复已保存的结果与任务，不自动计算。'}}</p>
    <p v-if="record && !record.fresh && record.as_of" class="stale-result">上次筛选 {{record.as_of}}，有 {{record.historical_hit_count ?? 0}} 只历史命中。请重新筛选后查看当前候选。</p>
    <div v-if="groups.length" class="model-result-tabs" role="tablist" aria-label="筛选结果模型"><button v-for="item in groups" :key="item.model_id" role="tab" :aria-selected="group?.model_id===item.model_id" :class="{active:group?.model_id===item.model_id}" @click="resultModel=item.model_id">{{modelName(item.model_id)}} <b>{{item.hit_count}}</b></button></div>
    <section v-if="group" class="model-results">
      <div class="result-summary"><div><b>{{modelName(group.model_id)}}</b><p>{{group.as_of}} · 已评分 {{group.scored_stocks}} 只 · 原始命中 {{group.hit_count}} 只 · 阈值 &gt; {{modelScore(group.threshold,group.model_id)}}</p></div><HelpTooltip label="分数与命中理由说明">{{group.score_semantic}}。满足条件仅说明冻结日期的模型分数超过原阈值且通过资格过滤。按原分数排名；当前名称确认是ST、退市或新股时隐藏。无名称时用代码，下一开盘是否能成交、仓位和退出须另查。</HelpTooltip></div>
      <div class="result-pager" aria-label="模型候选翻页"><span>{{candidates.length}} 只可显示 · {{page}} / {{pages}} 页</span><button :disabled="page<=1" @click="page--">上一页</button><button :disabled="page>=pages" @click="page++">下一页</button></div>
      <div class="model-table-wrap"><table><thead><tr><th>排名 / 股票</th><th>模型分数</th><th>冻结收盘价</th><th>命中依据</th><th>操作</th></tr></thead><tbody><tr v-for="(candidate,index) in visible" :key="candidate.symbol"><td><span class="stock-rank">{{(page-1)*pageSize+index+1}}</span><b>{{stockName(candidate)}}</b><small>{{candidate.symbol.slice(2)}}</small></td><td class="score">{{modelScore(candidate.score,group.model_id)}}</td><td>{{typeof candidate.close==='number'?candidate.close.toFixed(2):'--'}}</td><td><span class="hit-badge">超过固定阈值</span><details><summary>查看理由</summary><p>模型原分数 {{modelScore(candidate.score,group.model_id)}} 高于固定阈值 {{modelScore(group.threshold,group.model_id)}}。</p><p v-if="candidate.technical_state">{{candidate.technical_state}}</p><p>{{candidate.reason || candidate.reasons?.join('；') || '通过冻结日资格过滤，且模型分数高于原始阈值。'}}</p></details></td><td class="result-actions"><button class="primary" @click="emit('analyze',{symbol:candidate.symbol,name:stockName(candidate)})">分析</button><button @click="emit('detail',{symbol:candidate.symbol,name:stockName(candidate)})">走势</button><button :disabled="adding===candidate.symbol || isAdded(candidate.symbol)" @click="add(candidate)">{{isAdded(candidate.symbol)?'已自选':'+自选'}}</button><button @click="openObserve(candidate)">观察</button></td></tr></tbody></table></div>
      <p v-if="!candidates.length" class="empty-model">当前模型没有满足条件且可展示的股票。保持空列表，等待下一次筛选。</p>
      <details class="screen-evidence"><summary>本次计算来源</summary><p>任务 #{{record?.job_id}} · 行情 {{group.as_of}} · 研究观察，生产准入：否</p><p>模型指纹 {{group.model_sha256}}</p><p>输入指纹 {{group.source_fingerprint}}</p></details>
    </section>
    <p v-else-if="record?.fresh" class="empty-model">上次任务未计算当前所选模型。点击筛选获取这一模型的结果。</p>
  </section>
  <NModal :show="!!observeTarget" preset="card" title="加入模型条件观察" :style="{width:'min(620px,94vw)',maxHeight:'88dvh',overflow:'auto'}" @update:show="v=>{if(!v&&!observing)observeTarget=null;}">
    <div class="condition-choice">
      <p><b>{{observeTarget ? stockName(observeTarget) : ''}}</b> · {{group ? modelName(group.model_id) : ''}}</p>
      <div class="observation-recommendation"><b>已预选：{{observePreset==='custom'?'自定义组合':selectedPreset?.name}}</b><p>{{observePreset==='model_hit'?'观察此模型原定阈值的命中／失效，保留模型自己的评分与排序规则。':selectedPreset?.description||'使用你已保存的自定义条件。'}}{{autoUpdate?' 同时开启原模型盘后自动更新。':' 盘后自动更新已关闭。'}}</p><p>直接保存即可。当前已命中会静默登记，后续变化再提醒。</p></div>
      <details class="observation-options" :open="observeAdvanced"><summary>调整观察条件（可选）</summary>
        <label v-for="preset in presets" :key="preset.id" :class="{active:observePreset===preset.id}"><input v-model="observePreset" type="radio" :value="preset.id"><div><b>{{preset.name}}{{preset.id==='model_hit'?'（原模型推荐）':''}}</b><p>{{preset.description}}</p></div></label>
        <label :class="{active:observePreset==='custom'}"><input v-model="observePreset" type="radio" value="custom"><div><b>自定义组合（进阶）</b><p>该股票仍须命中所选模型，再按分组AND/OR条件观察。</p></div></label>
        <div v-if="observePreset==='custom'" class="custom-condition-editor"><ConditionTreeEditor v-model="conditionTree"/><p class="screen-note">模型命中 且 {{describeCondition(conditionTree)}}。最多4层20项；缺数据保留未知。</p></div>
        <label class="auto-update"><input v-model="autoUpdate" type="checkbox"><div><b>同时开启此模型的盘后自动更新</b><p>复用或准备来源账户。应用保持运行且数据齐备后刷新信号；仅模拟，不下真实订单。</p></div></label>
        <p class="screen-note">自定义条件可组合主线领涨与实时涨跌幅，只用于提醒；盘中需要新鲜报价，主线需要完成扫描。手动调整按模型分别记住。</p>
        <button @click="recommendedObservation">恢复原模型推荐</button>
      </details>
      <p class="screen-note">通知统一在研究中心的“提醒与记录”设置。保存观察不会更改模型买卖规则。</p>
      <button class="primary" :disabled="observing" @click="observe">{{observing?'正在保存…':'保存并开始观察'}}</button>
    </div>
  </NModal>
  <NModal v-model:show="showEvidence" preset="card" title="多年研究证据" :style="{width:'min(1160px,94vw)',maxHeight:'88vh',overflow:'auto'}"><ResearchEvidence /></NModal>
</template>
<style scoped>
.research-model-screen{display:grid;gap:12px;min-width:0;color:var(--color-text-primary,#26354a)}.screen-query{position:sticky;top:0;z-index:4;display:grid;gap:10px;border:1px solid var(--color-border-0,#d9e0e8);border-radius:12px;padding:14px;background:var(--color-surface-1,#fff);box-shadow:0 3px 10px #15365708}.screen-title,.query-actions,.screen-tools,.result-pager{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.screen-title>b{font-size:15px}.screen-title>span,.screen-tools>span{color:var(--color-text-secondary,#69778c);font-size:12px;margin-left:auto}.research-model-screen button,.condition-choice button{padding:6px 10px;border:1px solid var(--color-border-0,#d9e0e8);border-radius:7px;background:var(--color-surface-1,#fff);color:var(--color-text-primary,#26354a);cursor:pointer;font-size:12px}.research-model-screen button:disabled,.condition-choice button:disabled{opacity:.5;cursor:default}.research-model-screen button.primary,.condition-choice button.primary{background:var(--color-accent,#256bdd);color:var(--color-accent-contrast,#fff);border-color:transparent}.query-actions select{padding:8px 10px;min-width:220px;max-width:100%;border-radius:8px;border:1px solid var(--color-border-0,#d9e0e8);background:var(--color-surface-1,#fff);color:inherit}.screen-tools button.selected{border-color:var(--color-accent,#256bdd);color:var(--color-accent,#256bdd);background:var(--color-surface-2,#f5f7fb)}.screen-note,.screen-error,.screen-notice,.stale-result,.empty-model{font-size:12px;line-height:1.8;margin:0;overflow-wrap:anywhere}.screen-note,.empty-model{color:var(--color-text-secondary,#69778c)}.screen-error{color:var(--color-error,#13714e)}.screen-notice{padding:10px 12px;background:var(--color-surface-2,#f5f7fb);border-radius:8px}.stale-result{background:var(--color-warning-bg,#fff7e5);color:var(--color-warning,#85600e);padding:10px 12px;border-radius:8px}.model-result-tabs{display:flex;gap:6px;flex-wrap:wrap}.model-result-tabs button{padding:8px 12px}.model-result-tabs button.active{background:var(--color-accent,#256bdd);border-color:transparent;color:var(--color-accent-contrast,#fff)}.model-result-tabs b{display:inline-block;border-radius:10px;background:#9ab4d92b;padding:0 5px;margin-left:5px}.model-results{background:var(--color-surface-1,#fff);border:1px solid var(--color-border-0,#d9e0e8);border-radius:12px;overflow:hidden;min-width:0}.result-summary{display:flex;align-items:center;gap:10px;padding:14px}.result-summary p{margin:5px 0 0;font-size:12px;color:var(--color-text-secondary,#69778c)}.result-pager{padding:8px 14px;border-top:1px solid var(--color-border-0,#d9e0e8);background:var(--color-surface-2,#f5f7fb);font-size:12px}.result-pager>span{margin-right:auto}.model-table-wrap{max-height:52vh;overflow:auto}.model-table-wrap table{color:var(--color-text-primary,#26354a);width:100%;border-collapse:collapse;font-size:12px;min-width:690px}.model-table-wrap th{color:var(--color-text-secondary,#69778c);position:sticky;top:0;z-index:2;background:var(--color-surface-2,#f5f7fb);text-align:left}.model-table-wrap th,.model-table-wrap td{padding:10px 12px;border-bottom:1px solid var(--color-border-0,#d9e0e8)}.model-table-wrap td:nth-child(1){min-width:160px}.model-table-wrap td small{display:block;margin:3px 0 0 23px;color:var(--color-text-secondary,#69778c)}.stock-rank{color:var(--color-text-secondary,#69778c);display:inline-block;min-width:23px}.score{font-variant-numeric:tabular-nums;color:var(--color-accent,#256bdd)}.hit-badge{font-size:var(--text-xs);color:var(--color-accent,#175bc2);background:var(--color-accent-dim,#edf3ff);padding:3px 6px;border-radius:6px}.result-actions{white-space:nowrap}.result-actions button{padding:4px 7px;margin-right:4px}.model-table-wrap details{margin-top:5px;color:var(--color-text-secondary,#69778c);max-width:330px}.model-table-wrap summary,.screen-evidence summary{cursor:pointer}.screen-evidence{font-size:var(--text-xs);color:var(--color-text-secondary,#69778c);padding:10px 14px;overflow-wrap:anywhere}.empty-model{padding:12px}.condition-choice{display:grid;gap:10px;font-size:13px}.condition-choice .observation-options>label{display:flex;gap:10px;align-items:flex-start;padding:12px;border:1px solid var(--color-border-0,#d9e0e8);border-radius:9px;cursor:pointer}.condition-choice .observation-options>label.active{border-color:var(--color-accent,#256bdd);background:var(--color-surface-2,#f5f7fb)}.condition-choice label p{margin:5px 0 0;color:var(--color-text-secondary,#69778c);font-size:12px;line-height:1.7}.condition-choice>button{justify-self:end}.condition-choice .auto-update{border-style:dashed}@media(max-width:620px){.screen-title>span{display:none}.query-actions select{min-width:0;width:100%}.query-actions>.primary{flex:1}.screen-tools>span{width:100%;margin-left:0}.model-result-tabs button{flex:1;min-width:130px}.screen-query{padding:10px}.model-table-wrap{max-height:45vh}}
.observation-recommendation{padding:12px;border:1px solid var(--color-accent);border-radius:9px;background:var(--color-accent-dim)}.observation-recommendation p{margin:7px 0 0;line-height:1.7;color:var(--color-text-secondary)}.observation-options{display:grid;gap:10px}.observation-options summary{cursor:pointer;color:var(--color-accent);padding:5px 0}.observation-options>label,.observation-options>.custom-condition-editor,.observation-options>button{margin-top:10px}
</style>
