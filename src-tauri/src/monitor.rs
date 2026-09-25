// src-tauri/src/monitor.rs
//! 个股监控（量化自动止损/止盈）评估。
//!
//! 止损位/止盈位由**量化模型**（ATR 波动止损）自动计算，用户只选择要监控的股票。
//! 与 `alerts.rs`（用户手动填参数的涨跌幅/固定价告警）共用交易时段与新鲜行情判断，
//! 触发后一次性标记（去重），重新计算（用户重开监控）时才重置。

use chrono::{DateTime, FixedOffset};
use serde::Serialize;
use std::collections::HashMap;

use crate::db::monitors::Monitor;
use crate::db::Database;
use crate::domain::{KLineData, Quote};

/// 监控触发事件（`monitor-triggered`）。
#[derive(Debug, Clone, Serialize)]
pub struct MonitorEvent {
    pub id: i64,
    pub code: String,
    pub market: String,
    pub name: String,
    pub reference_price: f64,
    /// "stop_loss" | "take_profit"
    pub trigger_type: String,
    /// 触发价位（止损价或止盈价）
    pub trigger_price: f64,
    pub current_price: f64,
    /// 相对参考价的涨跌幅 %
    pub change_pct: f64,
    pub triggered_at: String,
}

/// ATR 波动止损/止盈的倍数参数。
const ATR_PERIOD: usize = 14;
const STOP_ATR_MULT: f64 = 2.0;
const TAKE_ATR_MULT: f64 = 3.0;

/// 用 ATR 计算量化止损/止盈位。
///
/// - 止损价 = 参考价 − 2×ATR14（控制单笔回撤）
/// - 止盈价 = 参考价 + 3×ATR14（风险收益比约 1:1.5）
///
/// 需要足够长的日 K 才能算出 ATR（至少 ATR_PERIOD+1 根）。
pub fn compute_stop_take(klines: &[KLineData], reference_price: f64) -> Option<(f64, f64)> {
    if !reference_price.is_finite() || reference_price <= 0.0 {
        return None;
    }
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let highs: Vec<f64> = klines.iter().map(|k| k.high).collect();
    let lows: Vec<f64> = klines.iter().map(|k| k.low).collect();
    let atr = crate::quant::indicators::latest_finite(&crate::quant::indicators::atr(
        &highs, &lows, &closes, ATR_PERIOD,
    ))?;
    if atr <= 0.0 {
        return None;
    }
    let stop = reference_price - STOP_ATR_MULT * atr;
    let take = reference_price + TAKE_ATR_MULT * atr;
    if stop <= 0.0 {
        return None;
    }
    Some((stop, take))
}

/// 读取一个 "0"/"1" 形式的布尔设置项。
/// 缺失或读取失败时回落到 `default`，不会因为设置项缺失就静默停用功能。
fn setting_flag(db: &Database, key: &str, default: bool) -> bool {
    match db.get_setting(key) {
        Ok(Some(value)) => value != "0" && !value.eq_ignore_ascii_case("false"),
        Ok(None) => default,
        Err(error) => {
            log::warn!("Failed to read setting {key}: {error}");
            default
        }
    }
}

/// 评估一组刚拿到的行情，触发止损/止盈监控。
/// 仅对「新鲜」的交易时段行情评估（复用 alerts 的判断），缓存回放不会触发。
pub fn evaluate_monitors<F>(
    db: &Database,
    quotes: &[Quote],
    now: DateTime<FixedOffset>,
    mut publish: F,
) where
    F: FnMut(MonitorEvent),
{
    if !crate::alerts::is_alert_time(now) {
        return;
    }

    // AI / 量化智能总开关：关闭后所有自动智能功能停摆。
    if !setting_flag(db, "ai_enabled", true) {
        return;
    }
    // 智能监控子开关。
    if !setting_flag(db, "ai_monitor_enabled", true) {
        return;
    }

    let global_enabled = setting_flag(db, "alerts_enabled", true);
    if !global_enabled { return; }
    let monitors = match db.get_monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            log::warn!("Failed to read monitors: {error}");
            return;
        }
    };

    // 保留重复行情的首次命中语义，与原先逐项 find 一致。
    let mut quote_index = HashMap::with_capacity(quotes.len());
    for quote in quotes {
        quote_index.entry((quote.market.as_str(), quote.code.as_str())).or_insert(quote);
    }
    let now_rfc3339 = now.to_rfc3339();
    for monitor in monitors {
        if !monitor.enabled {
            continue;
        }
        let Some(quote) = quote_index.get(&(monitor.market.as_str(), monitor.code.as_str())) else {
            continue;
        };
        if !crate::alerts::is_fresh_trading_quote(quote, now) {
            continue;
        }
        let price = quote.price;
        if !price.is_finite() || price <= 0.0 {
            continue;
        }

        // 止损：跌破量化止损价（一次性）
        if price <= monitor.stop_price && monitor.last_triggered.as_deref() != Some("stop_loss") {
            if let Err(error) = db.mark_monitor_triggered(monitor.id, "stop_loss") {
                log::warn!("Failed to mark monitor triggered: {error}");
                continue;
            }
            publish(event(
                &monitor,
                "stop_loss",
                monitor.stop_price,
                price,
                &now_rfc3339,
            ));
            continue;
        }

        // 止盈：突破量化止盈价（一次性）
        if price >= monitor.take_price && monitor.last_triggered.as_deref() != Some("take_profit") {
            if let Err(error) = db.mark_monitor_triggered(monitor.id, "take_profit") {
                log::warn!("Failed to mark monitor triggered: {error}");
                continue;
            }
            publish(event(
                &monitor,
                "take_profit",
                monitor.take_price,
                price,
                &now_rfc3339,
            ));
        }
    }
}

fn event(
    monitor: &Monitor,
    trigger_type: &str,
    trigger_price: f64,
    current_price: f64,
    triggered_at: &str,
) -> MonitorEvent {
    MonitorEvent {
        id: monitor.id,
        code: monitor.code.clone(),
        market: monitor.market.clone(),
        name: monitor.name.clone(),
        reference_price: monitor.reference_price,
        trigger_type: trigger_type.to_owned(),
        trigger_price,
        current_price,
        change_pct: (current_price - monitor.reference_price) / monitor.reference_price * 100.0,
        triggered_at: triggered_at.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(close: f64, high: f64, low: f64) -> KLineData {
        KLineData {
            date: String::new(),
            open: close,
            high,
            low,
            close,
            volume: 1_000_000,
            turnover: 0.0,
        }
    }

    /// 生成波动递增的 60 根日 K（用于算出正 ATR）
    fn volatile_bars(n: usize) -> Vec<KLineData> {
        (0..n)
            .map(|i| {
                let c = 100.0 + i as f64;
                bar(c, c + 1.0 + (i % 3) as f64, c - 1.0 - (i % 3) as f64)
            })
            .collect()
    }

    #[test]
    fn compute_stop_take_sets_stop_below_and_take_above_reference() {
        let klines = volatile_bars(60);
        let (stop, take) = compute_stop_take(&klines, 150.0).unwrap();
        assert!(stop < 150.0 && stop > 0.0, "止损应低于参考价：{stop}");
        assert!(take > 150.0, "止盈应高于参考价：{take}");
        // 风险收益比固定为 2:3，故 (take - ref) = 1.5 * (ref - stop)
        assert!((((take - 150.0) / (150.0 - stop)) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn compute_stop_take_requires_enough_data() {
        assert!(compute_stop_take(&volatile_bars(10), 100.0).is_none());
        assert!(compute_stop_take(&[], 100.0).is_none());
    }
}
