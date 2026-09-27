//! Three research horizons. These are observations, not executable orders.

use crate::{datasource::history::LocalMinuteBar, domain::KLineData};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ModeSignal {
    pub mode: &'static str,
    pub label: &'static str,
    pub as_of: String,
    pub action: &'static str,
    pub reason: String,
    pub evidence: String,
}

pub fn swing(bars: &[KLineData]) -> ModeSignal {
    daily_signal(bars, false, false)
}

pub fn daily_signal(bars: &[KLineData], long: bool, intraday: bool) -> ModeSignal {
    let (mode, label) = if long { ("long_term", "中长期") }
        else if intraday { ("intraday", "盘中择时") } else { ("swing", "日线波段") };
    let mut result = ModeSignal { mode, label,
        as_of: bars.last().map(|bar| bar.date.clone()).unwrap_or_default(),
        action: "wait", reason: "已完成日 K 不足或价格无效".into(),
        evidence: "stockdb 已完成历史日 K；财报和资讯未作为本规则的交易依据".into() };
    let required = if long { 125 } else { 65 };
    if bars.len() < required || bars.iter().rev().take(required).any(|bar| !bar.close.is_finite() || bar.close <= 0.0) { return result; }
    let n = bars.len();
    let mean = |period: usize, lag: usize| bars[n-lag-period..n-lag].iter().map(|bar| bar.close).sum::<f64>() / period as f64;
    let close = bars[n-1].close;
    let ma20 = mean(20, 0);
    let ma60 = mean(60, 0);
    let exit_period = if long { 60 } else { 20 };
    if close < mean(exit_period, 0) && bars[n-2].close < mean(exit_period, 1) {
        result.action = "sell";
        result.reason = format!("趋势退出：连续两日收盘低于各自 MA{exit_period}；当前均线 {:.2}，收盘 {close:.2}", mean(exit_period, 0));
    } else {
        let aligned = close > ma20 && ma20 > ma60 && ma20 > mean(20, 5)
            && (!long || ma60 > mean(120, 0) && ma60 > mean(60, 5) && close > bars[n-61].close);
        let not_extended = intraday || close <= ma20 * 1.04;
        result.action = if aligned && not_extended { "watch_buy" } else { "wait" };
        result.reason = format!("{}：收盘 {close:.2}，MA20 {ma20:.2}，MA60 {ma60:.2}；{}",
            if aligned && not_extended { "趋势及回踩条件满足" } else { "等待趋势或回踩确认" },
            if intraday { "仅确定日线方向，必须再确认在线分钟信号" }
            else if long { "MA60 高于 MA120 且上行，60 日动量为正，距 MA20 不超过 4%" }
            else { "MA20 高于 MA60 且上行，距 MA20 不超过 4%" });
    }
    result
}

pub fn intraday(bars: &[LocalMinuteBar], day: &str) -> ModeSignal {
    let usable: Vec<_> = bars.iter().filter(|bar| bar.volume > 0.0).collect();
    let as_of = usable.last().map(|bar| bar.timestamp.clone()).unwrap_or_else(|| day.into());
    let (action, reason) = if usable.len() < 32 {
        ("wait", format!("只有 {} 根有效分钟 K；至少需要 32 根", usable.len()))
    } else {
        let last = usable[usable.len() - 1];
        let previous = &usable[usable.len() - 31..usable.len() - 1];
        let high = previous.iter().map(|bar| bar.high).fold(f64::NEG_INFINITY, f64::max);
        let avg_volume = previous.iter().map(|bar| bar.volume).sum::<f64>() / 30.0;
        let total_volume = usable.iter().map(|bar| bar.volume).sum::<f64>();
        let vwap = usable.iter().map(|bar| bar.amount).sum::<f64>() / total_volume;
        let prev_vwap = usable[..usable.len()-1].iter().map(|bar| bar.amount).sum::<f64>() / (total_volume-last.volume);
        if last.close < vwap && usable[usable.len() - 2].close < prev_vwap {
            ("sell", format!("连续两根有成交分钟 K 收于当日 VWAP {vwap:.2} 下方，盘中强度转弱"))
        } else if last.close > high && last.close > vwap && last.volume > avg_volume * 1.5 {
            ("watch_buy", format!("分钟收盘 {:.2} 突破此前 30 根高点 {high:.2}，量能为均量 {:.1} 倍且高于 VWAP {vwap:.2}", last.close, last.volume / avg_volume))
        } else {
            ("wait", format!("未同时满足突破、放量和 VWAP 确认：收盘 {:.2}，前高 {high:.2}，VWAP {vwap:.2}，量比 {:.1}", last.close, last.volume / avg_volume))
        }
    };
    ModeSignal { mode: "intraday", label: "盘中择时", as_of, action, reason,
        evidence: "本地历史 1 分钟 K 仅用于研究；实时模拟仍需新鲜报价、盘口和 T+1 校验".into() }
}

pub fn long_term(bars: &[KLineData]) -> ModeSignal {
    daily_signal(bars, true, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intraday_breakout_requires_volume_and_vwap() {
        let mut bars: Vec<_> = (0..32).map(|i| LocalMinuteBar {
            timestamp: format!("2026091809{i:04}"), open: 10.0, high: 10.1, low: 9.9,
            close: 10.0, volume: 100.0, amount: 1000.0,
        }).collect();
        bars[31].high = 10.3; bars[31].close = 10.3;
        assert_eq!(intraday(&bars, "2026-09-18").action, "wait");
        bars[31].volume = 200.0; bars[31].amount = 2060.0;
        assert_eq!(intraday(&bars, "2026-09-18").action, "watch_buy");
    }
}
