<script setup lang="ts">
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { jobActive, jobState, MODEL_CATALOG, type ResearchJob } from '@/types/research';
const props = defineProps<{ kind?: string; selectedId?: number | null }>();
const emit = defineEmits<{ complete: [ResearchJob]; updated: [ResearchJob[]]; selectRun: [number] }>();
const jobs = ref<ResearchJob[]>([]); const error = ref(''); const acting = ref<number | null>(null); const expanded = ref(false);
const seen = new Set<number>(); let disposed = false; let loading = false; let timer: ReturnType<typeof setInterval> | undefined;
const visible = computed(() => jobs.value.filter(j => !props.kind || j.kind === props.kind).slice(0, expanded.value ? 12 : 3));
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
    <p class="job-help">用于查看后台研究的真实进度、失败原因和已核验结果。模型任务按配置计数，技术组合研究按年份计数；“已完成”表示计算与核验结束，结果仍是研究观察，不会自动启用用户账户。</p>
    <details class="job-help"><summary>选择任务、刷新、取消与续跑的步骤</summary>
      <ol>
        <li>先在对应功能选择模型与对照再运行，或在“技术组合实验记录”启动五年研究。按任务号、类型和已核验数量定位记录；默认显示最近3条，点“展开更多记录”可查看最多12条最近记录，再点“收起记录”恢复简洁显示。组合历史任务在实验中的“研究记录”选择。</li>
        <li>页面打开时每2秒自动刷新；点“刷新记录”立即重新读取状态与已保存结果，不重跑研究。阶段说明看进度条下方文字，进度只在一项核验并保存后增加，不是剩余时间估计。</li>
        <li>执行中的任务点“取消”，先显示“正在停止”，等状态变成“已取消”。它只停止本应用启动的该项研究进程；已核验结果和账本保留，当前未核验项不能当作完成结果。</li>
        <li>关闭页面后任务继续，退出应用后重新打开会显示“中断待续”。中断、取消或失败的任务可点“继续未完成项”；有其他研究运行时需先等其结束。续跑跳过已保存且输入一致的项目，当前未完成项重新计算；行情或程序版本改变时会拒绝跨版本续算，应保留旧记录并新建任务。</li>
        <li>展开任务的“已保存…项结果”，已核验条目在运行中、取消后或失败后都可查看。有“查看账本”的条目会打开“冻结模型回放”页，查看净收益、回撤、资金曲线、逐笔成交与未平仓；模型筛选需完整完成后到对应候选区查看，取消后的部分筛选结果不作为最新完整候选展示；技术组合结果在“技术组合实验记录”按年查看，完整账户位置见该实验的说明。</li>
      </ol>
      <p>研究任务的“取消 / 继续未完成项”只控制这次计算，不切换用户模拟账户的买卖开关。要停止模拟模型买卖，到“自动模型交易”点对应账户的“暂停此账户买卖”，会撤销未成交委托并保留持仓；来源账本的“暂停盘后更新”控制来源更新。这些操作用途不同，查看研究结果也不会恢复已暂停的用户账户。</p>
    </details>
    <p v-if="error" class="job-error">{{ error }}</p>
    <p v-if="!visible.length" class="job-empty">尚无计算任务。先选择模型运行筛选、回放、前向来源或对照，或在技术组合实验中运行五年研究。</p>
    <article v-for="job in visible" :key="job.id" :data-job-state="job.state">
      <div class="job-title"><b>#{{ job.id }} {{ kindName(job.kind) }}</b><span class="job-state">{{ jobState(job.state) }}</span><span>{{ job.completed }}/{{ job.total }} 项已核验</span>
        <button v-if="jobActive(job)" :disabled="acting === job.id || job.cancel_requested" @click="action(job, 'cancel')">{{ job.cancel_requested ? '正在停止' : '取消' }}</button>
        <button v-else-if="['interrupted','cancelled','failed'].includes(job.state)" :disabled="active || acting === job.id" @click="action(job, 'resume')">继续未完成项</button>
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
    <button v-if="jobs.filter(j => !kind || j.kind === kind).length > 3" class="job-more" @click="expanded = !expanded">{{ expanded ? '收起记录' : '展开更多记录' }}</button>
  </section>
</template>
<style scoped>
.research-jobs{border:1px solid var(--color-border-0,#d9e0e8);border-radius:12px;padding:12px;background:var(--color-surface-1,#fff);font-size:12px}.research-jobs header,.job-title{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.research-jobs header button,.job-title button{margin-left:auto}.research-jobs button{border:1px solid var(--color-border-0,#d9e0e8);background:var(--color-surface-1,#fff);color:var(--color-text-primary,#26354a);border-radius:7px;padding:5px 10px;cursor:pointer}.research-jobs button:disabled{opacity:.5;cursor:default}.research-jobs article{margin-top:10px;padding:10px;background:var(--color-surface-2,#f5f7fb);border-radius:9px}.job-state{padding:2px 7px;border-radius:12px;background:var(--color-surface-2,#e9eef6);color:var(--color-text-secondary,#4b5b73)}article[data-job-state="running"] .job-state{background:var(--color-accent-dim,#e8f1ff);color:var(--color-accent,#175bc2)}article[data-job-state="failed"] .job-state,.job-error{color:var(--color-error,#13714e)}.job-error{white-space:pre-wrap;overflow-wrap:anywhere}.job-empty,.research-jobs article p{color:var(--color-text-secondary,#69778c);margin:6px 0;line-height:1.6}.research-jobs progress{width:100%;height:5px;accent-color:var(--color-accent,#2874e8)}.job-result{display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin-top:8px}.job-result button{margin-left:auto}.job-more{margin-top:8px}.research-jobs summary{cursor:pointer;color:var(--color-text-secondary,#69778c)}.job-help{margin:8px 0;color:var(--color-text-secondary,#69778c);line-height:1.7}.job-help ol{margin:8px 0;padding-left:22px}.job-help li+li{margin-top:5px}
</style>
