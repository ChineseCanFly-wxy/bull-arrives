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
    symbol.value = sym;
    loading.value = true;
    error.value = null;
    const manual = rule && rule !== 'auto' ? rule : null;
    ruleOverride.value = manual;
    ruleUsed.value = manual ?? 'auto';
    try {
      const result = await invoke<StockAnalysis>('analyze_stock', {
        symbol: sym,
        rule: manual ?? 'auto',
      });
      analysis.value = result;
      // 回填后端最终采用的规则：自动匹配时前端并不知道它会挑哪条
      if (!manual && result.rule_match?.recommended) {
        ruleUsed.value = result.rule_match.recommended;
      }
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
    ruleOverride.value = null;
  }

  return { analysis, symbol, loading, error, ruleUsed, ruleOverride, analyze, reset };
});
