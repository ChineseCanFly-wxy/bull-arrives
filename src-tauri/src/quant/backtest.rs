//! 单股规则回放与可信度门禁。
//!
//! 复权价只用于指标和信号；本地 stockdb 能提供完整未复权序列时，
//! 成交、止损和止盈使用未复权价。未提供全市场时点股票池、公司行为账本或
//! 冻结策略的实时前向证据时，门禁必须拒绝“可用/推荐”。

use super::playbook::TradeRule;
use super::{causal, indicators};
use crate::domain::KLineData;

const ROUND_TRIP_COST_PCT: f64 = 0.46;
const WARMUP_BARS: usize = 60;
const MIN_MEANINGFUL_TRADES: usize = 30;
const MIN_OOS_TRADES: usize = 5;
const BOOTSTRAP_SAMPLES: usize = 1_000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Trade {
    pub pnl_pct: f64,
    pub hold_days: usize,
    pub exit_index: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BacktestGate {
    pub key: String,
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BacktestTrustReport {
    pub eligible: bool,
    pub status: String,
    pub methodology: Vec<String>,
    pub gates: Vec<BacktestGate>,
    pub raw_execution: bool,
    pub oos_trades: usize,
    pub oos_expectancy_pct: f64,
    pub profit_probability: f64,
    pub doubled_cost_expectancy_pct: f64,
    pub cost_flip: bool,
    pub parameter_min_expectancy_pct: f64,
    pub benchmark_return_pct: Option<f64>,
    pub excess_return_pct: Option<f64>,
    pub period_returns_pct: Vec<f64>,
    pub residual_position: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BacktestStats {
    pub rule: TradeRule,
    pub rule_label: String,
    pub bars: usize,
    pub trades: usize,
    pub wins: usize,
    pub win_rate: f64,
    pub avg_win_pct: f64,
    pub avg_loss_pct: f64,
    pub payoff_ratio: f64,
    pub expectancy_pct: f64,
    pub total_return_pct: f64,
    pub max_drawdown_pct: f64,
    pub avg_hold_days: f64,
    pub cost_pct: f64,
    pub causal_audit: causal::CausalAudit,
    pub trust: BacktestTrustReport,
    pub note: String,
}

struct Position {
    entry: f64,
    stop_qfq: f64,
    take_qfq: f64,
    entry_index: usize,
}

fn raw_is_aligned(qfq: &[KLineData], raw: Option<&[KLineData]>) -> bool {
    raw.is_some_and(|raw| {
        raw.len() == qfq.len() && raw.iter().zip(qfq).all(|(raw, qfq)| raw.date == qfq.date)
    })
}

fn qfq_to_raw(level: f64, qfq: &KLineData, raw: &KLineData) -> f64 {
    if qfq.close.is_finite() && qfq.close > 0.0 {
        level * raw.close / qfq.close
    } else {
        level
    }
}

fn simulate(
    klines: &[KLineData],
    raw: Option<&[KLineData]>,
    rule: TradeRule,
    stop_mult: f64,
    take_mult: f64,
) -> (Vec<Trade>, bool) {
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let highs: Vec<f64> = klines.iter().map(|k| k.high).collect();
    let lows: Vec<f64> = klines.iter().map(|k| k.low).collect();
    let atr_series = indicators::atr(&highs, &lows, &closes, 14);
    let signals = causal::signal_series(klines, rule);
    let mut trades = Vec::new();
    let mut position: Option<Position> = None;

    for i in WARMUP_BARS..klines.len() {
        let execution = raw.map_or(&klines[i], |rows| &rows[i]);
        if let Some(pos) = position.as_ref() {
            let held = i - pos.entry_index;
            if execution.volume == 0 {
                continue;
            }
            let stop = qfq_to_raw(pos.stop_qfq, &klines[i], execution);
            let take = qfq_to_raw(pos.take_qfq, &klines[i], execution);
            let hit_stop = execution.low.is_finite() && execution.low <= stop;
            let hit_take = execution.high.is_finite() && execution.high >= take;
            let exit = if held > 0 && hit_stop {
                Some(execution.open.min(stop))
            } else if held > 0 && hit_take {
                Some(execution.open.max(take))
            } else if held >= rule.max_hold_days() {
                Some(execution.close)
            } else {
                None
            };
            if let Some(price) = exit {
                if pos.entry > 0.0 && price.is_finite() && price > 0.0 {
                    trades.push(Trade {
                        pnl_pct: (price - pos.entry) / pos.entry * 100.0,
                        hold_days: held,
                        exit_index: i,
                    });
                }
                position = None;
            }
            continue;
        }

        if i + 1 >= klines.len() || !signals[i].ready {
            continue;
        }
        let atr = atr_series.get(i).copied().unwrap_or(f64::NAN);
        let entry_index = i + 1;
        let entry_bar = raw.map_or(&klines[entry_index], |rows| &rows[entry_index]);
        if !atr.is_finite() || atr <= 0.0 || entry_bar.volume == 0 || entry_bar.open <= 0.0 {
            continue;
        }
        let qfq_entry = klines[entry_index].open;
        let stop_qfq = qfq_entry - stop_mult * atr;
        if stop_qfq <= 0.0 {
            continue;
        }
        position = Some(Position {
            entry: entry_bar.open,
            stop_qfq,
            take_qfq: qfq_entry + take_mult * atr,
            entry_index,
        });
    }
    (trades, position.is_some())
}

fn net_returns(trades: &[Trade], cost_pct: f64) -> Vec<f64> {
    trades
        .iter()
        .map(|trade| trade.pnl_pct - cost_pct)
        .collect()
}

fn expectancy(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn compounded_return(values: &[f64]) -> f64 {
    (values
        .iter()
        .fold(1.0, |equity, value| equity * (1.0 + value / 100.0))
        - 1.0)
        * 100.0
}

fn max_drawdown(values: &[f64]) -> f64 {
    let mut equity: f64 = 1.0;
    let mut peak: f64 = 1.0;
    let mut drawdown: f64 = 0.0;
    for value in values {
        equity *= 1.0 + value / 100.0;
        peak = peak.max(equity);
        drawdown = drawdown.max((peak - equity) / peak * 100.0);
    }
    drawdown
}

fn bootstrap_profit_probability(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let block = (values.len() as f64).sqrt().ceil() as usize;
    let mut seed = 0x5EED_u64;
    let mut profitable = 0;
    for _ in 0..BOOTSTRAP_SAMPLES {
        let mut sampled = 0;
        let mut total = 0.0;
        while sampled < values.len() {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let start = (seed as usize) % values.len();
            for offset in 0..block.min(values.len() - sampled) {
                total += values[(start + offset) % values.len()];
                sampled += 1;
            }
        }
        profitable += usize::from(total > 0.0);
    }
    profitable as f64 / BOOTSTRAP_SAMPLES as f64
}

fn benchmark_return(benchmark: Option<&[KLineData]>, start: &str, end: &str) -> Option<f64> {
    let rows: Vec<_> = benchmark?
        .iter()
        .filter(|row| row.date.as_str() >= start && row.date.as_str() <= end)
        .collect();
    let (first, last) = (rows.first()?, rows.last()?);
    (first.close > 0.0).then_some((last.close / first.close - 1.0) * 100.0)
}

fn gate(key: &str, label: &str, passed: bool, detail: impl Into<String>) -> BacktestGate {
    BacktestGate {
        key: key.into(),
        label: label.into(),
        passed,
        detail: detail.into(),
    }
}

/// 兼容现有纯计算调用；因没有未复权与基准证据，结果必然无法通过准入。
pub fn run(klines: &[KLineData], rule: TradeRule) -> Option<BacktestStats> {
    run_with_evidence(klines, None, None, rule)
}

pub fn run_with_evidence(
    klines: &[KLineData],
    raw: Option<&[KLineData]>,
    benchmark: Option<&[KLineData]>,
    rule: TradeRule,
) -> Option<BacktestStats> {
    if klines.len() <= WARMUP_BARS + 1 {
        return None;
    }
    let raw_execution = raw_is_aligned(klines, raw);
    let raw = if raw_execution { raw } else { None };
    let causal_audit = causal::audit(klines, rule);
    let (trades, residual_position) = simulate(klines, raw, rule, 2.0, rule.take_atr_mult());
    let net = net_returns(&trades, ROUND_TRIP_COST_PCT);
    let doubled = net_returns(&trades, ROUND_TRIP_COST_PCT * 2.0);
    let expectancy_pct = expectancy(&net);
    let doubled_cost_expectancy_pct = expectancy(&doubled);
    let cost_flip = expectancy_pct > 0.0 && doubled_cost_expectancy_pct <= 0.0;
    let split = WARMUP_BARS + (klines.len() - WARMUP_BARS) * 7 / 10;
    let oos: Vec<_> = trades
        .iter()
        .filter(|trade| trade.exit_index >= split)
        .map(|trade| trade.pnl_pct - ROUND_TRIP_COST_PCT)
        .collect();
    let oos_expectancy_pct = expectancy(&oos);
    let profit_probability = bootstrap_profit_probability(&oos);

    let sensitivity = [
        (1.5, rule.take_atr_mult() * 0.8),
        (2.0, rule.take_atr_mult()),
        (2.5, rule.take_atr_mult() * 1.2),
    ]
    .map(|(stop, take)| {
        let (candidate, _) = simulate(klines, raw, rule, stop, take);
        expectancy(&net_returns(&candidate, ROUND_TRIP_COST_PCT))
    });
    let parameter_min_expectancy_pct = sensitivity.into_iter().fold(f64::INFINITY, f64::min);

    let period_size = (klines.len() - WARMUP_BARS).div_ceil(3);
    let period_returns_pct = (0..3)
        .map(|period| {
            let start = WARMUP_BARS + period * period_size;
            let end = (start + period_size).min(klines.len());
            let values: Vec<_> = trades
                .iter()
                .filter(|trade| trade.exit_index >= start && trade.exit_index < end)
                .map(|trade| trade.pnl_pct - ROUND_TRIP_COST_PCT)
                .collect();
            compounded_return(&values)
        })
        .collect::<Vec<_>>();
    let total_return_pct = compounded_return(&net);
    let benchmark_return_pct =
        benchmark_return(benchmark, &klines[WARMUP_BARS].date, &klines.last()?.date);
    let excess_return_pct = benchmark_return_pct.map(|value| total_return_pct - value);

    let gates = vec![
        gate(
            "causal",
            "因果审计",
            causal_audit.passed,
            causal_audit.message.clone(),
        ),
        gate(
            "raw_execution",
            "未复权成交",
            raw_execution,
            if raw_execution {
                "复权价出信号，未复权价成交"
            } else {
                "缺少与信号日期完全对齐的未复权日 K"
            },
        ),
        gate(
            "oos",
            "样本外",
            oos.len() >= MIN_OOS_TRADES && oos_expectancy_pct > 0.0,
            format!(
                "后 30% 区间 {} 笔，每笔期望 {:.2}%",
                oos.len(),
                oos_expectancy_pct
            ),
        ),
        gate(
            "cost",
            "成本稳健",
            doubled_cost_expectancy_pct > 0.0,
            format!("双倍成本后每笔期望 {:.2}%", doubled_cost_expectancy_pct),
        ),
        gate(
            "parameters",
            "参数敏感性",
            parameter_min_expectancy_pct > 0.0,
            format!(
                "三组止损/止盈参数中最差每笔 {:.2}%",
                parameter_min_expectancy_pct
            ),
        ),
        gate(
            "sample",
            "样本量",
            trades.len() >= MIN_MEANINGFUL_TRADES,
            format!("{} 笔，门槛 {} 笔", trades.len(), MIN_MEANINGFUL_TRADES),
        ),
        gate(
            "bootstrap",
            "Block Bootstrap",
            oos.len() >= MIN_OOS_TRADES && profit_probability >= 0.95,
            format!("样本外盈利概率 {:.1}%", profit_probability * 100.0),
        ),
        gate(
            "multiple_testing",
            "多重检验",
            true,
            "规则不按历史收益选优，未产生参数搜索家族",
        ),
        gate(
            "benchmark",
            "基准超额",
            excess_return_pct.is_some_and(|value| value > 0.0),
            excess_return_pct.map_or_else(
                || "同期沪深300基准不可用".into(),
                |value| format!("相对沪深300 {value:+.2}%"),
            ),
        ),
        gate(
            "tradability",
            "可交易性撮合",
            false,
            "已阻断停牌，但历史 ST/板块涨跌停、一字板和部分成交尚未完整复原",
        ),
        gate(
            "point_in_time_portfolio",
            "时点股票池与组合账本",
            false,
            "当前是单股回放；未证明历史全市场股票池、先卖后买、容量、未成交和残余持仓",
        ),
        gate(
            "corporate_actions",
            "公司行为账本",
            false,
            "复权因子已用于价格换算，但送转股份与现金分红未进入持仓账本",
        ),
        gate(
            "forward",
            "实时前向验证",
            false,
            "尚无与冻结策略版本绑定的足量实时模拟证据",
        ),
    ];
    let eligible = gates.iter().all(|item| item.passed);
    let trust = BacktestTrustReport {
        eligible,
        status: if eligible {
            "已准入"
        } else {
            "未准入（仅研究回放）"
        }
        .into(),
        methodology: vec![
            "T 日收盘信号，T+1 开盘入场；当日不可卖".into(),
            if raw_execution {
                "指标/信号用前复权价，成交/止损/止盈用未复权价".into()
            } else {
                "缺少完整未复权序列，本次成交价仅作回放估算".into()
            },
            format!("每笔扣双边成本 {:.2}%，并重跑双倍成本", ROUND_TRIP_COST_PCT),
            "后 30% 时间段作样本外；固定规则不按历史收益挑选".into(),
            "样本外交易序列做 1000 次确定性移动块自助，同时拆分三段收益".into(),
            "本策略不使用财务因子；若日后加入，必须以公告日而非报告期可用".into(),
        ],
        gates,
        raw_execution,
        oos_trades: oos.len(),
        oos_expectancy_pct,
        profit_probability,
        doubled_cost_expectancy_pct,
        cost_flip,
        parameter_min_expectancy_pct,
        benchmark_return_pct,
        excess_return_pct,
        period_returns_pct,
        residual_position,
    };

    let wins = net.iter().filter(|value| **value > 0.0).count();
    let win_values: Vec<_> = net.iter().copied().filter(|value| *value > 0.0).collect();
    let loss_values: Vec<_> = net.iter().copied().filter(|value| *value <= 0.0).collect();
    let avg_win_pct = expectancy(&win_values);
    let avg_loss_pct = if loss_values.is_empty() {
        0.0
    } else {
        loss_values.iter().map(|value| value.abs()).sum::<f64>() / loss_values.len() as f64
    };
    let payoff_ratio = if avg_loss_pct > 0.0 {
        avg_win_pct / avg_loss_pct
    } else {
        0.0
    };
    let note = if trades.is_empty() {
        format!(
            "过去 {} 根有效日 K 未完成交易；{}",
            klines.len() - WARMUP_BARS,
            trust.status
        )
    } else {
        format!(
            "{}笔，每笔扣 {:.2}% 成本；双倍成本后期望 {:.2}%{}；{}。",
            trades.len(),
            ROUND_TRIP_COST_PCT,
            doubled_cost_expectancy_pct,
            if cost_flip {
                "，策略翻转为负"
            } else {
                ""
            },
            trust.status,
        )
    };

    Some(BacktestStats {
        rule,
        rule_label: rule.label().into(),
        bars: klines.len() - WARMUP_BARS,
        trades: trades.len(),
        wins,
        win_rate: if trades.is_empty() {
            0.0
        } else {
            wins as f64 / trades.len() as f64
        },
        avg_win_pct,
        avg_loss_pct,
        payoff_ratio,
        expectancy_pct,
        total_return_pct,
        max_drawdown_pct: max_drawdown(&net),
        avg_hold_days: if trades.is_empty() {
            0.0
        } else {
            trades.iter().map(|trade| trade.hold_days).sum::<usize>() as f64 / trades.len() as f64
        },
        cost_pct: ROUND_TRIP_COST_PCT,
        causal_audit,
        trust,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn klines_from(closes: &[f64]) -> Vec<KLineData> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &close)| KLineData {
                date: format!("2025-{:02}-{:02}", (i / 28) + 1, (i % 28) + 1),
                open: close,
                high: close * 1.015,
                low: close * 0.985,
                close,
                volume: 10_000,
                turnover: close * 10_000.0,
            })
            .collect()
    }

    #[test]
    fn short_history_yields_nothing() {
        assert!(run(&klines_from(&[10.0; 30]), TradeRule::TrendFollow).is_none());
    }

    #[test]
    fn missing_evidence_never_passes_admission() {
        let closes: Vec<_> = (0..250).map(|i| 10.0 + i as f64 * 0.08).collect();
        let stats = run(&klines_from(&closes), TradeRule::TrendFollow).unwrap();
        assert!(!stats.trust.eligible);
        assert!(stats
            .trust
            .gates
            .iter()
            .any(|gate| gate.key == "forward" && !gate.passed));
        assert!(stats.note.contains("未准入"));
    }

    #[test]
    fn aligned_raw_prices_are_used_for_execution() {
        let closes: Vec<_> = (0..250).map(|i| 10.0 + i as f64 * 0.08).collect();
        let qfq = klines_from(&closes);
        let mut raw = qfq.clone();
        for row in &mut raw {
            row.open *= 2.0;
            row.high *= 2.0;
            row.low *= 2.0;
            row.close *= 2.0;
        }
        let stats = run_with_evidence(&qfq, Some(&raw), None, TradeRule::TrendFollow).unwrap();
        assert!(stats.trust.raw_execution);
        assert!(!stats.trust.eligible, "实时前向等证据仍然缺失");
    }

    #[test]
    fn double_cost_flip_is_detectable() {
        let trades = [Trade {
            pnl_pct: 0.7,
            hold_days: 1,
            exit_index: 1,
        }];
        let base = expectancy(&net_returns(&trades, ROUND_TRIP_COST_PCT));
        let doubled = expectancy(&net_returns(&trades, ROUND_TRIP_COST_PCT * 2.0));
        assert!(base > 0.0 && doubled <= 0.0);
    }

    #[test]
    fn bootstrap_is_deterministic() {
        let values = [1.0, -0.2, 0.7, -0.1, 0.4, 0.2];
        assert_eq!(
            bootstrap_profit_probability(&values),
            bootstrap_profit_probability(&values)
        );
    }
}
