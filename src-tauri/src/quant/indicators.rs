// src-tauri/src/quant/indicators.rs
// 技术指标计算库 —— 纯函数、无 I/O、可单测。
//
// 这是后续「个股技术评分」「全市场扫描漏斗 L3」「AI 分析」的共同地基。
// 设计约定：
// - 所有函数返回与输入等长的 Vec<f64>，前导不足的部分填 f64::NAN；
//   调用方可用 `latest_finite` 取最新有效值，或自行 `.iter().filter(|v| v.is_finite())`。
// - 命名与国内主流行情软件（通达信 / 同花顺 / 东方财富）口径一致。

/// 简单移动平均 SMA。前 period-1 个位置为 NaN。
pub fn sma(values: &[f64], period: usize) -> Vec<f64> {
    let n = values.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n < period {
        return out;
    }
    let mut sum = 0.0;
    for i in 0..n {
        sum += values[i];
        if i >= period {
            sum -= values[i - period];
        }
        if i + 1 >= period {
            out[i] = sum / period as f64;
        }
    }
    out
}

/// 指数移动平均 EMA（以首个满窗口的 SMA 为种子，与国内软件一致）。
pub fn ema(values: &[f64], period: usize) -> Vec<f64> {
    let n = values.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n < period {
        return out;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut prev = values[..period].iter().sum::<f64>() / period as f64;
    out[period - 1] = prev;
    for i in period..n {
        prev = values[i] * k + prev * (1.0 - k);
        out[i] = prev;
    }
    out
}

/// MACD 指标（默认 12/26/9，柱 = 2×(DIF−DEA)，与国内主流软件一致）。
#[derive(Debug, Clone)]
pub struct Macd {
    pub dif: Vec<f64>,
    pub dea: Vec<f64>,
    pub hist: Vec<f64>,
}

pub fn macd(closes: &[f64], fast: usize, slow: usize, signal: usize) -> Macd {
    let n = closes.len();
    let mut dif = vec![f64::NAN; n];
    let mut dea = vec![f64::NAN; n];
    let mut hist = vec![f64::NAN; n];
    if slow <= fast || fast == 0 || signal == 0 || n < slow {
        return Macd { dif, dea, hist };
    }
    let ema_fast = ema(closes, fast);
    let ema_slow = ema(closes, slow);
    for i in 0..n {
        if ema_fast[i].is_finite() && ema_slow[i].is_finite() {
            dif[i] = ema_fast[i] - ema_slow[i];
        }
    }
    // DEA = EMA(DIF, signal)。DIF 从 slow-1 起才有值，故只对这一区间求 EMA。
    let dif_slice = &dif[slow - 1..];
    let dea_slice = ema(dif_slice, signal);
    for (j, &v) in dea_slice.iter().enumerate() {
        dea[slow - 1 + j] = v;
    }
    for i in 0..n {
        if dif[i].is_finite() && dea[i].is_finite() {
            hist[i] = 2.0 * (dif[i] - dea[i]);
        }
    }
    Macd { dif, dea, hist }
}

/// RSI（Wilder 平滑）。单边行情时返回 0（全跌）或 100（全涨）。
pub fn rsi(closes: &[f64], period: usize) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n <= period {
        return out;
    }
    let mut gain_sum = 0.0;
    let mut loss_sum = 0.0;
    for i in 1..=period {
        let diff = closes[i] - closes[i - 1];
        if diff >= 0.0 {
            gain_sum += diff;
        } else {
            loss_sum -= diff;
        }
    }
    let mut avg_gain = gain_sum / period as f64;
    let mut avg_loss = loss_sum / period as f64;
    out[period] = rsi_value(avg_gain, avg_loss);
    for i in period + 1..n {
        let diff = closes[i] - closes[i - 1];
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { -diff } else { 0.0 };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        out[i] = rsi_value(avg_gain, avg_loss);
    }
    out
}

#[inline]
fn rsi_value(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        if avg_gain == 0.0 {
            return 50.0; // 横盘无涨跌
        }
        return 100.0;
    }
    100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
}

/// KDJ 指标（默认 9 日；K/D 初始 50）。
#[derive(Debug, Clone)]
pub struct Kdj {
    pub k: Vec<f64>,
    pub d: Vec<f64>,
    pub j: Vec<f64>,
}

pub fn kdj(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Kdj {
    let n = closes.len();
    let mut k = vec![f64::NAN; n];
    let mut d = vec![f64::NAN; n];
    let mut j = vec![f64::NAN; n];
    if period == 0 || n < period || highs.len() != n || lows.len() != n {
        return Kdj { k, d, j };
    }
    let mut prev_k = 50.0;
    let mut prev_d = 50.0;
    for i in 0..n {
        let start = i + 1 - period.min(i + 1);
        let mut hh = highs[start];
        let mut ll = lows[start];
        for idx in start..=i {
            if highs[idx] > hh {
                hh = highs[idx];
            }
            if lows[idx] < ll {
                ll = lows[idx];
            }
        }
        let rsv = if (hh - ll).abs() < 1e-12 {
            50.0
        } else {
            (closes[i] - ll) / (hh - ll) * 100.0
        };
        let cur_k = (2.0 * prev_k + rsv) / 3.0;
        let cur_d = (2.0 * prev_d + cur_k) / 3.0;
        let cur_j = 3.0 * cur_k - 2.0 * cur_d;
        // 前 period-1 天窗口不足，不输出（与软件从第 N 天起有值一致）
        if i + 1 >= period {
            k[i] = cur_k;
            d[i] = cur_d;
            j[i] = cur_j;
        }
        prev_k = cur_k;
        prev_d = cur_d;
    }
    Kdj { k, d, j }
}

/// 布林带 BOLL（默认 20 日，2 倍标准差）。
#[derive(Debug, Clone)]
pub struct Boll {
    pub mid: Vec<f64>,
    pub upper: Vec<f64>,
    pub lower: Vec<f64>,
}

pub fn boll(closes: &[f64], period: usize, width: f64) -> Boll {
    let n = closes.len();
    let mid = sma(closes, period);
    let mut upper = vec![f64::NAN; n];
    let mut lower = vec![f64::NAN; n];
    for i in 0..n {
        if i + 1 < period {
            continue;
        }
        let slice = &closes[i + 1 - period..=i];
        let m = mid[i];
        let var = slice.iter().map(|&x| (x - m) * (x - m)).sum::<f64>() / period as f64;
        let std = var.sqrt();
        upper[i] = m + width * std;
        lower[i] = m - width * std;
    }
    Boll { mid, upper, lower }
}

/// N 日动量（涨跌幅）：`close[i] / close[i-period] - 1`。
pub fn momentum(closes: &[f64], period: usize) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![f64::NAN; n];
    for i in period..n {
        let base = closes[i - period];
        if base != 0.0 {
            out[i] = closes[i] / base - 1.0;
        }
    }
    out
}

/// 量比：今日成交量 / 过去 period 日平均成交量（不含今日）。
pub fn volume_ratio(volumes: &[f64], period: usize) -> Vec<f64> {
    let n = volumes.len();
    let mut out = vec![f64::NAN; n];
    for i in period..n {
        let slice = &volumes[i - period..i];
        let avg = slice.iter().sum::<f64>() / period as f64;
        if avg > 0.0 {
            out[i] = volumes[i] / avg;
        }
    }
    out
}

/// 单日真实波幅 TR = max(high-low, |high-prev_close|, |low-prev_close|)。
#[inline]
pub fn true_range(high: f64, low: f64, prev_close: f64) -> f64 {
    (high - low)
        .max((high - prev_close).abs())
        .max((low - prev_close).abs())
}

/// ATR（平均真实波幅），Wilder 平滑。用于量化自动止损/止盈。
pub fn atr(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n <= period || highs.len() != n || lows.len() != n {
        return out;
    }
    let mut tr_sum = 0.0;
    for i in 1..=period {
        tr_sum += true_range(highs[i], lows[i], closes[i - 1]);
    }
    let mut prev = tr_sum / period as f64;
    out[period] = prev;
    for i in period + 1..n {
        let tr = true_range(highs[i], lows[i], closes[i - 1]);
        prev = (prev * (period as f64 - 1.0) + tr) / period as f64;
        out[i] = prev;
    }
    out
}

/// 取序列中最后一个有限值（指标的最新有效值）。
pub fn latest_finite(values: &[f64]) -> Option<f64> {
    values.iter().rev().copied().find(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn sma_basic() {
        let r = sma(&[1.0, 2.0, 3.0, 4.0, 5.0], 3);
        assert!(r[0].is_nan() && r[1].is_nan());
        assert_close(r[2], 2.0);
        assert_close(r[3], 3.0);
        assert_close(r[4], 4.0);
    }

    #[test]
    fn sma_insufficient_returns_all_nan() {
        let r = sma(&[1.0, 2.0], 3);
        assert!(r.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn ema_basic() {
        let r = ema(&[1.0, 2.0, 3.0, 4.0, 5.0], 3);
        assert!(r[0].is_nan() && r[1].is_nan());
        assert_close(r[2], 2.0);
        assert_close(r[3], 3.0);
        assert_close(r[4], 4.0);
    }

    #[test]
    fn momentum_basic() {
        let r = momentum(&[1.0, 2.0, 4.0, 8.0], 1);
        assert!(r[0].is_nan());
        assert_close(r[1], 1.0);
        assert_close(r[2], 1.0);
        assert_close(r[3], 1.0);
    }

    #[test]
    fn rsi_all_up_is_100() {
        let r = rsi(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3);
        assert_close(latest_finite(&r).unwrap(), 100.0);
    }

    #[test]
    fn rsi_all_down_is_0() {
        let r = rsi(&[6.0, 5.0, 4.0, 3.0, 2.0, 1.0], 3);
        assert_close(latest_finite(&r).unwrap(), 0.0);
    }

    #[test]
    fn rsi_flat_is_50() {
        let r = rsi(&[2.0, 2.0, 2.0, 2.0, 2.0, 2.0], 3);
        assert_close(latest_finite(&r).unwrap(), 50.0);
    }

    #[test]
    fn macd_diff_positive_in_uptrend() {
        // 加速上涨（收盘价 = i²）：短期均线领先长期均线，DIF 与柱均应为正。
        // 注意不能用「匀速上涨」——匀速时 DIF/DEA 收敛到同一常数，柱趋近 0。
        let closes: Vec<f64> = (1..=40).map(|i| (i as f64).powi(2)).collect();
        let m = macd(&closes, 12, 26, 9);
        let dif = latest_finite(&m.dif).unwrap();
        assert!(dif > 0.0, "上涨趋势 DIF 应为正，实际 {dif}");
        // hist 也应与 dif 同号（dif 领先 dea）
        let hist = latest_finite(&m.hist).unwrap();
        assert!(hist > 0.0, "上涨趋势 MACD 柱应为正，实际 {hist}");
    }

    #[test]
    fn boll_bands_envelope_price() {
        let closes: Vec<f64> = (1..=25).map(|v| v as f64).collect();
        let b = boll(&closes, 20, 2.0);
        let mid = latest_finite(&b.mid).unwrap();
        let upper = latest_finite(&b.upper).unwrap();
        let lower = latest_finite(&b.lower).unwrap();
        assert_close(mid, sma(&closes, 20)[24]);
        assert!(upper > mid && mid > lower, "应满足 upper>mid>lower");
    }

    #[test]
    fn kdj_bounded_between_roughly() {
        let closes: Vec<f64> = (1..=30).map(|v| v as f64).collect();
        let highs: Vec<f64> = closes.iter().map(|v| v + 1.0).collect();
        let lows: Vec<f64> = closes.iter().map(|v| v - 1.0).collect();
        let kdj = kdj(&highs, &lows, &closes, 9);
        let k = latest_finite(&kdj.k).unwrap();
        let d = latest_finite(&kdj.d).unwrap();
        let j = latest_finite(&kdj.j).unwrap();
        assert!(k >= 0.0 && k <= 100.0);
        assert!(d >= 0.0 && d <= 100.0);
        // J 可能越界（3K-2D），但不应离谱
        assert!(j.abs() < 200.0);
    }

    #[test]
    fn volume_ratio_basic() {
        // 今日量 30，过去 3 日均量 10 → 量比 3
        let r = volume_ratio(&[10.0, 10.0, 10.0, 30.0], 3);
        assert!(r[0].is_nan() && r[1].is_nan() && r[2].is_nan());
        assert_close(r[3], 3.0);
    }

    #[test]
    fn true_range_uses_largest_of_three() {
        // high-low=2, |high-prev|=5, |low-prev|=1 → TR=5
        assert_close(true_range(10.0, 8.0, 5.0), 5.0);
        // 跳空高开：high-low=0.5, |high-prev|=2 → TR=2
        assert_close(true_range(12.0, 11.5, 10.0), 2.0);
    }

    #[test]
    fn atr_positive_for_volatile_series() {
        let n = 30;
        let closes: Vec<f64> = (0..n).map(|i| 100.0 + i as f64).collect();
        let highs: Vec<f64> = closes.iter().map(|c| c + 1.0).collect();
        let lows: Vec<f64> = closes.iter().map(|c| c - 1.0).collect();
        let a = atr(&highs, &lows, &closes, 14);
        let last = latest_finite(&a).unwrap();
        assert!(last > 0.0, "波动序列 ATR 应为正");
        // 恒定波动 2（high-low=2，无跳空）→ TR 恒为 2 → ATR 收敛到 2
        assert!((last - 2.0).abs() < 1e-6, "ATR 应收敛到 2，实际 {last}");
    }

    #[test]
    fn atr_insufficient_returns_all_nan() {
        let closes = vec![1.0, 2.0, 3.0];
        let a = atr(&closes, &closes, &closes, 14);
        assert!(a.iter().all(|v| v.is_nan()));
    }
}
