//! Built-in rule causality audit. The production signal implementation lives
//! here so plans, backtests and prefix recomputation cannot drift apart.

use crate::domain::KLineData;

use super::{indicators, playbook::TradeRule};

const MAX_AUDIT_BARS: usize = 640;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CausalAudit {
    pub passed: bool,
    pub static_checks: usize,
    pub dynamic_checks: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SignalDecision {
    pub ready: bool,
    pub waiting_for: Option<String>,
    values: [Option<f64>; 4],
}

#[derive(Debug, Clone, Copy)]
struct ConditionSpec {
    id: &'static str,
    field: &'static str,
    operator: &'static str,
    window: usize,
    offset: i32,
}

#[derive(Debug, Clone, Copy)]
struct RuleSpec {
    conditions: &'static [ConditionSpec],
    basis: &'static str,
    signal_price: &'static str,
    execution_price: &'static str,
}

const TREND: &[ConditionSpec] = &[
    ConditionSpec {
        id: "close",
        field: "close",
        operator: "value",
        window: 1,
        offset: 0,
    },
    ConditionSpec {
        id: "ma5",
        field: "close",
        operator: "sma",
        window: 5,
        offset: 0,
    },
    ConditionSpec {
        id: "ma20",
        field: "close",
        operator: "sma",
        window: 20,
        offset: 0,
    },
    ConditionSpec {
        id: "ma20_lag5",
        field: "close",
        operator: "sma",
        window: 20,
        offset: -5,
    },
];
const MEAN: &[ConditionSpec] = &[
    ConditionSpec {
        id: "close",
        field: "close",
        operator: "value",
        window: 1,
        offset: 0,
    },
    ConditionSpec {
        id: "rsi12",
        field: "close",
        operator: "rsi",
        window: 12,
        offset: 0,
    },
    ConditionSpec {
        id: "boll_lower20",
        field: "close",
        operator: "boll_lower",
        window: 20,
        offset: 0,
    },
];
const BREAKOUT: &[ConditionSpec] = &[
    ConditionSpec {
        id: "close",
        field: "close",
        operator: "value",
        window: 1,
        offset: 0,
    },
    ConditionSpec {
        id: "prior_high20",
        field: "high",
        operator: "rolling_max_exclusive",
        window: 20,
        offset: -1,
    },
];

fn spec(rule: TradeRule) -> RuleSpec {
    RuleSpec {
        conditions: match rule {
            TradeRule::TrendFollow => TREND,
            TradeRule::MeanReversion => MEAN,
            TradeRule::Breakout => BREAKOUT,
        },
        basis: "qfq",
        signal_price: "close",
        execution_price: "next_open",
    }
}

fn validate_spec(rule: TradeRule, spec: &RuleSpec) -> Result<usize, String> {
    const FIELDS: &[&str] = &["open", "high", "low", "close", "volume"];
    const OPS: &[&str] = &["value", "sma", "rsi", "boll_lower", "rolling_max_exclusive"];
    let mut checks = 0;
    for condition in spec.conditions {
        if !FIELDS.contains(&condition.field) {
            return Err(format!(
                "规则 {} 的字段 {} 不在允许列表",
                rule.id(),
                condition.field
            ));
        }
        checks += 1;
        if !OPS.contains(&condition.operator) {
            return Err(format!(
                "规则 {} 的算子 {} 不在允许列表",
                rule.id(),
                condition.operator
            ));
        }
        checks += 1;
        if !(1..=250).contains(&condition.window) {
            return Err(format!(
                "规则 {} 的条件 {} 窗口必须在 1..=250",
                rule.id(),
                condition.id
            ));
        }
        checks += 1;
        if condition.offset > 0 {
            return Err(format!(
                "规则 {} 的条件 {} 使用未来偏移 {}",
                rule.id(),
                condition.id,
                condition.offset
            ));
        }
        if condition.operator == "rolling_max_exclusive" && condition.offset != -1 {
            return Err(format!(
                "规则 {} 的条件 {} 必须排除当前 K 线",
                rule.id(),
                condition.id
            ));
        }
        checks += 1;
    }
    if spec.basis != "qfq" {
        return Err(format!("规则 {} 的价格口径必须是前复权 qfq", rule.id()));
    }
    if spec.signal_price != "close" || spec.execution_price != "next_open" {
        return Err(format!(
            "规则 {} 必须按当日收盘判定、下一日开盘成交",
            rule.id()
        ));
    }
    Ok(checks + 2)
}

fn valid(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn prior_high(highs: &[f64], i: usize, period: usize) -> Option<f64> {
    if i < period {
        return None;
    }
    highs[i - period..i]
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(None, |best, value| {
            Some(best.map_or(value, |current: f64| current.max(value)))
        })
}

pub(crate) fn signal_series(klines: &[KLineData], rule: TradeRule) -> Vec<SignalDecision> {
    let closes: Vec<f64> = klines.iter().map(|row| row.close).collect();
    let highs: Vec<f64> = klines.iter().map(|row| row.high).collect();
    let ma5 = indicators::sma(&closes, 5);
    let ma20 = indicators::sma(&closes, 20);
    let rsi12 = indicators::rsi(&closes, 12);
    let boll_lower = indicators::boll(&closes, 20, 2.0).lower;

    (0..klines.len()).map(|i| match rule {
        TradeRule::TrendFollow => {
            let close = valid(closes[i]);
            let fast = valid(ma5[i]);
            let slow = valid(ma20[i]);
            let before = i.checked_sub(5).and_then(|index| valid(ma20[index]));
            let ready = matches!((close, fast, slow, before), (Some(close), Some(fast), Some(slow), Some(before)) if slow > before && fast > slow && close > slow);
            let waiting_for = (!ready).then(|| {
                if slow.is_none() { "日 K 不足 20 根，算不出 MA20".to_string() }
                else if !matches!((slow, before), (Some(now), Some(old)) if now > old) { "MA20 仍在下行，趋势尚未走稳".to_string() }
                else if !matches!((close, slow), (Some(price), Some(avg)) if price > avg) { format!("现价还没站上 MA20（{:.2}）", slow.unwrap_or_default()) }
                else { "MA5 尚未上穿 MA20".to_string() }
            });
            SignalDecision { ready, waiting_for, values: [close, fast, slow, before] }
        }
        TradeRule::MeanReversion => {
            let close = valid(closes[i]);
            let rsi = valid(rsi12[i]);
            let lower = valid(boll_lower[i]);
            let ready = rsi.is_some_and(|value| value < 30.0)
                || matches!((close, lower), (Some(price), Some(line)) if price <= line);
            let waiting_for = (!ready).then(|| format!(
                "RSI(12) 为 {}，尚未进入超卖区（<30）；也未触及布林下轨",
                rsi.map(|value| format!("{value:.0}")).unwrap_or_else(|| "-".into())
            ));
            SignalDecision { ready, waiting_for, values: [close, rsi, lower, None] }
        }
        TradeRule::Breakout => {
            let close = valid(closes[i]);
            let high = prior_high(&highs, i, 20);
            let ready = matches!((close, high), (Some(price), Some(level)) if price > level);
            let waiting_for = (!ready).then(|| match (close, high) {
                (Some(price), Some(level)) => format!("现价 {price:.2} 尚未突破前 20 日高点 {level:.2}"),
                _ => "日 K 不足 21 根，算不出前 20 日高点".to_string(),
            });
            SignalDecision { ready, waiting_for, values: [close, high, None, None] }
        }
    }).collect()
}

pub(crate) fn latest_signal(klines: &[KLineData], rule: TradeRule) -> Option<SignalDecision> {
    signal_series(klines, rule).pop()
}

fn validate_data(klines: &[KLineData]) -> Result<(), String> {
    for (index, row) in klines.iter().enumerate() {
        if index > 0 && klines[index - 1].date >= row.date {
            return Err(format!("数据日期必须严格递增且唯一：{}", row.date));
        }
        if [row.open, row.high, row.low, row.close]
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
            || row.low > row.open.min(row.close)
            || row.high < row.open.max(row.close)
            || row.low > row.high
        {
            return Err(format!("数据 OHLC 无效：{}", row.date));
        }
    }
    Ok(())
}

fn dynamic_audit_with<F>(
    klines: &[KLineData],
    rule: TradeRule,
    spec: &RuleSpec,
    evaluate: F,
) -> Result<usize, String>
where
    F: Fn(&[KLineData], TradeRule) -> Vec<SignalDecision>,
{
    let full = evaluate(klines, rule);
    let warmup = spec
        .conditions
        .iter()
        .map(|condition| condition.window + condition.offset.unsigned_abs() as usize)
        .max()
        .unwrap_or(1);
    let mut checked = 0;
    for i in warmup.saturating_sub(1)..klines.len() {
        let truncated = evaluate(&klines[..=i], rule);
        let left = &full[i];
        let right = truncated
            .last()
            .ok_or_else(|| format!("规则 {} 在 {} 未返回决策", rule.id(), klines[i].date))?;
        if left.ready != right.ready {
            return Err(format!(
                "规则 {} 在 {} 的完整序列与截断重算信号不一致",
                rule.id(),
                klines[i].date
            ));
        }
        for (index, condition) in spec.conditions.iter().enumerate() {
            if left.values[index] != right.values[index] {
                return Err(format!(
                    "规则 {} 在 {} 的条件 {} 不一致：字段={} 算子={} 窗口={} 偏移={}",
                    rule.id(),
                    klines[i].date,
                    condition.id,
                    condition.field,
                    condition.operator,
                    condition.window,
                    condition.offset
                ));
            }
        }
        checked += 1;
    }
    Ok(checked)
}

pub fn audit(klines: &[KLineData], rule: TradeRule) -> CausalAudit {
    let spec = spec(rule);
    let static_checks = match validate_spec(rule, &spec) {
        Ok(count) => count,
        Err(message) => {
            return CausalAudit {
                passed: false,
                static_checks: 0,
                dynamic_checks: 0,
                message,
            }
        }
    };
    let start = klines.len().saturating_sub(MAX_AUDIT_BARS);
    let sample = &klines[start..];
    if let Err(message) = validate_data(sample) {
        return CausalAudit {
            passed: false,
            static_checks,
            dynamic_checks: 0,
            message,
        };
    }
    match dynamic_audit_with(sample, rule, &spec, signal_series) {
        Ok(dynamic_checks) => CausalAudit {
            passed: true,
            static_checks,
            dynamic_checks,
            message: format!(
                "因果审计通过：{static_checks} 项静态检查，{dynamic_checks} 个决策截点"
            ),
        },
        Err(message) => CausalAudit {
            passed: false,
            static_checks,
            dynamic_checks: 0,
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(n: usize) -> Vec<KLineData> {
        (0..n)
            .map(|i| {
                let close = 10.0 + i as f64 * 0.03 + (i as f64 * 0.2).sin();
                KLineData {
                    date: format!("{i:04}"),
                    open: close * 0.999,
                    high: close * 1.02,
                    low: close * 0.98,
                    close,
                    volume: 10_000 + i as u64,
                    turnover: 0.0,
                }
            })
            .collect()
    }

    #[test]
    fn built_in_rules_are_prefix_stable() {
        let rows = bars(180);
        for rule in [
            TradeRule::TrendFollow,
            TradeRule::MeanReversion,
            TradeRule::Breakout,
        ] {
            let result = audit(&rows, rule);
            assert!(result.passed, "{}: {}", rule.id(), result.message);
            assert!(result.dynamic_checks > 0);
        }
    }

    #[test]
    fn static_audit_rejects_future_offset_with_location() {
        const CONDITIONS: &[ConditionSpec] = &[ConditionSpec {
            id: "bad_lead",
            field: "close",
            operator: "value",
            window: 1,
            offset: 1,
        }];
        let bad = RuleSpec {
            conditions: CONDITIONS,
            basis: "qfq",
            signal_price: "close",
            execution_price: "next_open",
        };
        let error = validate_spec(TradeRule::TrendFollow, &bad).unwrap_err();
        assert!(error.contains("bad_lead") && error.contains("未来偏移"));
    }

    #[test]
    fn dynamic_audit_catches_hidden_future_read() {
        let rows = bars(90);
        let spec = spec(TradeRule::TrendFollow);
        let error = dynamic_audit_with(&rows, TradeRule::TrendFollow, &spec, |data, _| {
            (0..data.len())
                .map(|i| SignalDecision {
                    ready: false,
                    waiting_for: None,
                    values: [
                        Some(data.last().unwrap().close),
                        Some(data[i].close),
                        None,
                        None,
                    ],
                })
                .collect()
        })
        .unwrap_err();
        assert!(error.contains("close") && error.contains("算子=value"));
    }
}
