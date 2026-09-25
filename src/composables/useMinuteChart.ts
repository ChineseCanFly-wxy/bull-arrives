import { ref, type Ref, type MaybeRef, unref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { KLineData as KCLineData, DataLoader } from 'klinecharts';
import type { MinuteData } from '@/types';
import { useChartCore } from './useChartCore';

/**
 * 分时图 composable — 仅用于 MinuteChart 组件。
 * 不含副图指标、K 线懒加载等逻辑，与 K 线图表完全隔离。
 */
export function useMinuteChart(options: {
  chartRef: Ref<HTMLElement | null>;
  code: MaybeRef<string>;
  market: MaybeRef<string>;
  name?: MaybeRef<string>;
  /// 昨收价，用于悬停提示中的分时涨跌幅。
  prevClose?: MaybeRef<number | undefined>;
}) {
  const { chart, loading, error, periodToKlinecharts, syncPrecision, initChartCore, disposeChart: coreDispose, reapplyStyles } = useChartCore(options);

  let abortController: AbortController | null = null;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;
  let refreshGeneration = 0;

  /** subscribeBar 回调引用，增量推送数据到图表避免全量重绘导致的抖动 */
  let barSubscriber: ((bar: KCLineData) => void) | null = null;

  // ---- 数据映射 ----

  function mapMinuteToChart(data: MinuteData[]): KCLineData[] {
    const today = new Date();
    return data.map((d) => {
      let h = 0, m = 0;
      if (d.time.includes(':')) {
        [h, m] = d.time.split(':').map(Number);
      } else if (d.time.length >= 4) {
        h = Number(d.time.slice(0, 2));
        m = Number(d.time.slice(2, 4));
      }
      const ts = new Date(today.getFullYear(), today.getMonth(), today.getDate(), h || 0, m || 0).getTime();
      return {
        timestamp: ts,
        open: d.open ?? d.price,
        high: d.high ?? d.price,
        low: d.low ?? d.price,
        close: d.price,
        volume: d.volume,
      };
    });
  }

  // ---- 数据加载器 ----

  const klineData = ref<KCLineData[]>([]);

  const dataLoader: DataLoader = {
    getBars: async (params) => {
      if (params.type === 'init') {
        params.callback(klineData.value, { forward: false, backward: false });
      } else if (params.type === 'forward') {
        params.callback([], { forward: false, backward: false });
      } else {
        params.callback([], { forward: false, backward: false });
      }
    },
    subscribeBar: ({ callback }) => {
      barSubscriber = callback;
    },
    unsubscribeBar: () => {
      barSubscriber = null;
    },
  };

  // ---- 自动刷新 ----

  function startAutoRefresh() {
    stopAutoRefresh();
    const generation = refreshGeneration;
    const code = unref(options.code);
    const market = unref(options.market);
    let retryDelay = 5000;

    // 等当前请求结束后才安排下一次，切换股票时丢弃旧请求结果。
    const refresh = async () => {
      if (generation !== refreshGeneration || loading.value) {
        if (generation === refreshGeneration) refreshTimer = setTimeout(() => void refresh(), 5000);
        return;
      }
      try {
        const data = await invoke<MinuteData[]>('get_intraday', { code, market });
        if (generation !== refreshGeneration) return;
        const allBars = mapMinuteToChart(data);
        if (allBars.length > 0) {
          const now = Date.now();
          const validBars = allBars.filter((b) => b.timestamp <= now);
          const newLast = validBars[validBars.length - 1];
          klineData.value = validBars;
          if (barSubscriber) {
            if (newLast) barSubscriber(newLast);
          } else if (chart.value) {
            chart.value.setDataLoader(dataLoader);
          }
        }
        retryDelay = 5000;
      } catch (e) {
        if (generation !== refreshGeneration) return;
        console.error('[useMinuteChart] incremental update failed:', e);
        retryDelay = Math.min(retryDelay * 2, 30000);
      } finally {
        if (generation === refreshGeneration) refreshTimer = setTimeout(() => void refresh(), retryDelay);
      }
    };
    refreshTimer = setTimeout(() => void refresh(), retryDelay);
  }

  function stopAutoRefresh() {
    refreshGeneration++;
    if (refreshTimer !== null) {
      clearTimeout(refreshTimer);
      refreshTimer = null;
    }
  }

  // ---- 数据加载 ----

  async function loadData() {
    stopAutoRefresh();
    if (abortController) {
      abortController.abort();
    }
    abortController = new AbortController();
    const { signal } = abortController;

    loading.value = true;
    error.value = '';

    try {
      const data = await invoke<MinuteData[]>('get_intraday', {
        code: unref(options.code),
        market: unref(options.market),
      });
      if (signal.aborted) return;

      if (data.length) {
        klineData.value = mapMinuteToChart(data);
      }

      if (signal.aborted) return;
      if (chart.value) {
        chart.value.setSymbol({ ticker: unref(options.code), name: unref(options.name) || unref(options.code) });
        chart.value.setPeriod(periodToKlinecharts('minute'));
        chart.value.setDataLoader(dataLoader);
        syncPrecision(klineData.value);
      }
      startAutoRefresh();
    } catch (e) {
      if (signal.aborted) return;
      error.value = `加载数据失败: ${String(e).slice(0, 160)}`;
      console.error('[useMinuteChart] loadData failed:', e);
    } finally {
      if (!signal.aborted) {
        loading.value = false;
      }
    }
  }

  // ---- 初始化 ----

  function initChart() {
    initChartCore('minute');
  }

  function disposeChart() {
    stopAutoRefresh();
    barSubscriber = null;
    if (abortController) {
      abortController.abort();
      abortController = null;
    }
    coreDispose();
  }

  return {
    loading,
    error,
    initChart,
    loadData,
    disposeChart,
    applyTheme: reapplyStyles,
  };
}
