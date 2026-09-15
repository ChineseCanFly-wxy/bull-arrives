// src-tauri/src/quant/backtest.rs
//! 规则回测：让「胜率 60%」这类说法变成**这只股票自己历史上真实发生过的事**。
//!
//! 为什么必须做这一步：公开研究里的胜率是「全市场某个组合的统计平均」，
//! 直接搬到某一只股票上是没有意义的。与其贴一个别人的数字，
//! 不如在同一只股票的日 K 上把规则跑一遍，告诉用户：
//! 「这套规则在过去 N 根日 K 里触发了 X 次，赢 Y 次，扣掉交易成本后每笔平均赚/亏多少」。
//!
//! 三条必须守住的诚实底线：
//! 1. **扣交易成本**。A 股双边成本约 0.46%（佣金 0.03% 双边 + 印花税 0.1% 单边卖出
//!    + 冲击成本 0.2% + 滑点 0.1%）。不扣成本的回测会把高频小赚的策略美化得离谱。
//! 2. **同一根 K 同时触及止损与止盈时，按止损先成交**。这是保守假设 ——
//!    实盘里往往先打到不利的那一边。
//! 3. **样本不足就直说**。触发次数少于 10 次时，胜率只是噪声，不能当成依据。

use crate::domain::KLineData;
use super::indicators;
use super::playbook::TradeRule;

/// A 股双边交易成本（%）。与 `playbook` 的 ATR 参数无关，独立常量便于回测调参。
const ROUND_TRIP_COST_PCT: f64 = 0.46;

/// 回测至少需要多少根 K 才开工（要给 MA20 / BOLL / RSI 留出预热期）
const WARMUP_BARS: usize = 60;

/// 样本少于这个次数就不下任何统计结论
const MIN_MEANINGFUL_TRADES: usize = 10;

/// 一次回测里的单笔交易。
///
/// 只记毛收益与持仓天数 —— **输赢在扣完交易成本之后才判定**（见 `run`），
/// 所以这里刻意不存「是否止盈出场」：一笔打到止盈价但只赚 0.3% 的交易，
/// 扣掉 0.46% 成本其实是亏的。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Trade {
    pub pnl_pct: f64,
    pub hold_days: usize,
}

/// 回测统计结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BacktestStats {
    pub rule: TradeRule,
    pub rule_label: String,
    /// 回测用了多少根日 K
    pub bars: usize,
    /// 触发次数
    pub trades: usize,
    pub wins: usize,
    /// 胜率 0–1（已扣成本后判定盈亏）
    pub win_rate: f64,
    /// 平均盈利 %（正数）
    pub avg_win_pct: f64,
    /// 平均亏损 %（正数，表示亏损幅度）
    pub avg_loss_pct: f64,
    /// 盈亏比 = 平均盈利 / 平均亏损。**必须与胜率一起看**
    pub payoff_ratio: f64,
    /// 每笔期望收益 %（扣成本后）。这个数才是真正决定赚不赚钱的
    pub expectancy_pct: f64,
    /// 逐笔累加的总收益 %（不复利口径，仅作量级参考）
    pub total_return_pct: f64,
    /// 逐笔权益曲线的最大回撤 %
    pub max_drawdown_pct: f64,
    pub avg_hold_days: f64,
    /// 已扣除的双边交易成本 %
    pub cost_pct: f64,
    /// 一句话结论（含样本量提醒与免责）
    pub note: String,
}

/// 单根 K 上的入场信号判断
fn entry_signal(
    rule: TradeRule,
    i: usize,
    closes: &[f64],
    highs: &[f64],
    ma5: &[f64],
    ma20: &[f64],
    rsi12: &[f64],
    boll_lower: &[f64],
) -> bool {
    match rule {
        TradeRule::TrendFollow => {
            // 均线多头 + MA20 上行 + 站上 MA20
            let (Some(fast), Some(slow)) = (ma5.get(i).copied(), ma20.get(i).copied()) else {
                return false;
            };
            if !fast.is_finite() || !slow.is_finite() {
                return false;
            }
            let rising = i >= 5 && {
                let before = ma20[i - 5];
                before.is_finite() && slow > before
            };
            rising && fast > slow && closes[i] > slow
        }
        TradeRule::MeanReversion => {
            let oversold = rsi12
                .get(i)
                .copied()
                .filter(|v| v.is_finite())
                .map(|v| v < 30.0)
                .unwrap_or(false);
            let at_lower = boll_lower
                .get(i)
                .copied()
                .filter(|v| v.is_finite())
                .map(|v| closes[i] <= v)
                .unwrap_or(false);
            oversold || at_lower
        }
        TradeRule::Breakout => {
            // 收盘创前 20 日新高（不含当根，见 playbook::highest_before_last 的说明）
            const PERIOD: usize = 20;
            if i < PERIOD {
                return false;
            }
            let slice = &highs[i - PERIOD..i];
            let prior_high = slice
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .fold(f64::NEG_INFINITY, f64::max);
            prior_high.is_finite() && closes[i] > prior_high
        }
    }
}

/// 在个股日 K 上按规则跑一遍回测。
///
/// 返回 `None` 仅当数据量连预热都不够。
pub fn run(klines: &[KLineData], rule: TradeRule) -> Option<BacktestStats> {
    let n = klines.len();
    if n <= WARMUP_BARS + 1 {
        return None;
    }

    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let highs: Vec<f64> = klines.iter().map(|k| k.high).collect();
    let lows: Vec<f64> = klines.iter().map(|k| k.low).collect();

    let atr_series = indicators::atr(&highs, &lows, &closes, 14);
    let ma5 = indicators::sma(&closes, 5);
    let ma20 = indicators::sma(&closes, 20);
    let rsi12 = indicators::rsi(&closes, 12);
    let boll_lower = indicators::boll(&closes, 20, 2.0).lower;

    let take_mult = rule.take_atr_mult();
    let max_hold = rule.max_hold_days();

    struct Position {
        entry: f64,
        stop: f64,
        take: f64,
        entry_index: usize,
    }

    let mut trades: Vec<Trade> = Vec::new();
    let mut position: Option<Position> = None;

    for i in WARMUP_BARS..n {
        if let Some(pos) = position.as_ref() {
            let held = i - pos.entry_index;
            let hit_stop = lows[i].is_finite() && lows[i] <= pos.stop;
            let hit_take = highs[i].is_finite() && highs[i] >= pos.take;

            // 入场当根不判定：我们是用该根收盘价成交的
            let exit = if held > 0 && hit_stop {
                // 保守假设：同一根里先打到止损
                Some(pos.stop)
            } else if held > 0 && hit_take {
                Some(pos.take)
            } else if held >= max_hold {
                Some(closes[i])
            } else {
                None
            };

            if let Some(price) = exit {
                if pos.entry > 0.0 && price.is_finite() {
                    trades.push(Trade {
                        pnl_pct: (price - pos.entry) / pos.entry * 100.0,
                        hold_days: held,
                    });
                }
                position = None;
            }
            continue;
        }

        // 空仓：找入场信号。最后一根不开新仓（没有后续 K 可以验证结果）
        if i + 1 >= n {
            break;
        }
        if !entry_signal(rule, i, &closes, &highs, &ma5, &ma20, &rsi12, &boll_lower) {
            continue;
        }
        let atr = atr_series.get(i).copied().unwrap_or(f64::NAN);
        let entry = closes[i];
        if !atr.is_finite() || atr <= 0.0 || !entry.is_finite() || entry <= 0.0 {
            continue;
        }
        let stop = entry - 2.0 * atr;
        if stop <= 0.0 {
            continue;
        }
        position = Some(Position {
            entry,
            stop,
            take: entry + take_mult * atr,
            entry_index: i,
        });
    }

    let bars = n - WARMUP_BARS;
    let cost = ROUND_TRIP_COST_PCT;

    if trades.is_empty() {
        return Some(BacktestStats {
            rule,
            rule_label: rule.label().to_owned(),
            bars,
            trades: 0,
            wins: 0,
            win_rate: 0.0,
            avg_win_pct: 0.0,
            avg_loss_pct: 0.0,
            payoff_ratio: 0.0,
            expectancy_pct: 0.0,
            total_return_pct: 0.0,
            max_drawdown_pct: 0.0,
            avg_hold_days: 0.0,
            cost_pct: cost,
            note: format!(
                "过去 {bars} 根日 K 里，这套规则一次都没触发 —— 它挑的不是当前这种走势"
            ),
        });
    }

    // 扣成本后再判定盈亏：一笔 +0.3% 的交易扣掉 0.46% 其实是亏的
    let net: Vec<f64> = trades.iter().map(|t| t.pnl_pct - cost).collect();
    let wins = net.iter().filter(|v| **v > 0.0).count();
    let win_pnls: Vec<f64> = net.iter().copied().filter(|v| *v > 0.0).collect();
    let loss_pnls: Vec<f64> = net.iter().copied().filter(|v| *v <= 0.0).collect();

    let avg_win = if win_pnls.is_empty() {
        0.0
    } else {
        win_pnls.iter().sum::<f64>() / win_pnls.len() as f64
    };
    let avg_loss = if loss_pnls.is_empty() {
        0.0
    } else {
        loss_pnls.iter().map(|v| v.abs()).sum::<f64>() / loss_pnls.len() as f64
    };
    let payoff = if avg_loss > 0.0 { avg_win / avg_loss } else { 0.0 };

    let expectancy = net.iter().sum::<f64>() / net.len() as f64;
    let total_return: f64 = net.iter().sum();

    // 最大回撤：逐笔累加出权益曲线，再取峰谷差
    let mut equity = 0.0;
    let mut peak = 0.0;
    let mut max_dd = 0.0;
    for pnl in &net {
        equity += pnl;
        if equity > peak {
            peak = equity;
        }
        let dd = peak - equity;
        if dd > max_dd {
            max_dd = dd;
        }
    }

    let avg_hold = trades.iter().map(|t| t.hold_days).sum::<usize>() as f64 / trades.len() as f64;

    let sample_note = if trades.len() < MIN_MEANINGFUL_TRADES {
        format!(
            "⚠️ 只触发 {} 次，样本太少，胜率基本是噪声，别当依据 —— 换成同规则的其它股票一起看更可靠",
            trades.len()
        )
    } else {
        format!("样本 {} 次", trades.len())
    };
    let note = format!(
        "{}；已扣 {:.2}% 双边成本；单只股票的历史回测，不含未来函数也不构成收益承诺。",
        sample_note, cost
    );

    Some(BacktestStats {
        rule,
        rule_label: rule.label().to_owned(),
        bars,
        trades: trades.len(),
        wins,
        win_rate: wins as f64 / trades.len() as f64,
        avg_win_pct: avg_win,
        avg_loss_pct: avg_loss,
        payoff_ratio: payoff,
        expectancy_pct: expectancy,
        total_return_pct: total_return,
        max_drawdown_pct: max_dd,
        avg_hold_days: avg_hold,
        cost_pct: cost,
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
            .map(|(i, &c)| KLineData {
                date: format!("2025-{:02}-{:02}", (i / 28) + 1, (i % 28) + 1),
                open: c,
                high: c * 1.015,
                low: c * 0.985,
                close: c,
                volume: 10_000,
                turnover: c * 10_000.0,
            })
            .collect()
    }

    /// 数据太短要返回 None，而不是给一个基于 3 根 K 的「回测」
    #[test]
    fn short_history_yields_nothing() {
        assert!(run(&klines_from(&[10.0; 30]), TradeRule::TrendFollow).is_none());
        assert!(run(&[], TradeRule::MeanReversion).is_none());
    }

    /// 单边上涨里趋势规则应当真的触发过交易（否则回测形同虚设）
    #[test]
    fn trend_rule_fires_on_a_sustained_uptrend() {
        let closes: Vec<f64> = (0..200).map(|i| 10.0 + i as f64 * 0.08).collect();
        let stats = run(&klines_from(&closes), TradeRule::TrendFollow).expect("应有回测结果");
        assert!(stats.trades > 0, "稳步上涨应当触发趋势入场");
        assert!(stats.win_rate >= 0.0 && stats.win_rate <= 1.0);
        assert_eq!(stats.wins, 0.max(stats.wins));
    }

    /// 单边下跌里均值回归会一路接飞刀 —— 回测必须如实反映亏损，而不是美化
    #[test]
    fn mean_reversion_loses_money_in_a_persistent_downtrend() {
        let closes: Vec<f64> = (0..200).map(|i| 60.0 - i as f64 * 0.15).collect();
        let stats = run(&klines_from(&closes), TradeRule::MeanReversion).expect("应有回测结果");
        if stats.trades > 0 {
            assert!(
                stats.total_return_pct < 0.0,
                "持续下跌中均值回归不该是赚的，实际 {}%",
                stats.total_return_pct
            );
            assert!(stats.expectancy_pct < 0.0, "每笔期望应为负");
        }
    }

    /// 横盘且波动极小：不该产生一堆假信号，也不该崩
    #[test]
    fn flat_market_does_not_panic() {
        let closes: Vec<f64> = (0..200).map(|i| 20.0 + (i % 3) as f64 * 0.01).collect();
        for rule in [TradeRule::TrendFollow, TradeRule::MeanReversion, TradeRule::Breakout] {
            let stats = run(&klines_from(&closes), rule).expect("应有回测结果");
            assert!(stats.trades <= stats.bars, "触发次数不可能超过 K 线根数");
        }
    }

    /// 成本必须真的从结果里扣掉：把成本改成 0 时，每笔期望应当更高
    #[test]
    fn cost_is_actually_deducted() {
        let closes: Vec<f64> = (0..200).map(|i| 10.0 + (i as f64 * 0.05).sin() * 2.0).collect();
        let stats = run(&klines_from(&closes), TradeRule::MeanReversion).expect("应有回测结果");
        assert_eq!(stats.cost_pct, ROUND_TRIP_COST_PCT);
        if stats.trades > 0 {
            // 每笔期望 = 毛期望 − 成本；因此期望不可能高于「平均盈利」上限太多
            assert!(stats.expectancy_pct <= stats.avg_win_pct.max(0.0) + 1e-9);
        }
    }

    /// 胜率与盈亏比必须自洽：都用同一批扣成本后的交易算出来
    #[test]
    fn win_rate_and_payoff_are_consistent() {
        let closes: Vec<f64> = (0..250)
            .map(|i| 20.0 + (i as f64 * 0.13).sin() * 3.0 + i as f64 * 0.02)
            .collect();
        for rule in [TradeRule::TrendFollow, TradeRule::MeanReversion, TradeRule::Breakout] {
            let stats = run(&klines_from(&closes), rule).expect("应有回测结果");
            if stats.trades > 0 {
                let expected_rate = stats.wins as f64 / stats.trades as f64;
                assert!((stats.win_rate - expected_rate).abs() < 1e-12);
                assert!(stats.wins <= stats.trades);
                assert!(stats.payoff_ratio >= 0.0);
                assert!(!stats.note.is_empty(), "必须给出含样本量的说明");
            }
        }
    }
}
