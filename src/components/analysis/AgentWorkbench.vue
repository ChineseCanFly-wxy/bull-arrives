<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';

const props = defineProps<{ fingerprint: string; busy: boolean; installed: boolean; visible: boolean }>();
const emit = defineEmits<{ ask: [question: string] }>();
interface Prompt { id: string; label: string; content: string; hash: string }
interface Inspection { prompts: Prompt[]; input_json: string; schema_json: string; missing_data: string[]; timeout_seconds: number; budget_usd: string; executable: string | null; history_summary: string }
interface Run { id: number; role: string; round: number; task: string; status: string; started_at: string; duration_ms: number | null; prompt_hash: string; error: string | null }
interface Activity { current: { role: string; round: number; task: string; process_id: number | null; elapsed_ms: number; timeout_seconds: number } | null; runs: Run[] }
interface Detail { prompt: string; input_json: string; schema_json: string }
const inspection = ref<Inspection | null>(null);
const activity = ref<Activity>({ current: null, runs: [] });
const detail = ref<Detail | null>(null);
const detailId = ref<number | null>(null);
const selectedPrompt = ref('single_stock_analysis:');
const question = ref('');
const error = ref('');
let generation = 0;
let activityRequest = 0;
let detailRequest = 0;
let timer: ReturnType<typeof setInterval> | undefined;
const prompt = computed(() => inspection.value?.prompts.find(p => p.id === selectedPrompt.value));
const roleNames: Record<string, string> = { technical: '技术分析师', bull: '多方研究员', bear: '空方研究员', risk: '风控综合' };
const taskNames: Record<string, string> = { single_stock_analysis: '单股解读 / 快照问答', multi_role_analysis: '多角色研判', news_summary: '资讯摘要', dynamic_filter: '动态筛选建议', connection_test: '连接测试' };
const statusNames: Record<string, string> = { received: '结果文件已返回', failed: '调用失败', running: '执行中', interrupted: '已结束或被中断' };

async function refreshActivity() {
  const fingerprint = props.fingerprint;
  const request = ++activityRequest;
  try {
    const value = await invoke<Activity>('get_agent_activity', { contextFingerprint: fingerprint });
    if (fingerprint === props.fingerprint && request === activityRequest) activity.value = value;
  } catch (e) { if (fingerprint === props.fingerprint && request === activityRequest) error.value = String(e); }
}
watch(() => props.fingerprint, async fingerprint => {
  const request = ++generation;
  ++detailRequest;
  inspection.value = null; detail.value = null; detailId.value = null; question.value = ''; error.value = '';
  activity.value = { current: null, runs: [] };
  try {
    const value = await invoke<Inspection>('inspect_agent_task', { contextFingerprint: fingerprint });
    if (request === generation) inspection.value = value;
  } catch (e) { if (request === generation) error.value = String(e); }
  if (request === generation) void refreshActivity();
}, { immediate: true });
watch(() => [props.busy, props.visible] as const, ([busy, visible]) => {
  if (timer) clearInterval(timer);
  if (visible) void refreshActivity();
  if (busy && visible) timer = setInterval(() => void refreshActivity(), 1000);
}, { immediate: true });
onBeforeUnmount(() => { ++generation; ++activityRequest; ++detailRequest; if (timer) clearInterval(timer); });
async function openRecord(id: number) {
  const request = ++detailRequest;
  detailId.value = id; detail.value = null;
  try {
    const value = await invoke<Detail>('get_agent_run_detail', { runId: id });
    if (request === detailRequest) detail.value = value;
  } catch (e) { if (request === detailRequest) error.value = String(e); }
}
function ask() { if (question.value.trim() && !props.busy && props.installed) emit('ask', question.value.trim()); }
</script>

<template>
  <section class="workbench">
    <p class="intro">AI 解释本次指标、交易规则与回测，列出支持因素、风险和失效条件。多角色研判依次进行独立分析 → 多空交叉回应 → 风控综合，共最多六次调用。</p>
    <p class="note">本区域的快照问答、Agent 解读和多角色研判在后台运行，不弹出终端；受限模式不联网搜索，也不加载全局 skills / MCP。若想直接对话，请使用下方“打开 Claude Code 终端”。</p>
    <details>
      <summary>查看分析方法、提示词与本次输入</summary>
      <div v-if="inspection" class="inspection">
        <p>模型沿用 Claude Code 当前配置；应用未读取具体模型名。每次上限 {{ inspection.timeout_seconds }} 秒、应用单次预算上限 ${{ inspection.budget_usd }}；多角色最多六次独立调用，账户额度与实际费用以服务商为准。</p>
        <p class="path">执行程序：{{ inspection.executable || '未检测到' }}</p>
        <p>行情上下文：{{ inspection.history_summary }}</p>
        <p><b>本次数据局限</b></p>
        <ul><li v-for="item in inspection.missing_data" :key="item">{{ item }}</li></ul>
        <label>分析方法 <select v-model="selectedPrompt"><option v-for="item in inspection.prompts" :key="item.id" :value="item.id">{{ item.label }}</option></select></label>
        <p class="note">以下是应用实际发送的任务提示词（不含 Claude Code 自身的内置系统提示）。提示词内容有版本指纹；修改后不会误用旧缓存。</p>
        <pre>{{ prompt?.content }}</pre>
        <small class="path">提示词指纹：{{ prompt?.hash }}</small>
        <details><summary>本次冻结数据 JSON（运行前预览）</summary><pre>{{ inspection.input_json }}</pre></details>
        <details><summary>单股输出结构约束 JSON</summary><pre>{{ inspection.schema_json }}</pre></details>
        <p class="note">提问会额外携带下方问题；多角色会携带对应阶段的角色意见。实际发送内容可在调用记录中查看。</p>
      </div>
      <p v-else>正在读取快照…</p>
    </details>

    <div class="question">
      <label for="agent-question">围绕当前快照提问</label>
      <textarea id="agent-question" v-model="question" maxlength="500" rows="2" :disabled="busy" placeholder="例如：评分较高但交易规则没就绪，主要差在哪里？" />
      <div class="question-actions">
        <button :disabled="busy || !installed || !question.trim()" @click="ask">针对问题解读</button>
        <small>单次问答，最多 500 字；问题随调用记录保存在本地，仍只使用当前快照。</small>
      </div>
    </div>

    <div v-if="busy || activity.current" class="progress" role="status" aria-live="polite">
      <template v-if="activity.current">
        <b>{{ roleNames[activity.current.role] || taskNames[activity.current.task] || 'AI 任务' }}</b>
        <span v-if="activity.current.round"> · 第 {{ activity.current.round }} 阶段</span>
        <span> · 已运行 {{ (activity.current.elapsed_ms / 1000).toFixed(0) }} 秒 / 单次上限 {{ activity.current.timeout_seconds }} 秒</span>
        <small v-if="activity.current.process_id"> · 任务进程 PID {{ activity.current.process_id }}</small>
      </template>
      <span v-else>正在校验快照、查找缓存或切换角色…</span>
    </div>

    <details>
      <summary>当前快照的调用记录（{{ activity.runs.length }}）</summary>
      <p class="note">本地保留最近 100 次真实调用，此处最多展示当前快照的 20 次。缓存复用不会启动进程，也不会新增调用。文件返回不代表业务校验通过，以结果卡为准。</p>
      <p v-if="!activity.runs.length">尚无真实调用记录。升级前的调用没有过程记录。</p>
      <div class="runs">
        <button v-for="run in activity.runs" :key="run.id" class="run" :class="{ selected: detailId === run.id }" @click="openRecord(run.id)">
          <b>{{ roleNames[run.role] || taskNames[run.task] || run.task }}<template v-if="run.round"> · 阶段 {{ run.round }}</template></b>
          <span>{{ statusNames[run.status] || run.status }} · {{ run.duration_ms === null ? '—' : `${(run.duration_ms / 1000).toFixed(1)} 秒` }}</span>
          <small>{{ new Date(run.started_at).toLocaleString() }}</small><small v-if="run.error">{{ run.error }}</small>
        </button>
      </div>
      <div v-if="detail">
        <details open><summary>这次实际提示词</summary><pre>{{ detail.prompt }}</pre></details>
        <details><summary>这次实际输入（包含提问或先前角色观点）</summary><pre>{{ detail.input_json }}</pre></details>
        <details><summary>这次输出结构</summary><pre>{{ detail.schema_json }}</pre></details>
      </div>
    </details>
    <p v-if="error" class="error">{{ error }}</p>
  </section>
</template>

<style scoped>
.workbench { margin-bottom: 14px; font-size: var(--text-xs); line-height: 1.7; color: var(--color-text-secondary); }
:global([data-style="trading"]) .workbench,
:global([data-style="modern"]) .workbench { margin-bottom: 0; }
:global([data-style="trading"]) .workbench > details,
:global([data-style="modern"]) .workbench > details { background: var(--color-surface-0); }
.intro { color: var(--color-text-primary); margin-top: 0; }
.note { color: var(--color-text-tertiary); }
details { margin: 9px 0; padding: 8px 10px; border: 1px solid var(--color-border-0); border-radius: var(--radius-sm); }
summary { cursor: pointer; color: var(--color-text-primary); font-weight: 600; }
pre { max-height: 280px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; padding: 10px; background: var(--color-surface-2); font: inherit; }
.path { overflow-wrap: anywhere; }
.question { display: grid; gap: 6px; margin: 12px 0; }
:global([data-style="trading"]) .question,
:global([data-style="modern"]) .question { gap: var(--space-2); margin: var(--space-2) 0; padding: var(--space-3); border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-0); }
textarea, select { color: var(--color-text-primary); background: var(--color-surface-2); border: 1px solid var(--color-border-0); border-radius: var(--radius-sm); padding: 8px; font: inherit; }
textarea { width: 100%; box-sizing: border-box; resize: vertical; }
.question-actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
button { cursor: pointer; font: inherit; border: 1px solid var(--color-border-0); border-radius: var(--radius-sm); padding: 6px 10px; background: var(--color-surface-2); color: var(--color-text-primary); }
button:disabled { opacity: .5; cursor: not-allowed; }
.progress { padding: 10px; margin: 10px 0; background: var(--color-surface-2); border-left: 3px solid var(--color-accent); }
.runs { display: grid; gap: 6px; max-height: 250px; overflow: auto; }
.run { display: flex; gap: 6px 14px; flex-wrap: wrap; text-align: left; }
.run.selected { border-color: var(--color-accent); }
.error { color: var(--color-error); }
:global([data-style="trading"]) .error,
:global([data-style="modern"]) .error { background: var(--color-error-bg); padding: var(--space-2); border-radius: var(--radius-sm); }
</style>
