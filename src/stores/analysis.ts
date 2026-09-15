// src/stores/analysis.ts
// 个股技术分析状态管理。

import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { StockAnalysis } from '@/types/analysis';

export const useAnalysisStore = defineStore('analysis', () => {
  const analysis = ref<StockAnalysis | null>(null);
  const symbol = ref('');
  const loading = ref(false);
  const error = ref<string | null>(null);
  /** 本次分析用的交易规则 —— 由调用方（当前策略）决定，用于界面回显 */
  const ruleUsed = ref<string>('trend_follow');

  /**
   * 分析一只股票。
   *
   * `rule` 决定用哪套交易规则算买点/止损/止盈与回测。
   * 它跟「当前策略」绑定：筛选器里选的是哪条策略，就用那条策略配套的规则 ——
   * 否则会出现「用超跌策略选出来、却按趋势规则给买点」的错配。
   */
  async function analyze(sym: string, rule?: string) {
    symbol.value = sym;
    loading.value = true;
    error.value = null;
    ruleUsed.value = rule || 'trend_follow';
    try {
      analysis.value = await invoke<StockAnalysis>('analyze_stock', {
        symbol: sym,
        rule: ruleUsed.value,
      });
    } catch (e) {
      analysis.value = null;
      error.value = `分析失败：${e}`;
    } finally {
      loading.value = false;
    }
  }

  function reset() {
    analysis.value = null;
    symbol.value = '';
    error.value = null;
  }

  return { analysis, symbol, loading, error, ruleUsed, analyze, reset };
});
