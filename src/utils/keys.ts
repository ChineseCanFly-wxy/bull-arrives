// src/utils/keys.ts
// Shared injection keys for cross-component coordination

export const CLEAR_INDEX_DETAIL_KEY = Symbol('clearIndexDetail');

export interface StockDetailTarget {
  code: string;
  market: string;
  name: string;
}

export interface StockDetailCoordinator {
  openStockDetail: (target: StockDetailTarget) => void;
  registerOpenStockFn?: (fn: (target: StockDetailTarget) => void) => void;
}

export const OPEN_STOCK_DETAIL_KEY = Symbol('openStockDetail');
