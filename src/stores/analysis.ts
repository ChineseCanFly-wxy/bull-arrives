// src/stores/analysis.ts
// 个股技术分析状态管理。

import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AgentAnalysisResponse, AgentStatus, AgentTeamResponse, ResearchReport, StockAnalysis } from '@/types/analysis';

export interface InteractiveTask {
  id: string;
  context_fingerprint: string;
  symbol: string;
  as_of: string;
  created_at: string;
  state: 'prepared' | 'opened' | 'validated';
  directory: string;
  live: boolean;
}

export interface InteractiveTaskActivity {
  task: InteractiveTask;
  output_present: boolean;
  output_bytes: number | null;
  output_modified_at: string | null;
  last_checked_at: string;
}

export interface LiveEvent {
  seq: number;
  task_id: string;
  fingerprint: string;
  kind: 'user' | 'text' | 'text_delta' | 'tool' | 'done' | 'error';
  text: string;
}

export interface LiveStatus {
  task_id: string;
  fingerprint: string;
  running: boolean;
  can_resume: boolean;
  events: LiveEvent[];
}

export const useAnalysisStore = defineStore('analysis', () => {
  let analysisRequest = 0;
  let agentRequest = 0;
  let researchRequest = 0;
  const analysis = ref<StockAnalysis | null>(null);
  const symbol = ref('');
  const loading = ref(false);
  const error = ref<string | null>(null);
  const agentStatus = ref<AgentStatus | null>(null);
  const agentAnalysis = ref<AgentAnalysisResponse | null>(null);
  const agentLoading = ref(false);
  const interactiveTasks = ref<InteractiveTask[]>([]);
  const interactiveActivities = ref<Record<string, InteractiveTaskActivity>>({});
  const interactiveBusy = ref(false);
  const interactiveError = ref<string | null>(null);
  const interactiveNotice = ref<string | null>(null);
  const liveStatus = ref<LiveStatus | null>(null);
  const liveBusy = ref(false);
  const liveError = ref<string | null>(null);
  let unlistenLive: UnlistenFn | null = null;
  const historicalInteractiveResult = ref<AgentAnalysisResponse | null>(null);
  const teamAnalysis = ref<AgentTeamResponse | null>(null);
  const teamLoading = ref(false);
  const teamError = ref<string | null>(null);
  const research = ref<ResearchReport | null>(null);
  const researchLoading = ref(false);
  const researchError = ref<string | null>(null);
  /** 本次分析最终采用的规则（自动匹配到的或用户手动指定的），用于界面回显 */
  const ruleUsed = ref<string>('trend_follow');
  /** 用户手动指定的规则；null 表示走自动匹配 */
  const ruleOverride = ref<string | null>(null);

  /**
   * 分析一只股票。
   *
   * `rule` 传具体规则 id 表示**用户手动指定**（界面上的切换按钮）；
   * 不传或传 `'auto'` 则让后端按**当前市场状态**自动匹配一条。
   *
   * 为什么默认自动：三条规则的入场条件本身就是状态判据（趋势 / 超跌 / 突破），
   * 让每只股票用它当下适用的那条，比一刀切用「策略配套的那条」更合理 ——
   * 也顺带避免了「低波动稳健被硬配成趋势跟随，可低波动股根本走不出趋势」这类错配。
   *
   * ⚠️ 自动匹配**不看历史回测收益** —— 实测那样挑的选对率只有 40%（随机 33%）。
   */
  async function analyze(sym: string, rule?: string | null) {
    const request = ++analysisRequest;
    ++agentRequest;
    if (liveStatus.value?.running) void invoke<boolean>('cancel_live_analysis', { taskId: liveStatus.value.task_id });
    liveStatus.value = null;
    liveError.value = null;
    if (agentLoading.value || teamLoading.value) void invoke<boolean>('cancel_agent_analysis');
    agentLoading.value = false;
    teamLoading.value = false;
    symbol.value = sym;
    loading.value = true;
    error.value = null;
    analysis.value = null;
    agentAnalysis.value = null;
    teamAnalysis.value = null;
    teamError.value = null;
    interactiveTasks.value = [];
    interactiveActivities.value = {};
    interactiveError.value = null;
    interactiveNotice.value = null;
    historicalInteractiveResult.value = null;
    research.value = null;
    researchError.value = null;
    const manual = rule && rule !== 'auto' ? rule : null;
    ruleOverride.value = manual;
    ruleUsed.value = manual ?? 'auto';
    try {
      const result = await invoke<StockAnalysis>('analyze_stock', {
        symbol: sym,
        rule: manual ?? 'auto',
      });
      if (request !== analysisRequest || symbol.value !== sym) return;
      analysis.value = result;
      if (result.agent_context_fingerprint) void loadInteractiveTasks(result.agent_context_fingerprint);
      // 回填后端最终采用的规则：自动匹配时前端并不知道它会挑哪条
      if (!manual && result.rule_match?.recommended) {
        ruleUsed.value = result.rule_match.recommended;
      }
    } catch (e) {
      if (request !== analysisRequest || symbol.value !== sym) return;
      analysis.value = null;
      error.value = `分析失败：${e}`;
    } finally {
      if (request === analysisRequest) loading.value = false;
    }
  }

  async function loadAgentStatus() {
    try {
      agentStatus.value = await invoke<AgentStatus>('get_agent_status');
    } catch (e) {
      agentStatus.value = { installed: false, state: 'unavailable', path: null, message: String(e), guidance: '请在设置 → 智能中重新检测或测试连接。', run_dir: null };
    }
  }

  async function listenLiveEvents() {
    if (unlistenLive) return;
    unlistenLive = await listen<LiveEvent>('agent-live-event', ({ payload }) => {
      const current = liveStatus.value;
      if (!current || payload.task_id !== current.task_id || payload.fingerprint !== current.fingerprint) return;
      if (payload.fingerprint !== analysis.value?.agent_context_fingerprint) return;
      const last = current.events.at(-1)?.seq ?? 0;
      if (payload.seq <= last) return;
      const events = [...current.events.slice(-239), payload];
      const ended = payload.kind === 'done' || payload.kind === 'error';
      liveStatus.value = { ...current, running: ended ? false : current.running, events };
      if (ended) {
        void invoke<LiveStatus>('get_live_analysis', { taskId: current.task_id }).then(result => {
          if (liveStatus.value?.task_id === result.task_id && analysis.value?.agent_context_fingerprint === result.fingerprint
            && (liveStatus.value.events.at(-1)?.seq ?? 0) <= (result.events.at(-1)?.seq ?? 0)) {
            liveStatus.value = result;
          }
        }).catch(e => { liveError.value = `更新会话状态失败：${e}`; });
        void loadInteractiveTasks(current.fingerprint);
      }
    });
  }

  async function startLiveAnalysis() {
    const fingerprint = analysis.value?.agent_context_fingerprint;
    if (!fingerprint || liveBusy.value || liveStatus.value?.running) return;
    liveBusy.value = true;
    liveError.value = null;
    try {
      await listenLiveEvents();
      const result = await invoke<LiveStatus>('start_live_analysis', { contextFingerprint: fingerprint });
      if (analysis.value?.agent_context_fingerprint === fingerprint) {
        const events = liveStatus.value?.task_id === result.task_id && (liveStatus.value.events.at(-1)?.seq ?? 0) > (result.events.at(-1)?.seq ?? 0)
          ? liveStatus.value.events : result.events;
        liveStatus.value = { ...result, events, running: events.at(-1)?.kind === 'done' || events.at(-1)?.kind === 'error' ? false : result.running };
        await loadInteractiveTasks(fingerprint);
      } else {
        void invoke('cancel_live_analysis', { taskId: result.task_id });
      }
    } catch (e) { liveError.value = `启动应用内会话失败：${e}`; }
    finally { liveBusy.value = false; }
  }

  async function askLiveAnalysis(question: string) {
    const current = liveStatus.value;
    if (!current || liveBusy.value || current.running || !question.trim()) return;
    liveBusy.value = true;
    liveError.value = null;
    try {
      const result = await invoke<LiveStatus>('ask_live_analysis', { taskId: current.task_id, question: question.trim() });
      if (liveStatus.value?.task_id === current.task_id) liveStatus.value = result;
    } catch (e) { liveError.value = `追问失败：${e}`; }
    finally { liveBusy.value = false; }
  }

  async function cancelLiveAnalysis() {
    if (!liveStatus.value?.running) return;
    await invoke<boolean>('cancel_live_analysis', { taskId: liveStatus.value.task_id });
  }

  async function restoreLiveAnalysis(taskId: string) {
    const task = interactiveTasks.value.find(item => item.id === taskId);
    if (!task || task.context_fingerprint !== analysis.value?.agent_context_fingerprint) return;
    liveError.value = null;
    try {
      await listenLiveEvents();
      const result = await invoke<LiveStatus>('get_live_analysis', { taskId });
      if (analysis.value?.agent_context_fingerprint === task.context_fingerprint) liveStatus.value = result;
    } catch (e) { liveError.value = `恢复会话状态失败：${e}`; }
  }

  async function loadInteractiveTasks(fingerprint: string) {
    const currentSymbol = symbol.value;
    try {
      const tasks = await invoke<InteractiveTask[]>('list_interactive_analyses', { symbol: currentSymbol });
      if (analysis.value?.agent_context_fingerprint === fingerprint && symbol.value === currentSymbol) {
        interactiveTasks.value = tasks;
        const ids = new Set(tasks.map(task => task.id));
        interactiveActivities.value = Object.fromEntries(
          Object.entries(interactiveActivities.value).filter(([id]) => ids.has(id)),
        );
      }
    } catch (e) {
      if (analysis.value?.agent_context_fingerprint === fingerprint && symbol.value === currentSymbol) interactiveError.value = `读取交互任务失败：${e}`;
    }
  }

  async function inspectInteractiveTask(taskId: string) {
    const task = interactiveTasks.value.find(item => item.id === taskId);
    const currentSymbol = symbol.value;
    if (!task || task.symbol !== currentSymbol) return;
    try {
      const activity = await invoke<InteractiveTaskActivity>('inspect_interactive_analysis', { taskId });
      if (symbol.value === currentSymbol && interactiveTasks.value.some(item => item.id === taskId)
        && activity.task.id === taskId && activity.task.context_fingerprint === task.context_fingerprint) {
        interactiveActivities.value = { ...interactiveActivities.value, [taskId]: activity };
      }
    } catch (e) {
      if (symbol.value === currentSymbol) interactiveError.value = `读取任务状态失败：${e}`;
    }
  }

  async function startInteractiveAnalysis() {
    const fingerprint = analysis.value?.agent_context_fingerprint;
    if (!fingerprint || interactiveBusy.value) return;
    interactiveBusy.value = true;
    interactiveError.value = null;
    interactiveNotice.value = null;
    try {
      const task = await invoke<InteractiveTask>('start_interactive_analysis', { contextFingerprint: fingerprint });
      if (analysis.value?.agent_context_fingerprint === fingerprint) {
        interactiveTasks.value = [task, ...interactiveTasks.value];
        interactiveNotice.value = '已请求打开 Claude Code 终端；请在终端交互，完成后手动导入结果。';
      }
    } catch (e) {
      if (analysis.value?.agent_context_fingerprint === fingerprint) {
        interactiveError.value = String(e);
        await loadInteractiveTasks(fingerprint);
      }
    } finally {
      interactiveBusy.value = false;
    }
  }

  async function resumeInteractiveAnalysis(taskId: string) {
    if (interactiveBusy.value) return;
    interactiveBusy.value = true;
    interactiveError.value = null;
    try {
      await invoke('resume_interactive_analysis', { taskId });
      interactiveNotice.value = '已请求打开恢复会话的终端；若 Claude Code 提示会话不存在，请新建分析任务。';
    } catch (e) {
      interactiveError.value = String(e);
    } finally {
      interactiveBusy.value = false;
    }
  }

  async function deleteInteractiveAnalysis(taskId: string, closedSession: boolean) {
    if (interactiveBusy.value) return;
    const fingerprint = analysis.value?.agent_context_fingerprint;
    interactiveBusy.value = true;
    interactiveError.value = null;
    try {
      await invoke('delete_interactive_analysis', { taskId, closedSession });
      if (fingerprint && analysis.value?.agent_context_fingerprint === fingerprint) {
        interactiveTasks.value = interactiveTasks.value.filter(task => task.id !== taskId);
        if (liveStatus.value?.task_id === taskId) liveStatus.value = null;
        interactiveNotice.value = '已清理该交互任务的本地文件。';
      }
    } catch (e) {
      interactiveError.value = `清理失败：${e}`;
    } finally {
      interactiveBusy.value = false;
    }
  }

  async function importInteractiveAnalysis(taskId: string) {
    const fingerprint = analysis.value?.agent_context_fingerprint;
    if (!fingerprint || interactiveBusy.value) return;
    const task = interactiveTasks.value.find(item => item.id === taskId);
    if (!task || task.symbol !== symbol.value) {
      interactiveError.value = '该任务不属于当前股票，不能查看结果。';
      return;
    }
    if (agentLoading.value || teamLoading.value) {
      interactiveError.value = '请先结束正在运行的后台 Agent 分析，再导入交互结果。';
      return;
    }
    interactiveBusy.value = true;
    interactiveError.value = null;
    try {
      const response = await invoke<AgentAnalysisResponse>('import_interactive_analysis', { taskId });
      if (analysis.value?.agent_context_fingerprint !== fingerprint || symbol.value !== task.symbol
        || response.context_fingerprint !== task.context_fingerprint) {
        throw new Error('交互结果与任务快照或当前股票不匹配');
      }
      if (task.context_fingerprint === fingerprint) {
        ++agentRequest;
        agentAnalysis.value = response;
        historicalInteractiveResult.value = null;
        interactiveNotice.value = '当前快照的结构化分析结果已通过校验。';
      } else {
        historicalInteractiveResult.value = response;
        interactiveNotice.value = '历史快照结果已校验，仅供回顾，不作为当前行情的分析结论。';
      }
      await loadInteractiveTasks(fingerprint);
    } catch (e) {
      if (analysis.value?.agent_context_fingerprint === fingerprint) interactiveError.value = `导入失败：${e}`;
    } finally {
      interactiveBusy.value = false;
    }
  }

  async function analyzeWithAgent(userQuestion = '') {
    const contextFingerprint = analysis.value?.agent_context_fingerprint;
    if (!contextFingerprint || !agentStatus.value?.installed || agentLoading.value || teamLoading.value) return;
    const request = ++agentRequest;
    agentLoading.value = true;
    agentAnalysis.value = null;
    try {
      const result = await invoke<AgentAnalysisResponse>('analyze_stock_agent', {
        contextFingerprint,
        userQuestion: userQuestion.trim() || null,
      });
      if (request === agentRequest
        && analysis.value?.agent_context_fingerprint === contextFingerprint
        && result.context_fingerprint === contextFingerprint) {
        agentAnalysis.value = result;
      }
    } catch (e) {
      if (request !== agentRequest || analysis.value?.agent_context_fingerprint !== contextFingerprint) return;
      agentAnalysis.value = {
        status: 'failed', provider: 'claude_code', cached: false, conclusion: null,
        summary: null, claims: [],
        evidence: [], confidence: null, invalidation_conditions: [],
        error: String(e), guidance: '已降级为纯量化结果。', generated_at: '',
        context_fingerprint: contextFingerprint,
      };
    } finally {
      if (request === agentRequest) agentLoading.value = false;
    }
  }

  async function cancelAgentAnalysis() {
    await invoke<boolean>('cancel_agent_analysis');
  }

  async function analyzeWithTeam() {
    const contextFingerprint = analysis.value?.agent_context_fingerprint;
    if (!contextFingerprint || !agentStatus.value?.installed || teamLoading.value || agentLoading.value) return;
    const request = ++agentRequest;
    teamLoading.value = true;
    teamAnalysis.value = null;
    teamError.value = null;
    try {
      const result = await invoke<AgentTeamResponse>('analyze_stock_team', { contextFingerprint });
      if (request === agentRequest && analysis.value?.agent_context_fingerprint === contextFingerprint) {
        teamAnalysis.value = result;
      }
    } catch (e) {
      if (request === agentRequest) teamError.value = String(e);
    } finally {
      if (request === agentRequest) teamLoading.value = false;
    }
  }

  async function runResearch() {
    if (!symbol.value || researchLoading.value) return;
    const sym = symbol.value;
    const request = ++researchRequest;
    researchLoading.value = true;
    researchError.value = null;
    try {
      const result = await invoke<ResearchReport>('run_stock_research', { symbol: sym });
      if (request === researchRequest && symbol.value === sym) research.value = result;
    } catch (e) {
      if (request === researchRequest && symbol.value === sym) researchError.value = String(e);
    } finally {
      if (request === researchRequest) researchLoading.value = false;
    }
  }

  function reset() {
    ++analysisRequest;
    ++agentRequest;
    ++researchRequest;
    if (agentLoading.value || teamLoading.value) void invoke<boolean>('cancel_agent_analysis');
    if (liveStatus.value?.running) void invoke<boolean>('cancel_live_analysis', { taskId: liveStatus.value.task_id });
    liveStatus.value = null;
    liveError.value = null;
    analysis.value = null;
    symbol.value = '';
    error.value = null;
    ruleOverride.value = null;
    agentAnalysis.value = null;
    agentLoading.value = false;
    interactiveTasks.value = [];
    interactiveActivities.value = {};
    interactiveError.value = null;
    interactiveNotice.value = null;
    historicalInteractiveResult.value = null;
    teamAnalysis.value = null;
    teamError.value = null;
    teamLoading.value = false;
    research.value = null;
    researchLoading.value = false;
    researchError.value = null;
    loading.value = false;
  }

  return {
    analysis, symbol, loading, error, ruleUsed, ruleOverride,
    agentStatus, agentAnalysis, agentLoading, interactiveTasks, interactiveActivities, interactiveBusy, interactiveError, interactiveNotice, historicalInteractiveResult,
    liveStatus, liveBusy, liveError, startLiveAnalysis, askLiveAnalysis, cancelLiveAnalysis, restoreLiveAnalysis,
    teamAnalysis, teamLoading, teamError, research, researchLoading, researchError,
    analyze, loadAgentStatus, analyzeWithAgent, analyzeWithTeam, runResearch, cancelAgentAnalysis, reset,
    loadInteractiveTasks, inspectInteractiveTask, startInteractiveAnalysis, resumeInteractiveAnalysis, importInteractiveAnalysis, deleteInteractiveAnalysis,
  };
});
