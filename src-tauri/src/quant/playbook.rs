// src-tauri/src/quant/playbook.rs
//! 交易规则（playbook）：把「选出来的股票」变成「可以照着做的买卖计划」。
//!
//! 为什么需要这一层：筛选器跑的是**全市场单日快照**，里面没有均线、没有 RSI、
//! 没有布林轨道 —— 它只能粗筛出「看起来符合某类形态」的候选。
//! 真正的买点必须逐只看日 K 才算得准，所以这里独立成一层：
//! **策略负责粗筛，规则负责精确点位**。
//!
//! ## 三条规则的定位（来自公开研究，不是拍脑袋）
//!
//! 需要先说清楚一件事：**胜率高不等于赚钱**。
//! 公开资料里有个很典型的例子：某 ATR 追踪止损策略在沪深 300 上一年回测
//! **胜率 66.7%，但累计收益只有 +1.06%**（夏普 0.12）—— 赢的次数多，赢的钱少，
//! 一次大亏就全吐回去。所以本模块给出的结论里，胜率与盈亏比、期望值**必须同时看**：
//!
//! | 规则族 | 胜率 | 盈亏比 | 靠什么赚钱 |
//! |---|---|---|---|
//! | 均值回归 | 高（公开测算 >60%） | 低（<1.5） | 多次小赚累积；怕的是趋势行情里越跌越买 |
//! | 趋势跟随 | 低（约 40%） | 高（>2） | 少数大行情；靠止损截断亏损、让利润奔跑 |
//! | 突破 | 中 | 中（约 1.5） | 放量确认能显著抬高胜率：有测算显示给均线信号叠加
//!   量能验证后，后续 5 日上涨概率从 47% 抬到 65% |
//!
//! 因此界面上**不允许只显示胜率** —— 一个 70% 胜率、盈亏比 0.5 的规则是亏钱的。

use crate::domain::KLineData;
use super::indicators::{self, latest_finite};

/// ATR 周期（与 `monitor.rs` 的自动止损止盈保持一致，避免同一只股票两处给不同止损）
const ATR_PERIOD: usize = 14;

/// 止损的 ATR 倍数。公开测算里「金叉日收盘价 − 2×ATR(14)」是被反复验证的起始值。
const STOP_ATR_MULT: f64 = 2.0;

/// 单笔可以亏掉的本金比例。仓位由它反推：仓位 = 风险预算 / 止损幅度。
/// 2% 是行业里最常见的单笔风险上限。
const RISK_BUDGET_PCT: f64 = 2.0;

/// 单只股票的仓位上限。再看好也不该一把梭。
const MAX_POSITION_PCT: f64 = 30.0;

/// 交易规则族。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeRule {
    /// 趋势跟随：顺着均线做，止损紧、止盈远
    TrendFollow,
    /// 均值回归：超跌买入，回到中枢就走，持仓短
    MeanReversion,
    /// 突破：放量突破前高买入
    Breakout,
}

impl TradeRule {
    pub fn id(self) -> &'static str {
        match self {
            Self::TrendFollow => "trend_follow",
            Self::MeanReversion => "mean_reversion",
            Self::Breakout => "breakout",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TrendFollow => "趋势跟随",
            Self::MeanReversion => "均值回归",
            Self::Breakout => "放量突破",
        }
    }

    /// 按 id 解析；未知 id 回落到趋势跟随（信息量最大、适用面最广的一种）
    pub fn from_id(id: &str) -> Self {
        match id.trim() {
            "mean_reversion" => Self::MeanReversion,
            "breakout" => Self::Breakout,
            _ => Self::TrendFollow,
        }
    }

    /// 这类规则的「性格」：公开研究里的参考胜率区间与盈亏比方向。
    /// 只用于向用户解释「为什么这类策略胜率长这样」，不是收益承诺。
    pub fn profile(self) -> &'static str {
        match self {
            Self::TrendFollow => "胜率通常只有四成上下，但盈亏比可以到 2 以上 —— 靠少数几波大行情赚钱，多数交易是小亏出局",
            Self::MeanReversion => "胜率能到六成以上，但盈亏比普遍不足 1.5 —— 赚的是多次小钱，怕的是单边下跌里一路接飞刀",
            Self::Breakout => "胜率中等偏上，盈亏比约 1.5 —— 关键是放量确认，缺了量能验证的突破假信号率很高",
        }
    }

    /// 止盈的 ATR 倍数。均值回归目标要近（回到中枢就走），趋势跟随放得远。
    pub(crate) fn take_atr_mult(self) -> f64 {
        match self {
            Self::TrendFollow => 4.0,
            Self::MeanReversion => 2.0,
            Self::Breakout => 3.0,
        }
    }

    /// 最长持仓天数（时间止损）。均值回归是短打，趋势跟随给足空间。
    pub(crate) fn max_hold_days(self) -> usize {
        match self {
            Self::TrendFollow => 60,
            Self::MeanReversion => 5,
            Self::Breakout => 10,
        }
    }
}

/// 一只股票在某条规则下的操作计划。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TradePlan {
    pub rule: TradeRule,
    pub rule_label: String,
    /// 规则的性格说明（含胜率 / 盈亏比的定位）
    pub rule_profile: String,
    /// 参考价（多数情况是最新收盘价）
    pub reference_price: f64,
    /// 建议建仓区间。等价格落进来再买，而不是无脑市价追。
    pub buy_low: f64,
    pub buy_high: f64,
    /// 止损价
    pub stop_loss: f64,
    /// 止盈价
    pub take_profit: f64,
    /// 盈亏比 = (止盈 − 参考价) / (参考价 − 止损)
    pub risk_reward: f64,
    /// 止损幅度 %（相对参考价）
    pub stop_pct: f64,
    /// 建议仓位上限 %（按单笔风险预算反推）
    pub position_pct: f64,
    /// 当前是否已经满足入场条件
    pub ready: bool,
    /// 不满足时，缺什么条件
    pub waiting_for: Option<String>,
    /// 价位是怎么来的（逐条说明，便于用户判断是否认同）
    pub notes: Vec<String>,
}

impl TradePlan {
    /// 把计划渲染成一段可直接读的文字，供复制到笔记 / 聊天
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "{}：买入区间 {:.2}–{:.2}，止损 {:.2}（-{:.1}%），止盈 {:.2}，盈亏比 {:.1}，仓位上限 {:.0}%",
            self.rule_label,
            self.buy_low,
            self.buy_high,
            self.stop_loss,
            self.stop_pct,
            self.take_profit,
            self.risk_reward,
            self.position_pct
        );
        if !self.ready {
            if let Some(wait) = &self.waiting_for {
                out.push_str(&format!("（当前未触发：{wait}）"));
            }
        }
        out
    }
}

/// 计算一只股票在某条规则下的操作计划。
///
/// 返回 `None` 的情况：K 线不足以算出 ATR / 均线 / 布林，或价格数据异常 ——
/// 与其给一个瞎编的止损位，不如什么都不给。
pub fn plan(klines: &[KLineData], rule: TradeRule) -> Option<TradePlan> {
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let highs: Vec<f64> = klines.iter().map(|k| k.high).collect();
    let lows: Vec<f64> = klines.iter().map(|k| k.low).collect();

    let close = *closes.last()?;
    if !close.is_finite() || close <= 0.0 {
        return None;
    }

    let atr = latest_finite(&indicators::atr(&highs, &lows, &closes, ATR_PERIOD))?;
    if atr <= 0.0 {
        return None;
    }

    let ma5 = latest_finite(&indicators::sma(&closes, 5));
    let ma20 = latest_finite(&indicators::sma(&closes, 20));
    let ma20_series = indicators::sma(&closes, 20);
    let rsi12 = latest_finite(&indicators::rsi(&closes, 12));
    let boll = indicators::boll(&closes, 20, 2.0);
    let boll_lower = latest_finite(&boll.lower);
    let boll_upper = latest_finite(&boll.upper);
    let boll_mid = latest_finite(&boll.mid);

    let mut notes: Vec<String> = Vec::new();
    notes.push(format!("ATR(14) = {:.3}，波动越大止损越宽", atr));

    // MA20 是否在上行 —— 趋势跟随的必备前提，用 5 日前比较避免被单日噪声带偏
    let ma20_rising = {
        let n = ma20_series.len();
        if n > 5 {
            match (latest_finite(&ma20_series), ma20_series.get(n - 6).copied()) {
                (Some(now), Some(before)) if before.is_finite() => now > before,
                _ => false,
            }
        } else {
            false
        }
    };

    let (buy_low, buy_high, stop_loss, take_profit, ready, waiting_for) = match rule {
        TradeRule::TrendFollow => {
            let anchor = ma20.unwrap_or(close);
            // 买点挂在 MA20 附近：趋势票的常规加仓位置，而不是追高
            let low = anchor * 0.99;
            let high = (anchor * 1.02).max(low);
            let stop = close - STOP_ATR_MULT * atr;
            let take = close + rule.take_atr_mult() * atr;
            notes.push(format!(
                "买点取 MA20（{:.2}）附近：均线是趋势票的天然支撑，回踩买比追高买成本低",
                anchor
            ));
            notes.push("止损用 2×ATR 且跌破 MA20 一并视为离场信号，避免在趋势破坏后硬扛".to_string());

            let above_ma20 = ma20.map(|v| close > v).unwrap_or(false);
            let (ready, waiting) = if ma20.is_none() {
                (false, Some("日 K 不足 20 根，算不出 MA20".to_string()))
            } else if !ma20_rising {
                (false, Some("MA20 仍在下行，趋势尚未走稳".to_string()))
            } else if !above_ma20 {
                (false, Some(format!("现价还没站上 MA20（{:.2}）", anchor)))
            } else {
                let ma5_ok = ma5.map(|v| v > anchor).unwrap_or(false);
                if ma5_ok {
                    (true, None)
                } else {
                    (false, Some("MA5 尚未上穿 MA20".to_string()))
                }
            };
            (low, high, stop, take, ready, waiting)
        }
        TradeRule::MeanReversion => {
            // 买点挂在布林下轨：均值回归的经典入场位
            let lower = boll_lower.unwrap_or(close * 0.95);
            let mid = boll_mid.unwrap_or(close);
            let low = lower;
            // 上沿只取到「下轨与中轨之间靠下三分之一」，避免买在刚跌一点点的位置
            let high = (lower + (mid - lower) * 0.33).max(low);
            let stop = close - STOP_ATR_MULT * atr;
            // 止盈取「ATR 目标」与「布林上轨」中**较近**的那个：
            // 均值回归赚的是回到中枢那一段，上轨到头就该走，贪了会把小胜拖成大亏。
            let atr_take = close + rule.take_atr_mult() * atr;
            let take = match boll_upper {
                Some(upper) if upper > close => atr_take.min(upper),
                _ => atr_take,
            };
            notes.push(format!(
                "买点取布林下轨（{:.2}）附近：这是公开测算里均值回归最主要的入场信号",
                lower
            ));
            if let Some(upper) = boll_upper.filter(|u| *u > close && *u < atr_take) {
                notes.push(format!(
                    "止盈取布林上轨 {:.2}（比 {:.0}×ATR 的目标更近，先到先走）",
                    upper,
                    rule.take_atr_mult()
                ));
            } else {
                notes.push(format!(
                    "止盈只给 {:.0}×ATR —— 均值回归赚的是「回到中枢」那一段，贪了就把小胜变大亏",
                    rule.take_atr_mult()
                ));
            }
            notes.push(format!(
                "{} 个交易日内不涨就走（时间止损），别让短打变成长套",
                rule.max_hold_days()
            ));

            let (ready, waiting) = if rsi12.is_none() && boll_lower.is_none() {
                (false, Some("日 K 不足，算不出 RSI / 布林轨道".to_string()))
            } else {
                let oversold_rsi = rsi12.map(|v| v < 30.0).unwrap_or(false);
                let at_lower = boll_lower.map(|v| close <= v * 1.01).unwrap_or(false);
                if oversold_rsi || at_lower {
                    (true, None)
                } else {
                    let rsi_text = rsi12
                        .map(|v| format!("{:.0}", v))
                        .unwrap_or_else(|| "-".to_string());
                    (false, Some(format!("RSI(12) 为 {rsi_text}，尚未进入超卖区（<30）；也未触及布林下轨")))
                }
            };
            (low, high, stop, take, ready, waiting)
        }
        TradeRule::Breakout => {
            // 买点挂在「前 20 日最高价」上方 —— 突破了才算数
            let prior_high = highest_before_last(&highs, 20).unwrap_or(close);
            let low = prior_high;
            let high = prior_high * 1.02;
            let stop = close - STOP_ATR_MULT * atr;
            let take = close + rule.take_atr_mult() * atr;
            notes.push(format!(
                "买点取前 20 日最高价（{:.2}）上方：突破前高才是突破，没破之前的都不算",
                prior_high
            ));
            notes.push("放量是突破的验证条件 —— 没有量能配合的突破假信号率很高，别只价格破就上".to_string());

            let (ready, waiting) = {
                let broke = close > prior_high * 0.995;
                if broke {
                    (true, None)
                } else {
                    (
                        false,
                        Some(format!(
                            "现价 {:.2} 尚未突破前 20 日高点 {:.2}",
                            close, prior_high
                        )),
                    )
                }
            };
            (low, high, stop, take, ready, waiting)
        }
    };

    // 止损必须落在合理区间：既不能为负，也不能高于参考价（那等于一进场就触发）
    if stop_loss <= 0.0 || stop_loss >= close {
        log::warn!(
            "[playbook] {} 算出的止损价不合理（close={} stop={}），放弃输出操作计划",
            rule.id(),
            close,
            stop_loss
        );
        return None;
    }

    let stop_distance = close - stop_loss;
    let risk_reward = if stop_distance > 0.0 {
        (take_profit - close) / stop_distance
    } else {
        0.0
    };

    let stop_pct = stop_distance / close * 100.0;
    // 仓位 = 单笔风险预算 / 止损幅度，再套上限
    let position_pct = if stop_pct > 0.0 {
        (RISK_BUDGET_PCT / stop_pct * 100.0).clamp(1.0, MAX_POSITION_PCT)
    } else {
        0.0
    };
    notes.push(format!(
        "仓位上限 {:.0}%：按单笔最多亏 {}% 本金、止损幅度 {:.1}% 反推（再高也不超过 {}%）",
        position_pct, RISK_BUDGET_PCT, stop_pct, MAX_POSITION_PCT
    ));

    Some(TradePlan {
        rule,
        rule_label: rule.label().to_owned(),
        rule_profile: rule.profile().to_owned(),
        reference_price: close,
        buy_low,
        buy_high,
        stop_loss,
        take_profit,
        risk_reward,
        stop_pct,
        position_pct,
        ready,
        waiting_for,
        notes,
    })
}

/// 前 `period` 根（不含最后一根）的最高价。
///
/// 判断突破必须把「今天」排除在外 —— 否则今天的最高价天生就在集合里，
/// `close > 最高价` 永远不成立，突破规则会一次都不触发。
pub(crate) fn highest_before_last(highs: &[f64], period: usize) -> Option<f64> {
    let n = highs.len();
    if n < period + 1 {
        return None;
    }
    let slice = &highs[n - 1 - period..n - 1];
    slice
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.max(v))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一段可控的日 K：收盘价按传入序列走，高低价围绕收盘小幅波动。
    fn klines_from(closes: &[f64]) -> Vec<KLineData> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| KLineData {
                date: format!("2025-01-{:02}", (i % 28) + 1),
                open: c,
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                volume: 10_000,
                turnover: c * 10_000.0,
            })
            .collect()
    }

    /// 单边上涨：MA20 上行且价格站上均线 → 趋势规则应当给出「可入场」
    #[test]
    fn trend_rule_is_ready_on_a_steady_uptrend() {
        let closes: Vec<f64> = (0..120).map(|i| 10.0 + i as f64 * 0.1).collect();
        let plan = plan(&klines_from(&closes), TradeRule::TrendFollow).expect("应能算出计划");
        assert!(plan.ready, "稳步上涨时趋势规则应可入场：{:?}", plan.waiting_for);
        assert!(plan.take_profit > plan.reference_price);
        assert!(plan.stop_loss < plan.reference_price);
        assert!(plan.risk_reward > 0.0);
    }

    /// 单边下跌：MA20 下行 → 趋势规则必须说「不满足」，不能装作能买
    #[test]
    fn trend_rule_refuses_a_downtrend() {
        let closes: Vec<f64> = (0..120).map(|i| 30.0 - i as f64 * 0.1).collect();
        let plan = plan(&klines_from(&closes), TradeRule::TrendFollow).expect("应能算出计划");
        assert!(!plan.ready, "下跌趋势里不该给出入场信号");
        assert!(plan.waiting_for.is_some(), "被拒时必须说明差什么条件");
    }

    /// 止损必须为正、且低于参考价 —— 否则等于一开仓就触发
    #[test]
    fn stop_loss_is_always_sane() {
        for closes in [
            (0..120).map(|i| 10.0 + i as f64 * 0.1).collect::<Vec<f64>>(),
            (0..120).map(|i| 30.0 - i as f64 * 0.1).collect::<Vec<f64>>(),
            vec![50.0; 120],
        ] {
            for rule in [TradeRule::TrendFollow, TradeRule::MeanReversion, TradeRule::Breakout] {
                if let Some(plan) = plan(&klines_from(&closes), rule) {
                    assert!(plan.stop_loss > 0.0, "{:?} 止损价为负", rule);
                    assert!(plan.stop_loss < plan.reference_price, "{:?} 止损不低于现价", rule);
                    assert!(plan.position_pct >= 1.0 && plan.position_pct <= MAX_POSITION_PCT);
                }
            }
        }
    }

    /// K 线不足就什么都不给，而不是编一个价位出来
    #[test]
    fn insufficient_history_yields_no_plan() {
        let closes: Vec<f64> = (0..10).map(|i| 10.0 + i as f64).collect();
        assert!(plan(&klines_from(&closes), TradeRule::TrendFollow).is_none());
        assert!(plan(&[], TradeRule::MeanReversion).is_none());
    }

    /// 突破用的前高必须排除今日 —— 否则「收盘价 > 前高」永远不成立
    #[test]
    fn breakout_high_excludes_the_latest_bar() {
        let highs = vec![1.0, 2.0, 3.0, 4.0, 5.0, 100.0];
        // 前 5 根（下标 0..5，不含最后一根）的最高价是 5，不能被今日的 100 污染
        assert_eq!(highest_before_last(&highs, 5), Some(5.0));
        assert_eq!(highest_before_last(&[1.0, 2.0], 5), None);
    }

    /// 规则 id 往返稳定，未知 id 安全回落（前端持久化依赖这一点）
    #[test]
    fn rule_ids_round_trip_and_fall_back_safely() {
        for rule in [TradeRule::TrendFollow, TradeRule::MeanReversion, TradeRule::Breakout] {
            assert_eq!(TradeRule::from_id(rule.id()), rule);
        }
        assert_eq!(TradeRule::from_id("不存在的规则"), TradeRule::TrendFollow);
        assert_eq!(TradeRule::from_id(""), TradeRule::TrendFollow);
    }
}
