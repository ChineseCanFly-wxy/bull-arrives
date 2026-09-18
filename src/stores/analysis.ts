// src/stores/analysis.ts
// 个股技术分析状态管理。

import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { AgentAnalysisResponse, AgentStatus, AgentTeamResponse, ResearchReport, StockAnalysis } from '@/types/analysis';

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
  const teamAnalysis = ref<AgentTeamResponse | null>(null);
  const teamLoading = ref(false);
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
    if (agentLoading.value || teamLoading.value) void invoke<boolean>('cancel_agent_analysis');
    symbol.value = sym;
    loading.value = true;
    error.value = null;
    analysis.value = null;
    agentAnalysis.value = null;
    teamAnalysis.value = null;
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
    agentStatus.value = await invoke<AgentStatus>('get_agent_status');
  }

  async function analyzeWithAgent() {
    const contextFingerprint = analysis.value?.agent_context_fingerprint;
    if (!contextFingerprint || !agentStatus.value?.installed || agentLoading.value) return;
    const request = ++agentRequest;
    agentLoading.value = true;
    try {
      const result = await invoke<AgentAnalysisResponse>('analyze_stock_agent', {
        contextFingerprint,
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
        evidence: [], confidence: null, invalidation_conditions: [],
        error: String(e), guidance: '已降级为纯量化结果。', generated_at: '',
        context_fingerprint: contextFingerprint,
      };
    } finally {
      agentLoading.value = false;
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
    try {
      const result = await invoke<AgentTeamResponse>('analyze_stock_team', { contextFingerprint });
      if (request === agentRequest && analysis.value?.agent_context_fingerprint === contextFingerprint) {
        teamAnalysis.value = result;
      }
    } catch (e) {
      if (request === agentRequest) console.warn('[agent-team]', e);
    } finally {
      teamLoading.value = false;
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
    analysis.value = null;
    symbol.value = '';
    error.value = null;
    ruleOverride.value = null;
    agentAnalysis.value = null;
    teamAnalysis.value = null;
    teamLoading.value = false;
    research.value = null;
    researchLoading.value = false;
    researchError.value = null;
    loading.value = false;
  }

  return {
    analysis, symbol, loading, error, ruleUsed, ruleOverride,
    agentStatus, agentAnalysis, agentLoading, teamAnalysis, teamLoading,
    research, researchLoading, researchError,
    analyze, loadAgentStatus, analyzeWithAgent, analyzeWithTeam, runResearch, cancelAgentAnalysis, reset,
  };
});
