// src-tauri/src/commands/monitor.rs
//! 个股监控（量化自动止损/止盈）的命令。
//!
//! 「开启监控」时由后端自动拉日 K、用 ATR 计算止损/止盈位并落库，
//! 用户只需选择股票，无需手填任何参数。

use crate::db::monitors::Monitor;
use crate::db::Database;
use std::sync::Arc;
use tauri::State;

/// 列出全部监控规则。
#[tauri::command]
pub fn get_monitors(db: State<'_, Arc<Database>>) -> Result<Vec<Monitor>, String> {
    db.get_monitors().map_err(|e| e.to_string())
}

/// 开启（或刷新）一只股票的量化监控。
///
/// 后端自动：拉 120 根日 K → 以最新收盘价为参考价 → 用 ATR14 算止损/止盈位 → 落库。
/// 返回落库后的完整监控（含算出的止损/止盈价，供前端展示）。
#[tauri::command]
pub async fn save_monitor(
    db: State<'_, Arc<Database>>,
    code: String,
    market: String,
    name: String,
) -> Result<Monitor, String> {
    let klines = crate::datasource::kline::fetch_daily_kline(&code, 120)
        .await
        .map_err(|e| format!("拉取日K失败：{e}"))?;

    let reference_price = klines
        .last()
        .map(|k| k.close)
        .filter(|c| c.is_finite() && *c > 0.0)
        .ok_or("日K数据为空，无法计算参考价")?;

    let (stop_price, take_price) = crate::monitor::compute_stop_take(&klines, reference_price)
        .ok_or("日K数据不足，无法计算 ATR 止损/止盈位（至少需 15 根）")?;

    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let monitor = Monitor {
        id: 0,
        code,
        market,
        name,
        enabled: true,
        reference_price,
        stop_price,
        take_price,
        last_triggered: None,
        updated_at: now,
    };
    db.upsert_monitor(&monitor)?;
    // 返回落库后的完整记录（含 id）
    let list = db.get_monitors().map_err(|e| e.to_string())?;
    list.into_iter()
        .find(|m| m.code == monitor.code && m.market == monitor.market)
        .ok_or_else(|| "保存后未找到记录".to_string())
}

/// 删除一条监控。
#[tauri::command]
pub fn delete_monitor(
    db: State<'_, Arc<Database>>,
    code: String,
    market: String,
) -> Result<(), String> {
    db.delete_monitor(&code, &market).map_err(|e| e.to_string())
}
