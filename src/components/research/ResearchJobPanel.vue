<script setup lang="ts">
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { jobActive, jobState, MODEL_CATALOG, type ResearchJob } from '@/types/research';
const props = defineProps<{ kind?: string; selectedId?: number | null }>();
const emit = defineEmits<{ complete: [ResearchJob]; updated: [ResearchJob[]]; selectRun: [number] }>();
const jobs = ref<ResearchJob[]>([]); const error = ref(''); const acting = ref<number | null>(null);
const seen = new Set<number>(); let disposed = false; let loading = false; let timer: ReturnType<typeof setInterval> | undefined;
const visible = computed(() => jobs.value.filter(j => !props.kind || j.kind === props.kind).sort((a,b)=>b.id-a.id).slice(0, 1));
const active = computed(() => jobs.value.some(jobActive));
const kindName = (kind: string) => ({ scan: '模型筛选', replay: '历史回放', forward: '纸上账户', compare: '有限对照', explore:'技术组合研究' }[kind] ?? kind);
const modelName = (id: string) => MODEL_CATALOG.find(m => m.id === id)?.name ?? id;
const pct = (n: unknown) => typeof n === 'number' && Number.isFinite(n) ? n.toFixed(2) + '%' : '--';
async function refresh() {
  if (loading || disposed) return; loading = true;
  try {
    const rows = await invoke<ResearchJob[]>('research_job_list'); if (disposed) return;
    const first = jobs.value.length === 0;
    for (const job of rows) {
      if (job.state === 'complete' && !seen.has(job.id)) {
        seen.add(job.id); if (!first || job.id === props.selectedId) emit('complete', job);
      }
    }
    jobs.value = rows; emit('updated', rows); error.value = '';
  } catch (e) { if (!disposed) error.value = String(e); }
  finally { loading = false; }
}
async function action(job: ResearchJob, action: 'cancel' | 'resume') {
  acting.value = job.id; error.value = '';
  try { await invoke(action === 'cancel' ? 'research_job_cancel' : 'research_job_resume', { id: job.id }); await refresh(); }
  catch (e) { error.value = String(e); } finally { acting.value = null; }
}
watch(() => props.selectedId, id => { if (id) { seen.delete(id); void refresh(); } });
onMounted(() => { void refresh(); timer = setInterval(() => void refresh(), 2000); });
onBeforeUnmount(() => { disposed = true; if (timer) clearInterval(timer); });
defineExpose({ refresh, active });
</script>
<template>
  <section class="research-jobs" aria-label="研究任务进度">
    <header><b>{{ active ? '后台任务正在运行' : '任务记录' }}</b><HelpTooltip label="后台任务说明">关闭此页面后任务继续。退出应用会中断当前项，重新打开可续跑；只跳过已完成且输入版本一致的步骤。取消只停止本应用启动的研究进程。进度按真正完成的配置计数，校验前不生成结果。</HelpTooltip><button @click="refresh">刷新记录</button></header>
    <p class="job-help">只保留最新一次任务的进度、失败原因和已核验结果。新任务替换旧运行记录，账户和成交账本保留。刷新不启动计算或切换账户。</p>
    <details class="job-help"><summary>取消与续跑</summary><p>关闭页面后任务继续。取消保留已核验结果；模型筛选和前向任务中断后可继续，输入版本改变时需重新计算。暂停自动买卖请在对应账户操作。</p></details>
    <p v-if="error" class="job-error">{{ error }}</p>
    <p v-if="!visible.length" class="job-empty">尚无模型任务。自动运行或模型筛选产生记录后会显示在这里。</p>
    <article v-for="job in visible" :key="job.id" :data-job-state="job.state">
      <div class="job-title"><b>#{{ job.id }} {{ kindName(job.kind) }}</b><span class="job-state">{{ jobState(job.state) }}</span><span>{{ job.completed }}/{{ job.total }} 项已核验</span>
        <button v-if="jobActive(job)" :disabled="acting === job.id || job.cancel_requested" @click="action(job, 'cancel')">{{ job.cancel_requested ? '正在停止' : '取消' }}</button>
        <button v-else-if="['scan','forward'].includes(job.kind) && ['interrupted','cancelled','failed'].includes(job.state)" :disabled="active || acting === job.id" @click="action(job, 'resume')">继续未完成项</button>
      </div>
      <progress :value="job.completed" :max="job.total" :aria-label="'任务' + job.id + '进度'"></progress><p>{{ job.message }}</p>
      <details v-if="job.results.length"><summary>已保存 {{ job.results.length }} 项结果</summary>
        <p>这里只列出已核验并保存的项目；取消保留这些结果，打开它们不会启动新的计算或用户账户买卖。</p>
        <div v-for="result in job.results" :key="result.key" class="job-result">
          <span v-if="result.year">{{result.year}} · {{result.bank_size}}种候选 · {{result.accounts}}份核验账本</span>
          <span v-else>{{ modelName(result.model_id) }} · {{ result.comparison === 'holding15' ? '15日' : result.comparison === 'cost_double' ? '双费' : '基准' }}</span>
          <span v-if="result.metrics">收益 {{ pct(result.metrics.net_return_pct) }} / 回撤 {{ pct(result.metrics.max_drawdown_pct) }} / {{ result.metrics.completed_holding_cycles }} 笔闭合周期</span>
          <button v-if="result.run_id" @click="emit('selectRun', result.run_id)">查看账本</button>
        </div>
      </details>
    </article>
  </section>
</template>
<style scoped>
.research-jobs{border:1px solid var(--color-border-0,#d9e0e8);border-radius:12px;padding:12px;background:var(--color-surface-1,#fff);font-size:12px}.research-jobs header,.job-title{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.research-jobs header button,.job-title button{margin-left:auto}.research-jobs button{border:1px solid var(--color-border-0,#d9e0e8);background:var(--color-surface-1,#fff);color:var(--color-text-primary,#26354a);border-radius:7px;padding:5px 10px;cursor:pointer}.research-jobs button:disabled{opacity:.5;cursor:default}.research-jobs article{margin-top:10px;padding:10px;background:var(--color-surface-2,#f5f7fb);border-radius:9px}.job-state{padding:2px 7px;border-radius:12px;background:var(--color-surface-2,#e9eef6);color:var(--color-text-secondary,#4b5b73)}article[data-job-state="running"] .job-state{background:var(--color-accent-dim,#e8f1ff);color:var(--color-accent,#175bc2)}article[data-job-state="failed"] .job-state,.job-error{color:var(--color-error,#13714e)}.job-error{white-space:pre-wrap;overflow-wrap:anywhere}.job-empty,.research-jobs article p{color:var(--color-text-secondary,#69778c);margin:6px 0;line-height:1.6}.research-jobs progress{width:100%;height:5px;accent-color:var(--color-accent,#2874e8)}.job-result{display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin-top:8px}.job-result button{margin-left:auto}.job-more{margin-top:8px}.research-jobs summary{cursor:pointer;color:var(--color-text-secondary,#69778c)}.job-help{margin:8px 0;color:var(--color-text-secondary,#69778c);line-height:1.7}.job-help ol{margin:8px 0;padding-left:22px}.job-help li+li{margin-top:5px}
</style>
