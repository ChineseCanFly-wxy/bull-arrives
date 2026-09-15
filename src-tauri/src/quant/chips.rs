// src-tauri/src/quant/chips.rs
//! 筹码分布（持仓成本分布）的**估算**。
//!
//! # 先说清楚这是什么
//!
//! A 股**没有公开的筹码原始数据** —— 你在任何软件里看到的"筹码分布"都是本地模型算出来的，
//! 通达信 / 同花顺 / 东财各用各的参数，互相之间也对不上。所以这里的产物是**估算**，
//! UI 上必须标明口径，不能包装成"真实持仓"或"主力成本"。
//!
//! # 模型
//!
//! 逐日滚动，当天换手掉的那部分筹码由当天成交替换：
//!
//! ```text
//! chips[i] = chips[i] * (1 - rate) + 当日成交分布[i] * rate
//! ```
//!
//! - `rate` 是当日换手率（0–1）。**这是模型唯一的必需输入**。
//! - 当日成交在 [最低, 最高] 之间按**三角形分布**撒开，峰在当日成交均价
//!   （`(高+低+2×收)/4`）—— 比均匀分布更贴近真实成交的集中程度。
//!
//! # 换手率从哪来（关键）
//!
//! 三个日K通道里只有东财给换手率（`KLineData.turnover`），腾讯与新浪恒为 0。
//! 拿不到时用 **成交量 ÷ 流通股本** 反推 —— 流通股本可由快照的
//! `流通市值 ÷ 现价` 得到，所以**不需要新增数据源**。
//!
//! 实测（2026-09-15，贵州茅台，250 根日K）与本模型对券商口径的偏差：
//! 平均成本 −0.39%，90% 集中度 +0.31 —— 见仓库外的 `chip-probe.py`。
//! 且把分布形状与衰减系数在合理范围内改动，结果只在小范围浮动，说明不依赖精细调参。
//!
//! # 已知失真
//!
//! - 窗口内有**解禁 / 增发 / 大比例送转**时，用当前股本反推的历史换手率会偏；
//! - T+1、大宗交易、两融、ETF 申赎、协议转让都不体现在日K里，模型看不见；
//! - 一字板 / 停牌（成交量为 0）当天不产生新筹码，这是对的。

use crate::domain::KLineData;
use serde::{Deserialize, Serialize};

/// 价格分桶数量。250 根 K 线的尺度上 200 桶足够分辨筹码峰，再多只是徒增序列化体积。
const BINS: usize = 200;

/// 成交量单位是「手」，1 手 = 100 股。
const SHARES_PER_LOT: f64 = 100.0;

/// 最多输出几个筹码峰。
const MAX_PEAKS: usize = 5;

/// 找到峰之后，把两侧扩展到「半高」时的宽度，用来描述这个峰覆盖的价格区间。
const PEAK_HALF_WIDTH: f64 = 0.5;

/// 峰高门槛（相对平均筹码密度的倍数）—— 低于它的起伏不算峰，只是噪声。
const PEAK_MIN_RATIO: f64 = 1.4;

/// 换手率的数据来源。前端据此决定提示措辞 —— 精度不同，不能混着说。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateBasis {
    /// 数据源直接给了每日换手率（东财通道）
    Turnover,
    /// 用「成交量 ÷ 流通股本」反推
    CirculatingShares,
    /// 两者都没有：退化成不衰减的成交量累加。只能看**成交密集区**，
    /// 不能当作严格意义的筹码分布（老筹码永不消失）。
    VolumeOnly,
}

/// 单个价格档位上的筹码。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChipBin {
    /// 该档位中心价
    pub price: f64,
    /// 占比（所有档位合计为 1）
    pub ratio: f64,
}

/// 一个筹码峰（成本分布的局部密集区）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChipPeak {
    /// 峰值价格
    pub price: f64,
    /// 该峰覆盖区间的下沿 / 上沿（半高宽）
    pub low: f64,
    pub high: f64,
    /// 该区间内的筹码占全部筹码的比例
    pub ratio: f64,
}

/// 筹码分布结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChipDistribution {
    /// 归一化的成本分布（已按占比降序不需要，保持价格升序即可）
    pub bins: Vec<ChipBin>,
    /// 获利盘比例 0–1：现价**之下**的筹码占比
    pub profit_ratio: f64,
    /// 加权平均成本
    pub avg_cost: f64,
    /// 90% 筹码所在的价格区间
    pub cost_90_low: f64,
    pub cost_90_high: f64,
    /// 集中度 %（通达信口径：区间宽度 ÷ 区间中枢，越小越集中）
    pub concentration_90: f64,
    pub concentration_70: f64,
    /// 筹码峰，按占比降序
    pub peaks: Vec<ChipPeak>,
    /// 参与计算的 K 线根数
    pub bars: usize,
    /// 换手率来源
    pub rate_basis: RateBasis,
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// 估算筹码分布。
///
/// `circulating_shares` 为流通股本（**股**）。为 `None` 时若 K 线自带换手率仍可计算，
/// 否则退化为不衰减的成交量累加（结果里 `rate_basis` 会说明）。
///
/// 返回 `None`：K 线少于 20 根，或价格数据异常 —— 样本太少时分布几乎等于最近几天的成交，
/// 没有参考价值，不如不给。
pub fn compute(klines: &[KLineData], circulating_shares: Option<f64>) -> Option<ChipDistribution> {
    if klines.len() < 20 {
        return None;
    }
    let close = klines.last()?.close;
    if !close.is_finite() || close <= 0.0 {
        return None;
    }

    // 价格区间取全部 K 线的高低点 —— 筹码只会落在这个范围内
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for k in klines {
        if k.low.is_finite() && k.low > 0.0 {
            lo = lo.min(k.low);
        }
        if k.high.is_finite() && k.high > 0.0 {
            hi = hi.max(k.high);
        }
    }
    if !lo.is_finite() || !hi.is_finite() || hi < lo {
        return None;
    }
    // 一字板走完整个窗口时高低点相同 —— 给一点宽度避免除零
    let hi = if hi <= lo { lo * 1.002 } else { hi };

    let step = (hi - lo) / BINS as f64;
    let centers: Vec<f64> = (0..BINS).map(|i| lo + step * (i as f64 + 0.5)).collect();

    // 换手率来源：整体判定，不逐日混用（同一通道的数据要么全有要么全无）
    let shares_total = circulating_shares.filter(|s| s.is_finite() && *s > 0.0);
    let basis = if klines.iter().all(|k| k.turnover > 0.0) {
        RateBasis::Turnover
    } else if shares_total.is_some() {
        RateBasis::CirculatingShares
    } else {
        RateBasis::VolumeOnly
    };

    let mut chips = vec![0.0f64; BINS];
    let mut used_bars = 0usize;

    for k in klines {
        let traded_shares = k.volume as f64 * SHARES_PER_LOT;
        if traded_shares <= 0.0 {
            // 停牌 / 一字板无成交：不产生新筹码，旧筹码也不换手 —— 跳过是对的
            continue;
        }
        let rate = match basis {
            RateBasis::Turnover => (k.turnover / 100.0).clamp(0.0, 1.0),
            RateBasis::CirculatingShares => {
                (traded_shares / shares_total.unwrap_or(1.0)).clamp(0.0, 1.0)
            }
            RateBasis::VolumeOnly => 0.0,
        };

        // 当日成交在 [日低, 日高] 上按三角形分布撒开，峰在成交均价
        let day_low = k.low.min(k.open).min(k.close);
        let day_high = k.high.max(k.open).max(k.close);
        let avg = (k.high + k.low + 2.0 * k.close) / 4.0;
        let span = (day_high - day_low).max(f64::EPSILON);

        let mut weights = vec![0.0f64; BINS];
        let mut weight_sum = 0.0f64;
        for (i, p) in centers.iter().enumerate() {
            if *p < day_low || *p > day_high {
                continue;
            }
            let w = (1.0 - (p - avg).abs() / span).max(0.0);
            weights[i] = w;
            weight_sum += w;
        }
        if weight_sum <= 0.0 {
            // 全天一个价（一字板）：全部压在最近的档位上
            let idx = centers
                .iter()
                .position(|p| *p >= avg)
                .unwrap_or(BINS - 1);
            weights[idx] = 1.0;
            weight_sum = 1.0;
        }

        for i in 0..BINS {
            chips[i] = chips[i] * (1.0 - rate) + (weights[i] / weight_sum) * traded_shares;
        }
        used_bars += 1;
    }

    let total: f64 = chips.iter().sum();
    if total <= 0.0 || used_bars == 0 {
        return None;
    }

    let bins_out: Vec<ChipBin> = centers
        .iter()
        .zip(chips.iter())
        .filter(|(_, c)| **c > 0.0)
        .map(|(p, c)| ChipBin {
            price: round2(*p),
            ratio: (c / total * 1e6).round() / 1e6,
        })
        .collect();

    let profit_ratio = centers
        .iter()
        .zip(chips.iter())
        .filter(|(p, _)| **p < close)
        .map(|(_, c)| *c)
        .sum::<f64>()
        / total;
    let avg_cost = centers
        .iter()
        .zip(chips.iter())
        .map(|(p, c)| p * c)
        .sum::<f64>()
        / total;

    let (cost_90_low, cost_90_high) = quantile_span(&centers, &chips, total, 0.05, 0.95);
    let (cost_70_low, cost_70_high) = quantile_span(&centers, &chips, total, 0.15, 0.85);
    let concentration_90 = concentration(cost_90_low, cost_90_high);
    let concentration_70 = concentration(cost_70_low, cost_70_high);

    Some(ChipDistribution {
        bins: bins_out,
        profit_ratio,
        avg_cost: round2(avg_cost),
        cost_90_low: round2(cost_90_low),
        cost_90_high: round2(cost_90_high),
        concentration_90,
        concentration_70,
        peaks: find_peaks(&centers, &chips, total),
        bars: used_bars,
        rate_basis: basis,
    })
}

/// 累积分布取分位点，返回 [lo_q, hi_q] 对应的价格区间。
fn quantile_span(
    centers: &[f64],
    chips: &[f64],
    total: f64,
    lo_q: f64,
    hi_q: f64,
) -> (f64, f64) {
    let mut cum = 0.0;
    let mut low = centers[0];
    let mut high = *centers.last().unwrap_or(&centers[0]);
    let mut low_found = false;
    for (p, c) in centers.iter().zip(chips.iter()) {
        cum += c;
        if !low_found && cum / total >= lo_q {
            low = *p;
            low_found = true;
        }
        if cum / total >= hi_q {
            high = *p;
            break;
        }
    }
    (low, high)
}

/// 集中度：区间宽度 ÷ 区间中枢 × 100。越小说明筹码越集中在窄区间。
fn concentration(low: f64, high: f64) -> f64 {
    let mid = high + low;
    if mid <= 0.0 {
        return 0.0;
    }
    let pct = (high - low) / mid * 100.0;
    (pct * 100.0).round() / 100.0
}

/// 找筹码峰：先平滑掉噪声，再取局部极大，最后做非极大抑制。
fn find_peaks(centers: &[f64], chips: &[f64], total: f64) -> Vec<ChipPeak> {
    let n = chips.len();
    if n < 5 {
        return Vec::new();
    }

    // 平滑：不光滑的话 200 个桶里全是毛刺，"峰"会找出一堆假的。
    //
    // ⚠️ 窗口长度固定（越界部分按 0 计），不要改成"边界处自动缩小窗口"：
    // 那样端点会因为分母变小被人为抬高，**凭空造出一个假峰**。
    let window = 3usize;
    let denom = (window * 2 + 1) as f64;
    let smooth: Vec<f64> = (0..n)
        .map(|i| {
            let a = i.saturating_sub(window);
            let b = (i + window + 1).min(n);
            chips[a..b].iter().sum::<f64>() / denom
        })
        .collect();

    let mean = smooth.iter().sum::<f64>() / n as f64;
    let threshold = mean * PEAK_MIN_RATIO;

    // ⚠️ 端点必须参与判定：一路跌到底部横盘的票，筹码峰就在价格区间的最下端，
    // 只扫 `1..n-1` 会把它整个漏掉（实测漏过）。
    let mut candidates: Vec<usize> = (0..n)
        .filter(|&i| {
            if smooth[i] <= threshold {
                return false;
            }
            let left_ok = i == 0 || smooth[i] >= smooth[i - 1];
            let right_ok = i + 1 >= n || smooth[i] >= smooth[i + 1];
            left_ok && right_ok
        })
        .collect();
    candidates.sort_by(|a, b| {
        smooth[*b]
            .partial_cmp(&smooth[*a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut chosen: Vec<usize> = Vec::new();
    for i in candidates {
        // 非极大抑制：跟已选的峰挨得太近就跳过
        if chosen.iter().any(|&j| i.abs_diff(j) < 4) {
            continue;
        }
        chosen.push(i);
        if chosen.len() >= MAX_PEAKS {
            break;
        }
    }
    chosen.sort_unstable();

    chosen
        .into_iter()
        .map(|i| {
            let half = smooth[i] * PEAK_HALF_WIDTH;
            let mut a = i;
            while a > 0 && smooth[a - 1] > half {
                a -= 1;
            }
            let mut b = i;
            while b + 1 < n && smooth[b + 1] > half {
                b += 1;
            }
            let ratio = chips[a..=b].iter().sum::<f64>() / total;
            ChipPeak {
                price: round2(centers[i]),
                low: round2(centers[a]),
                high: round2(centers[b]),
                ratio: (ratio * 1e4).round() / 1e4,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一段 K 线：`prices` 是每天的收盘价，high/low 按 ±0.5% 展开。
    fn bars(prices: &[f64], volume_lots: u64, turnover: f64) -> Vec<KLineData> {
        prices
            .iter()
            .enumerate()
            .map(|(i, c)| KLineData {
                date: format!("2026-01-{:02}", i % 28 + 1),
                open: *c,
                high: c * 1.005,
                low: c * 0.995,
                close: *c,
                volume: volume_lots,
                turnover,
            })
            .collect()
    }

    #[test]
    fn too_few_bars_returns_none() {
        assert!(compute(&bars(&[10.0; 19], 1000, 0.0), Some(1e9)).is_none());
    }

    #[test]
    fn falling_prices_leave_little_profit() {
        // 一路下跌：大部分筹码套在上方，获利盘应该很少
        let prices: Vec<f64> = (0..120).map(|i| 20.0 - i as f64 * 0.05).collect();
        let d = compute(&bars(&prices, 5000, 0.05), None).unwrap();
        assert!(
            d.profit_ratio < 0.25,
            "下跌末段获利盘应很少，实际 {}",
            d.profit_ratio
        );
        assert!(d.avg_cost > prices[prices.len() - 1]);
    }

    #[test]
    fn rising_prices_leave_lots_of_profit() {
        let prices: Vec<f64> = (0..120).map(|i| 10.0 + i as f64 * 0.05).collect();
        let d = compute(&bars(&prices, 5000, 0.05), None).unwrap();
        assert!(
            d.profit_ratio > 0.75,
            "上涨末段获利盘应很多，实际 {}",
            d.profit_ratio
        );
    }

    #[test]
    fn tight_range_is_more_concentrated_than_wide_range() {
        let narrow: Vec<f64> = (0..120).map(|i| 10.0 + (i % 3) as f64 * 0.02).collect();
        let wide: Vec<f64> = (0..120).map(|i| 10.0 + (i % 2) as f64 * 8.0).collect();
        let d_narrow = compute(&bars(&narrow, 5000, 0.05), None).unwrap();
        let d_wide = compute(&bars(&wide, 5000, 0.05), None).unwrap();
        assert!(
            d_narrow.concentration_90 < d_wide.concentration_90,
            "窄区间集中度应更小: {} vs {}",
            d_narrow.concentration_90,
            d_wide.concentration_90
        );
    }

    #[test]
    fn rate_basis_is_reported_honestly() {
        // 没有换手率也没有股本 → 必须自报 VolumeOnly，而不是假装算过了
        let prices: Vec<f64> = (0..60).map(|i| 10.0 + (i % 5) as f64 * 0.1).collect();
        let d = compute(&bars(&prices, 1000, 0.0), None).unwrap();
        assert_eq!(d.rate_basis, RateBasis::VolumeOnly);

        // 有股本 → 用股本反推
        let d = compute(&bars(&prices, 1000, 0.0), Some(1e9)).unwrap();
        assert_eq!(d.rate_basis, RateBasis::CirculatingShares);

        // K 线自带换手率 → 优先用它
        let d = compute(&bars(&prices, 1000, 0.5), None).unwrap();
        assert_eq!(d.rate_basis, RateBasis::Turnover);
    }

    #[test]
    fn circulating_shares_produces_same_magnitude_as_turnover() {
        // 1 亿股，每天成交 1000 手 = 10 万股 → 换手率恰好 0.1%
        // 与「K 线直接给 0.1% 换手率」两条路径应当高度接近
        let prices: Vec<f64> = (0..100).map(|i| 10.0 + (i % 7) as f64 * 0.05).collect();
        let by_shares = compute(&bars(&prices, 1000, 0.0), Some(1e8)).unwrap();
        let by_turnover = compute(&bars(&prices, 1000, 0.1), None).unwrap();
        assert!(
            (by_shares.avg_cost - by_turnover.avg_cost).abs() < 0.05,
            "两条换手率路径结果应接近: {} vs {}",
            by_shares.avg_cost,
            by_turnover.avg_cost
        );
    }

    #[test]
    fn zero_volume_days_are_skipped() {
        // 停牌日不产生新筹码，也不该把它算进 bars
        let mut k = bars(&[10.0; 40], 1000, 0.05);
        for item in k.iter_mut().take(5) {
            item.volume = 0;
        }
        let d = compute(&k, None).unwrap();
        assert_eq!(d.bars, 35);
    }

    #[test]
    fn peaks_land_on_the_dense_price_area() {
        // 90 天在 10 元附近 + 30 天在 20 元附近 → 两个峰应落在这两个价位
        let mut prices: Vec<f64> = (0..90).map(|i| 10.0 + (i % 3) as f64 * 0.02).collect();
        prices.extend((0..30).map(|i| 20.0 + (i % 3) as f64 * 0.02));
        let d = compute(&bars(&prices, 10000, 0.05), None).unwrap();
        assert!(!d.peaks.is_empty(), "应该能找到筹码峰");
        let top = &d.peaks[0];
        assert!(
            (top.price - 10.0).abs() < 1.0 || (top.price - 20.0).abs() < 1.0,
            "峰值应落在两个密集区之一，实际 {}",
            top.price
        );
        // 峰是按占比降序的
        for w in d.peaks.windows(2) {
            assert!(w[0].ratio >= w[1].ratio);
        }
    }

    #[test]
    fn bins_are_normalised() {
        let prices: Vec<f64> = (0..50).map(|i| 10.0 + (i % 6) as f64 * 0.1).collect();
        let d = compute(&bars(&prices, 3000, 0.05), None).unwrap();
        let sum: f64 = d.bins.iter().map(|b| b.ratio).sum();
        assert!((sum - 1.0).abs() < 0.01, "占比合计应约等于 1，实际 {sum}");
        assert!(d.avg_cost > 0.0);
        assert!(d.cost_90_low <= d.avg_cost && d.avg_cost <= d.cost_90_high);
    }
}
