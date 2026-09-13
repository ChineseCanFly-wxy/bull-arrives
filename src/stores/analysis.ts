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

  async function analyze(sym: string) {
    symbol.value = sym;
    loading.value = true;
    error.value = null;
    try {
      analysis.value = await invoke<StockAnalysis>('analyze_stock', { symbol: sym });
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

  return { analysis, symbol, loading, error, analyze, reset };
});
