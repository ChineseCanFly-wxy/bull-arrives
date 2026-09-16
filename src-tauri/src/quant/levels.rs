// src-tauri/src/quant/levels.rs
//! 支撑位与压力位的识别 —— **按交易策略加权**。
//!
//! # 为什么必须按策略分
//!
//! "哪个价位重要"本身没有唯一答案，取决于你打算怎么交易：
//!
//! - **趋势跟随**：均线就是支撑。回踩 MA20 买是这套打法的入场逻辑，
//!   所以 MA 的权重最高，布林轨道基本不看。
//! - **均值回归**：布林上下轨才是主战场（下轨买、上轨走），均线只当参考。
//! - **放量突破**：前高是触发位，前高之上有没有"一层层套牢区"决定突破能不能走。
//!   这里看的是**摆动高点被触及的次数**与 20 日高点 —— 一个价位被反复冲高回落，
//!   说明上方抛压集中在那里。
//!
//! ⚠️ 这里**没有**筹码分布这一路来源（v1.5.1 移除）：A 股没有公开的筹码原始数据，
//! 各软件的"筹码峰"都是自己的模型算的、互相之间对不上，拿它当支撑压力位会误导
//! （详见 `CHANGELOG.md` v1.5.1）。上方抛压改用**被市场验证过的价位**表达 ——
//! 摆动高点、20 日高点，这些都是真实成交打出来的。
//!
//! 所以本模块不是"找出所有支撑压力就完事"，而是给每个位一个**强度分**，
//! 再乘以当前策略的权重，最后只留最该看的那几个。
//!
//! # 克制提醒
//!
//! 这些价位是**参考区间**，不是精确点位。真实市场里价位经常被击穿又拉回，
//! 强度分只表达"这个位置的参考价值相对更高"，不表达"跌到这里一定会停"。

use crate::domain::KLineData;
use crate::quant::indicators;
use crate::quant::playbook::TradeRule;
use serde::{Deserialize, Serialize};

/// 判定"摆动点"时前后各看几根 —— 太小会把噪声当结构，太大又只剩几个点。
const SWING_LOOKBACK: usize = 5;

/// 只从最近这么多根里找摆动点。更早的结构参考价值已经很低了。
const SWING_WINDOW: usize = 120;

/// 价格相差在这个比例以内的候选位视为同一个位置，合并成一条（共振）。
const MERGE_TOLERANCE: f64 = 0.008;

/// 每侧（支撑 / 压力）最多输出几条。
const MAX_LEVELS_EACH_SIDE: usize = 3;

/// 位在现价上方还是下方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelKind {
    /// 现价之下 —— 回调时的承接参考
    Support,
    /// 现价之上 —— 反弹时的阻力参考
    Resistance,
}

/// 这个价位是从哪来的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelSource {
    Ma20,
    Ma60,
    Ma120,
    BollLower,
    BollUpper,
    SwingLow,
    SwingHigh,
    PriorLow20,
    PriorHigh20,
}

impl LevelSource {
    pub fn label(self) -> &'static str {
        match self {
            LevelSource::Ma20 => "MA20",
            LevelSource::Ma60 => "MA60",
            LevelSource::Ma120 => "MA120",
            LevelSource::BollLower => "布林下轨",
            LevelSource::BollUpper => "布林上轨",
            LevelSource::SwingLow => "摆动低点",
            LevelSource::SwingHigh => "摆动高点",
            LevelSource::PriorLow20 => "20日低点",
            LevelSource::PriorHigh20 => "20日高点",
        }
    }
}

/// 一条支撑位 / 压力位。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: f64,
    pub kind: LevelKind,
    /// 强度最高的那个来源（决定用法）
    pub source: LevelSource,
    /// 可能由多个来源共振而成，这里是中文来源说明（如 "MA20 + 摆动低点"）
    pub label: String,
    /// 为什么这个位置值得看
    pub note: String,
    /// 参考强度 0–100（已按当前策略加权）
    pub strength: f64,
    /// 距现价的百分比（正数）
    pub distance_pct: f64,
}

/// 当前策略下各来源的权重倍数。
///
/// 筹码来源移除后，它原来的权重按各自策略的侧重点摊给了摇摆价（swing）与前 20 日高低
/// （prior）—— 这两路本来就承担"上方抛压 / 下方承接"的语义，交给它们最顺。
struct Weights {
    ma: f64,
    boll: f64,
    swing: f64,
    prior: f64,
}

fn rule_weights(rule: TradeRule) -> Weights {
    match rule {
        TradeRule::TrendFollow => Weights { ma: 1.35, boll: 0.80, swing: 1.10, prior: 0.95 },
        TradeRule::MeanReversion => Weights { ma: 0.90, boll: 1.40, swing: 1.00, prior: 0.85 },
        TradeRule::Breakout => Weights { ma: 0.95, boll: 0.75, swing: 1.20, prior: 1.40 },
    }
}

/// 策略是怎么看待"位置"的说明放在前端 `TRADE_RULE_OPTIONS.focus` ——
/// 规则是前端选好再传下来的，文案没必要两边各维护一份。

struct Candidate {
    price: f64,
    source: LevelSource,
    strength: f64,
    note: String,
}

/// 识别支撑与压力位。
///
/// 输入只要日 K 与当前交易规则 —— 所有来源都是**价格自己走出来的**（均线 / 布林 /
/// 摆动点 / 前 20 日高低），不依赖任何外部估算数据。
pub fn detect(klines: &[KLineData], rule: TradeRule) -> Vec<PriceLevel> {
    if klines.len() < 30 {
        return Vec::new();
    }
    let close = klines.last().map(|k| k.close).unwrap_or(0.0);
    if !close.is_finite() || close <= 0.0 {
        return Vec::new();
    }

    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let highs: Vec<f64> = klines.iter().map(|k| k.high).collect();
    let lows: Vec<f64> = klines.iter().map(|k| k.low).collect();
    let w = rule_weights(rule);

    let mut candidates: Vec<Candidate> = Vec::new();

    // ── 均线：动态支撑/压力 ──
    for (period, source) in [
        (20usize, LevelSource::Ma20),
        (60, LevelSource::Ma60),
        (120, LevelSource::Ma120),
    ] {
        let Some(v) = indicators::latest_finite(&indicators::sma(&closes, period)) else {
            continue;
        };
        if v <= 0.0 {
            continue;
        }
        // 周期越长，被触碰后失效的概率越低，但价格意义也越钝
        let base = match source {
            LevelSource::Ma20 => 68.0,
            LevelSource::Ma60 => 62.0,
            _ => 54.0,
        };
        candidates.push(Candidate {
            price: v,
            source,
            strength: base * w.ma,
            note: format!(
                "{period} 日均线当前在 {v:.2}，{}",
                if close > v { "现价在其上方，回踩到这里是常见承接位" } else { "现价在其下方，反抽到这里容易受阻" }
            ),
        });
    }

    // ── 布林轨道 ──
    let boll = indicators::boll(&closes, 20, 2.0);
    for (series, source, base) in [
        (&boll.lower, LevelSource::BollLower, 60.0),
        (&boll.upper, LevelSource::BollUpper, 55.0),
    ] {
        let Some(v) = indicators::latest_finite(series) else {
            continue;
        };
        if v <= 0.0 {
            continue;
        }
        candidates.push(Candidate {
            price: v,
            source,
            strength: base * w.boll,
            note: format!(
                "布林{}轨在 {v:.2} —— 统计上的极值位置，短期偏离后倾向回归",
                if source == LevelSource::BollLower { "下" } else { "上" }
            ),
        });
    }

    // ── 摆动高低点：真正被市场"验证过"的价位 ──
    let (swing_highs, swing_lows) = swing_points(&highs, &lows);
    for (price, touches) in swing_highs {
        candidates.push(Candidate {
            price,
            source: LevelSource::SwingHigh,
            // 被反复触及的高点更硬：每多一次 +8 分，封顶 +32
            strength: (46.0 + (touches as f64 - 1.0).max(0.0) * 8.0).min(78.0) * w.swing,
            note: if touches > 1 {
                format!("近期有 {touches} 次冲高都在 {price:.2} 附近回落，说明上方抛压集中在这里")
            } else {
                format!("{price:.2} 是近期的一个阶段性高点")
            },
        });
    }
    for (price, touches) in swing_lows {
        candidates.push(Candidate {
            price,
            source: LevelSource::SwingLow,
            strength: (46.0 + (touches as f64 - 1.0).max(0.0) * 8.0).min(78.0) * w.swing,
            note: if touches > 1 {
                format!("近期有 {touches} 次回踩都在 {price:.2} 附近止住，说明这里有承接")
            } else {
                format!("{price:.2} 是近期的一个阶段性低点")
            },
        });
    }

    // ── 前 20 日高低点（不含最后一根，避免把"正在突破"算成已成立的压力）──
    if let Some(v) = highest_before_last(&highs, 20) {
        candidates.push(Candidate {
            price: v,
            source: LevelSource::PriorHigh20,
            strength: 72.0 * w.prior,
            note: format!("前 20 日最高 {v:.2} —— 突破策略的触发位，站上去才算突破"),
        });
    }
    if let Some(v) = lowest_before_last(&lows, 20) {
        candidates.push(Candidate {
            price: v,
            source: LevelSource::PriorLow20,
            strength: 64.0 * w.prior,
            note: format!("前 20 日最低 {v:.2} —— 短线多头的最后一道防线"),
        });
    }

    if candidates.is_empty() {
        return Vec::new();
    }

    // 按价格排序后把相近的合并成一条（多来源共振 → 强度加成）
    let merged = merge_candidates(candidates);

    let side_of = |price: f64| {
        if price <= close {
            LevelKind::Support
        } else {
            LevelKind::Resistance
        }
    };

    let mut out: Vec<PriceLevel> = merged
        .into_iter()
        .map(|c| {
            let distance = (c.price - close).abs() / close * 100.0;
            // 离现价太远的位参考价值低：每远 1% 扣 2 分，最多扣 30
            let decay = (distance * 2.0).min(30.0);
            PriceLevel {
                price: (c.price * 100.0).round() / 100.0,
                kind: side_of(c.price),
                source: c.source,
                label: c.label,
                note: c.note,
                strength: ((c.strength - decay).clamp(0.0, 100.0) * 10.0).round() / 10.0,
                distance_pct: (distance * 100.0).round() / 100.0,
            }
        })
        .collect();

    // 每侧只留最强的几条；同侧内按"离现价由近到远"输出，便于从上到下读
    let mut supports: Vec<PriceLevel> = out
        .iter()
        .filter(|l| l.kind == LevelKind::Support)
        .cloned()
        .collect();
    let mut resistances: Vec<PriceLevel> = out
        .iter()
        .filter(|l| l.kind == LevelKind::Resistance)
        .cloned()
        .collect();

    supports.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap_or(std::cmp::Ordering::Equal));
    supports.truncate(MAX_LEVELS_EACH_SIDE);
    supports.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));

    resistances.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap_or(std::cmp::Ordering::Equal));
    resistances.truncate(MAX_LEVELS_EACH_SIDE);
    resistances.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

    out = resistances;
    out.extend(supports);
    out
}

struct MergedCandidate {
    price: f64,
    source: LevelSource,
    label: String,
    note: String,
    strength: f64,
}

/// 把价格接近的候选位合并：价格取加权平均，强度取最大并给共振加成。
fn merge_candidates(mut candidates: Vec<Candidate>) -> Vec<MergedCandidate> {
    candidates.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

    let mut groups: Vec<Vec<Candidate>> = Vec::new();
    for c in candidates {
        match groups.last_mut() {
            Some(group) => {
                let reference = group[0].price;
                if (c.price - reference).abs() <= reference * MERGE_TOLERANCE {
                    group.push(c);
                } else {
                    groups.push(vec![c]);
                }
            }
            None => groups.push(vec![c]),
        }
    }

    groups
        .into_iter()
        .map(|group| {
            // 强度最高的是"主来源"，它决定这条位怎么用
            let strongest = group
                .iter()
                .max_by(|a, b| a.strength.partial_cmp(&b.strength).unwrap_or(std::cmp::Ordering::Equal))
                .expect("group is never empty");
            let total_weight: f64 = group.iter().map(|c| c.strength).sum();
            let price = if total_weight > 0.0 {
                group.iter().map(|c| c.price * c.strength).sum::<f64>() / total_weight
            } else {
                strongest.price
            };

            // 多个来源落在同一价位 = 共振，比单一来源更值得信；单一来源不加成，避免虚高
            let strength = if group.len() > 1 {
                (strongest.strength + 6.0 * (group.len() as f64 - 1.0)).min(95.0)
            } else {
                strongest.strength
            };

            let mut labels: Vec<&str> = group.iter().map(|c| c.source.label()).collect();
            labels.dedup();
            let label = labels.join(" + ");

            let mut notes: Vec<String> = group.iter().map(|c| c.note.clone()).collect();
            notes.dedup();
            let note = if notes.len() > 1 {
                notes.join("；")
            } else {
                notes.pop().unwrap_or_default()
            };

            MergedCandidate {
                price,
                source: strongest.source,
                label,
                note,
                strength,
            }
        })
        .collect()
}

/// 找摆动高低点，并按价格聚类成 (价位, 被触及次数)。
///
/// 只扫最近 `SWING_WINDOW` 根，且两端各留 `SWING_LOOKBACK` 根作为比较窗口 ——
/// 序列两端的点没有完整的前后邻域，不参与判定。
fn swing_points(highs: &[f64], lows: &[f64]) -> (Vec<(f64, usize)>, Vec<(f64, usize)>) {
    let n = highs.len();
    if n < SWING_LOOKBACK * 2 + 1 {
        return (Vec::new(), Vec::new());
    }
    let start = n.saturating_sub(SWING_WINDOW);

    let mut high_pts = Vec::new();
    let mut low_pts = Vec::new();
    for i in (start + SWING_LOOKBACK)..(n - SWING_LOOKBACK) {
        let window_hi = &highs[i - SWING_LOOKBACK..=i + SWING_LOOKBACK];
        let is_high = window_hi.iter().all(|v| *v <= highs[i]);
        if is_high && highs[i].is_finite() && highs[i] > 0.0 {
            high_pts.push(highs[i]);
        }
        let window_lo = &lows[i - SWING_LOOKBACK..=i + SWING_LOOKBACK];
        let is_low = window_lo.iter().all(|v| *v >= lows[i]);
        if is_low && lows[i].is_finite() && lows[i] > 0.0 {
            low_pts.push(lows[i]);
        }
    }

    (cluster_prices(high_pts), cluster_prices(low_pts))
}

/// 把价格接近的点归为一组，返回 (组均价, 组内个数)。
fn cluster_prices(mut points: Vec<f64>) -> Vec<(f64, usize)> {
    points.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<(f64, usize)> = Vec::new();
    for p in points {
        match out.last_mut() {
            Some((price, count)) if (p - *price).abs() <= *price * MERGE_TOLERANCE => {
                // 增量平均，避免额外存一个累加器
                *price = (*price * *count as f64 + p) / (*count as f64 + 1.0);
                *count += 1;
            }
            _ => out.push((p, 1)),
        }
    }
    out
}

/// 最近 `n` 根（**不含最后一根**）的最高价。
fn highest_before_last(values: &[f64], n: usize) -> Option<f64> {
    let end = values.len().checked_sub(1)?;
    let start = end.saturating_sub(n);
    values[start..end]
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.max(v))))
}

/// 最近 `n` 根（**不含最后一根**）的最低价。
fn lowest_before_last(values: &[f64], n: usize) -> Option<f64> {
    let end = values.len().checked_sub(1)?;
    let start = end.saturating_sub(n);
    values[start..end]
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.min(v))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一段有明确高低点的 K 线：`prices` 为收盘序列。
    fn bars(prices: &[f64]) -> Vec<KLineData> {
        prices
            .iter()
            .enumerate()
            .map(|(i, c)| KLineData {
                date: format!("2026-{:02}-{:02}", i / 28 + 1, i % 28 + 1),
                open: *c,
                high: c * 1.01,
                low: c * 0.99,
                close: *c,
                volume: 1000,
                turnover: 0.0,
            })
            .collect()
    }

    #[test]
    fn too_few_bars_returns_empty() {
        let prices: Vec<f64> = (0..20).map(|i| 10.0 + i as f64 * 0.01).collect();
        assert!(detect(&bars(&prices), TradeRule::TrendFollow).is_empty());
    }

    #[test]
    fn splits_supports_below_and_resistances_above() {
        let prices: Vec<f64> = (0..150)
            .map(|i| 10.0 + (i as f64 * 0.13).sin() * 1.5 + i as f64 * 0.01)
            .collect();
        let close = *prices.last().unwrap();
        let levels = detect(&bars(&prices), TradeRule::TrendFollow);
        assert!(!levels.is_empty(), "应能识别出价位");
        for l in &levels {
            match l.kind {
                LevelKind::Support => assert!(l.price <= close + 1e-6, "支撑应在现价下方: {} vs {close}", l.price),
                LevelKind::Resistance => assert!(l.price >= close - 1e-6, "压力应在现价上方: {} vs {close}", l.price),
            }
            assert!(l.strength > 0.0 && l.strength <= 100.0);
        }
    }

    #[test]
    fn outputs_at_most_three_per_side() {
        let prices: Vec<f64> = (0..200)
            .map(|i| 20.0 + (i as f64 * 0.31).sin() * 3.0)
            .collect();
        let levels = detect(&bars(&prices), TradeRule::Breakout);
        let supports = levels.iter().filter(|l| l.kind == LevelKind::Support).count();
        let resistances = levels.iter().filter(|l| l.kind == LevelKind::Resistance).count();
        assert!(supports <= MAX_LEVELS_EACH_SIDE, "支撑最多 3 条，实际 {supports}");
        assert!(resistances <= MAX_LEVELS_EACH_SIDE, "压力最多 3 条，实际 {resistances}");
    }

    #[test]
    fn resistances_are_sorted_near_to_far() {
        let prices: Vec<f64> = (0..150)
            .map(|i| 15.0 + (i as f64 * 0.17).sin() * 2.0)
            .collect();
        let levels = detect(&bars(&prices), TradeRule::TrendFollow);
        let r: Vec<f64> = levels
            .iter()
            .filter(|l| l.kind == LevelKind::Resistance)
            .map(|l| l.price)
            .collect();
        assert!(r.windows(2).all(|w| w[0] <= w[1]), "压力应按由近到远升序: {r:?}");
    }

    #[test]
    fn strategy_changes_the_ranking() {
        // 同一条均线，在趋势跟随里应比在均值回归里得分更高
        let prices: Vec<f64> = (0..160)
            .map(|i| 12.0 + i as f64 * 0.02 + (i as f64 * 0.4).sin() * 0.3)
            .collect();
        let k = bars(&prices);
        let trend = detect(&k, TradeRule::TrendFollow);
        let mean = detect(&k, TradeRule::MeanReversion);

        let ma_score = |levels: &[PriceLevel]| -> f64 {
            levels
                .iter()
                .filter(|l| matches!(l.source, LevelSource::Ma20 | LevelSource::Ma60))
                .map(|l| l.strength)
                .fold(0.0f64, f64::max)
        };
        assert!(
            ma_score(&trend) > ma_score(&mean),
            "趋势策略里均线权重应更高: {} vs {}",
            ma_score(&trend),
            ma_score(&mean)
        );
    }

    #[test]
    fn bollinger_weighting_favours_mean_reversion() {
        let prices: Vec<f64> = (0..160)
            .map(|i| 30.0 + (i as f64 * 0.23).sin() * 2.5)
            .collect();
        let k = bars(&prices);
        let boll_score = |rule: TradeRule| -> f64 {
            detect(&k, rule)
                .iter()
                .filter(|l| matches!(l.source, LevelSource::BollLower | LevelSource::BollUpper))
                .map(|l| l.strength)
                .fold(0.0f64, f64::max)
        };
        assert!(
            boll_score(TradeRule::MeanReversion) > boll_score(TradeRule::Breakout),
            "均值回归里布林轨道权重应更高"
        );
    }

    #[test]
    fn swing_points_detect_obvious_peak_and_trough() {
        // 两个来回：10 → 14 → 10 → 14。
        //
        // ⚠️ 别用单个「上→下」来测：那种走势的低点只落在序列两端，
        // 而两端没有完整的前后邻域，按定义就不该判为摆动点 ——
        // 那种数据下"识别不出低点"是正确行为，不是 bug。
        let mut prices: Vec<f64> = (0..40).map(|i| 10.0 + i as f64 * 0.1).collect();
        prices.extend((0..40).map(|i| 14.0 - i as f64 * 0.1));
        prices.extend((0..40).map(|i| 10.0 + i as f64 * 0.1));
        let highs: Vec<f64> = prices.iter().map(|c| c * 1.01).collect();
        let lows: Vec<f64> = prices.iter().map(|c| c * 0.99).collect();
        let (h, l) = swing_points(&highs, &lows);
        assert!(!h.is_empty(), "应识别出摆动高点");
        assert!(!l.is_empty(), "应识别出摆动低点");
        let top = h.iter().map(|(p, _)| *p).fold(0.0f64, f64::max);
        assert!(top > 13.5, "最高点应在 13.5 以上，实际 {top}");
        let bottom = l.iter().map(|(p, _)| *p).fold(f64::MAX, f64::min);
        assert!(bottom < 10.5, "最低点应在 10.5 以下，实际 {bottom}");
    }
}
