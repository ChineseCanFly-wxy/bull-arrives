export interface Holding {
  watch_id: number;
  /** 每股成本乘以 10_000 后的十进制整数文本；null 表示未填写。 */
  cost_price: string | null;
  shares: number;
}

export interface HoldingMetrics {
  marketValue: string;
  /** 未填写成本时，浮盈和收益率均不可计算。 */
  profit: string | null;
  profitRate: string | null;
}

const PRICE_SCALE = 10_000n;
const MONEY_DECIMALS_DIVISOR = 100n;
const MAX_SCALED_PRICE = 9_223_372_036_854_775_807n;
export const MAX_HOLDING_SHARES = 9_000_000_000_000_000;

/** 把用户输入的非负价格转换为后端协议使用的 4 位定点整数文本。 */
export function decimalToScaledPrice(value: string): string | null {
  const normalized = value.trim();
  if (normalized === '') return null;
  const match = /^(\d+)(?:\.(\d{0,4}))?$/.exec(normalized);
  if (!match) throw new Error('成本价应为非负数字，最多保留 4 位小数');
  const integer = BigInt(match[1]);
  const fraction = BigInt((match[2] ?? '').padEnd(4, '0'));
  const scaled = integer * PRICE_SCALE + fraction;
  if (scaled > MAX_SCALED_PRICE) throw new Error('成本价超出安全范围');
  return scaled.toString();
}

/** 把后端的定点成本恢复为适合输入框回填的规范价格文本。 */
export function scaledPriceToDecimal(value: string | null): string {
  if (value === null) return '';
  const scaled = parseScaledPrice(value);
  const integer = scaled / PRICE_SCALE;
  const fraction = (scaled % PRICE_SCALE).toString().padStart(4, '0').replace(/0+$/, '');
  return fraction ? `${integer}.${fraction}` : integer.toString();
}

/** 行情 number 只在边界处转换一次，后续金额计算全部使用 BigInt。 */
export function quoteToScaledPrice(price: number): bigint {
  if (!Number.isFinite(price) || price < 0) throw new Error('行情价格无效');
  // toFixed 同时消除科学计数法，并按行情协议固定到四位小数。
  return BigInt(price.toFixed(4).replace('.', ''));
}

export function calculateHoldingMetrics(
  holding: Pick<Holding, 'cost_price' | 'shares'>,
  quotePrice: number,
): HoldingMetrics {
  if (!Number.isSafeInteger(holding.shares) || holding.shares < 0 || holding.shares > MAX_HOLDING_SHARES) {
    throw new Error('持仓股数无效');
  }

  const price = quoteToScaledPrice(quotePrice);
  const shares = BigInt(holding.shares);
  const marketValueScaled = price * shares;
  if (holding.cost_price === null) {
    return {
      marketValue: formatScaledMoney(marketValueScaled),
      profit: null,
      profitRate: null,
    };
  }

  const cost = parseScaledPrice(holding.cost_price);
  const costBasisScaled = cost * shares;
  const profitScaled = marketValueScaled - costBasisScaled;
  return {
    marketValue: formatScaledMoney(marketValueScaled),
    profit: formatScaledMoney(profitScaled),
    // 零成本没有数学上有意义的收益率；保留 profit，收益率返回 null。
    profitRate: costBasisScaled === 0n
      ? null
      : `${formatRatio(profitScaled * 10_000n, costBasisScaled)}%`,
  };
}

/** 将缩放 10_000 倍的金额格式化为两位小数，采用 half-up 且负数对称。 */
export function formatScaledMoney(value: bigint): string {
  return formatRoundedInteger(value, MONEY_DECIMALS_DIVISOR);
}

function parseScaledPrice(value: string): bigint {
  if (!/^\d+$/.test(value)) throw new Error('定点价格格式无效');
  const parsed = BigInt(value);
  if (parsed > MAX_SCALED_PRICE) throw new Error('定点价格超出安全范围');
  return parsed;
}

/** numerator / denominator，结果是已缩放 100 倍的百分数，保留两位小数。 */
function formatRatio(numerator: bigint, denominator: bigint): string {
  const negative = numerator < 0n;
  const absolute = negative ? -numerator : numerator;
  let quotient = absolute / denominator;
  const remainder = absolute % denominator;
  if (remainder * 2n >= denominator) quotient += 1n;
  return formatHundredths(negative && quotient !== 0n ? -quotient : quotient);
}

function formatRoundedInteger(value: bigint, divisor: bigint): string {
  const negative = value < 0n;
  const absolute = negative ? -value : value;
  let rounded = absolute / divisor;
  if ((absolute % divisor) * 2n >= divisor) rounded += 1n;
  return formatHundredths(negative && rounded !== 0n ? -rounded : rounded);
}

function formatHundredths(value: bigint): string {
  const negative = value < 0n;
  const absolute = negative ? -value : value;
  const integer = absolute / 100n;
  const fraction = (absolute % 100n).toString().padStart(2, '0');
  return `${negative ? '-' : ''}${integer}.${fraction}`;
}
