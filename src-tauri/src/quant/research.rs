use crate::domain::KLineData;
use serde::Serialize;

const HORIZON: usize = 5;
const QUANTILES: usize = 5;

#[derive(Debug, Clone, Serialize)]
pub struct QuantileResult {
    pub quantile: usize,
    pub samples: usize,
    pub avg_return_pct: f64,
    pub positive_probability: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FactorResearch {
    pub id: &'static str,
    pub label: &'static str,
    pub samples: usize,
    pub rank_ic: Option<f64>,
    pub quantiles: Vec<QuantileResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PatternResearch {
    pub id: &'static str,
    pub label: &'static str,
    pub samples: usize,
    pub positive_probability: Option<f64>,
    pub avg_return_pct: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResearchReport {
    pub bars: usize,
    pub horizon_days: usize,
    pub causal_audit_passed: bool,
    pub factors: Vec<FactorResearch>,
    pub patterns: Vec<PatternResearch>,
    pub stratification_status: &'static str,
    pub stratification_note: &'static str,
    pub intraday_status: &'static str,
    pub intraday_note: &'static str,
    pub runtime: &'static str,
}

#[derive(Clone, Copy)]
struct Observation {
    value: f64,
    future_return: f64,
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0usize), |(sum, count), value| {
        (sum + value, count + 1)
    });
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

fn factor_value(id: &str, bars: &[KLineData], index: usize) -> Option<f64> {
    let close = bars.get(index)?.close;
    if !close.is_finite() || close <= 0.0 {
        return None;
    }
    match id {
        "momentum20" if index >= 20 => Some(close / bars[index - 20].close - 1.0),
        "reversal5" if index >= 5 => Some(-(close / bars[index - 5].close - 1.0)),
        "low_volatility20" if index >= 20 => {
            let returns: Vec<_> = (index - 19..=index)
                .map(|i| bars[i].close / bars[i - 1].close - 1.0)
                .collect();
            let avg = mean(returns.iter().copied());
            Some(-mean(returns.into_iter().map(|value| (value - avg).powi(2))).sqrt())
        }
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn factor_observations(id: &str, bars: &[KLineData]) -> Vec<Observation> {
    (20..bars.len().saturating_sub(HORIZON))
        .filter_map(|index| {
            let value = factor_value(id, bars, index)?;
            let future_return = bars[index + HORIZON].close / bars[index].close - 1.0;
            future_return.is_finite().then_some(Observation {
                value,
                future_return,
            })
        })
        .collect()
}

fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<_> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut result = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && values[order[end]] == values[order[start]] {
            end += 1;
        }
        let rank = (start + end - 1) as f64 / 2.0;
        for &index in &order[start..end] {
            result[index] = rank;
        }
        start = end;
    }
    result
}

fn correlation(left: &[f64], right: &[f64]) -> Option<f64> {
    if left.len() < 10 || left.len() != right.len() {
        return None;
    }
    let left_avg = mean(left.iter().copied());
    let right_avg = mean(right.iter().copied());
    let numerator = left
        .iter()
        .zip(right)
        .map(|(a, b)| (a - left_avg) * (b - right_avg))
        .sum::<f64>();
    let denominator = (left.iter().map(|v| (v - left_avg).powi(2)).sum::<f64>()
        * right.iter().map(|v| (v - right_avg).powi(2)).sum::<f64>())
    .sqrt();
    (denominator > 0.0).then_some(numerator / denominator)
}

fn factor_report(id: &'static str, label: &'static str, bars: &[KLineData]) -> FactorResearch {
    let mut observations = factor_observations(id, bars);
    let rank_ic = correlation(
        &ranks(
            &observations
                .iter()
                .map(|item| item.value)
                .collect::<Vec<_>>(),
        ),
        &ranks(
            &observations
                .iter()
                .map(|item| item.future_return)
                .collect::<Vec<_>>(),
        ),
    );
    observations.sort_by(|a, b| a.value.total_cmp(&b.value));
    let mut buckets = vec![Vec::new(); QUANTILES];
    let total = observations.len();
    for (rank, item) in observations.into_iter().enumerate() {
        let bucket = (rank * QUANTILES / total.max(1)).min(QUANTILES - 1);
        buckets[bucket].push(item.future_return);
    }
    let quantiles = buckets
        .into_iter()
        .enumerate()
        .map(|(index, returns)| QuantileResult {
            quantile: index + 1,
            samples: returns.len(),
            avg_return_pct: mean(returns.iter().copied()) * 100.0,
            positive_probability: if returns.is_empty() {
                0.0
            } else {
                returns.iter().filter(|value| **value > 0.0).count() as f64 / returns.len() as f64
            },
        })
        .collect();
    FactorResearch {
        id,
        label,
        samples: total,
        rank_ic,
        quantiles,
    }
}

fn pattern_matches(id: &str, bars: &[KLineData], index: usize) -> bool {
    match id {
        "volume_breakout20" if index >= 20 => {
            let prior = &bars[index - 20..index];
            bars[index].close > prior.iter().map(|bar| bar.close).fold(f64::MIN, f64::max)
                && bars[index].volume as f64 > mean(prior.iter().map(|bar| bar.volume as f64))
        }
        "oversold10" if index >= 10 => {
            bars[index].close
                < bars[index - 10..index]
                    .iter()
                    .map(|bar| bar.close)
                    .fold(f64::MAX, f64::min)
        }
        "trend_pullback" if index >= 60 => {
            let ma20 = mean(bars[index - 19..=index].iter().map(|bar| bar.close));
            let ma60 = mean(bars[index - 59..=index].iter().map(|bar| bar.close));
            ma20 > ma60 && bars[index].close < ma20 && bars[index].close > ma60
        }
        _ => false,
    }
}

fn pattern_report(id: &'static str, label: &'static str, bars: &[KLineData]) -> PatternResearch {
    let mut returns = Vec::new();
    let mut drawdowns = Vec::new();
    for index in 60..bars.len().saturating_sub(HORIZON) {
        if !pattern_matches(id, bars, index) || bars[index].close <= 0.0 {
            continue;
        }
        let entry = bars[index].close;
        returns.push(bars[index + HORIZON].close / entry - 1.0);
        let minimum = bars[index + 1..=index + HORIZON]
            .iter()
            .map(|bar| bar.low)
            .fold(f64::MAX, f64::min);
        drawdowns.push(((entry - minimum) / entry).max(0.0));
    }
    let samples = returns.len();
    PatternResearch {
        id,
        label,
        samples,
        positive_probability: (samples > 0)
            .then(|| returns.iter().filter(|value| **value > 0.0).count() as f64 / samples as f64),
        avg_return_pct: (samples > 0).then(|| mean(returns.into_iter()) * 100.0),
        max_drawdown_pct: (samples > 0).then(|| drawdowns.into_iter().fold(0.0, f64::max) * 100.0),
    }
}

fn causal_audit(bars: &[KLineData]) -> bool {
    [80usize, 120, 180]
        .into_iter()
        .filter(|cutoff| *cutoff < bars.len())
        .all(|cutoff| {
            ["momentum20", "reversal5", "low_volatility20"]
                .into_iter()
                .all(|id| {
                    factor_value(id, bars, cutoff) == factor_value(id, &bars[..=cutoff], cutoff)
                })
                && ["volume_breakout20", "oversold10", "trend_pullback"]
                    .into_iter()
                    .all(|id| {
                        pattern_matches(id, bars, cutoff)
                            == pattern_matches(id, &bars[..=cutoff], cutoff)
                    })
        })
}

pub fn analyze(bars: &[KLineData]) -> Result<ResearchReport, String> {
    if bars.len() < 80 {
        return Err(format!(
            "研究样本不足：{} 根，至少需要 80 根日 K",
            bars.len()
        ));
    }
    if bars.windows(2).any(|rows| rows[0].date >= rows[1].date)
        || bars
            .iter()
            .any(|bar| !bar.close.is_finite() || bar.close <= 0.0)
    {
        return Err("K 线日期未严格递增或存在无效价格".into());
    }
    Ok(ResearchReport {
        bars: bars.len(),
        horizon_days: HORIZON,
        causal_audit_passed: causal_audit(bars),
        factors: vec![
            factor_report("momentum20", "20 日动量", bars),
            factor_report("reversal5", "5 日反转", bars),
            factor_report("low_volatility20", "20 日低波动", bars),
        ],
        patterns: vec![
            pattern_report("volume_breakout20", "20 日放量突破", bars),
            pattern_report("oversold10", "10 日新低", bars),
            pattern_report("trend_pullback", "多头趋势回踩", bars),
        ],
        stratification_status: "unavailable",
        stratification_note: "缺少点时板块与市值历史，不能用当前标签回填过去样本。",
        intraday_status: "unavailable",
        intraday_note: "当前研究输入为日 K；接入带时间戳的分钟历史后才能做盘中截止匹配。",
        runtime: "Rust 内置计算，无额外运行时",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(count: usize) -> Vec<KLineData> {
        (0..count)
            .map(|index| {
                let close = 10.0 + index as f64 * 0.03 + (index as f64 / 7.0).sin();
                KLineData {
                    date: format!("2025-{:03}", index),
                    open: close,
                    high: close * 1.01,
                    low: close * 0.99,
                    close,
                    volume: 10_000 + (index % 17) as u64 * 1_000,
                    turnover: 1.0,
                }
            })
            .collect()
    }

    #[test]
    fn research_is_causal_and_returns_distributions() {
        let report = analyze(&bars(260)).unwrap();
        assert!(report.causal_audit_passed);
        assert_eq!(report.factors.len(), 3);
        assert!(report
            .factors
            .iter()
            .all(|factor| factor.quantiles.len() == 5));
        assert!(report.patterns.iter().all(|pattern| pattern.samples > 0));
        assert_eq!(report.stratification_status, "unavailable");
    }
}
