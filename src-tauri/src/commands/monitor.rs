// src-tauri/src/commands/monitor.rs
//! 个股监控（量化自动止损/止盈）的命令。
//!
//! 「开启监控」时由后端自动拉日 K、用 ATR 计算止损/止盈位并落库，
//! 用户只需选择股票，无需手填任何参数。
//!
//! # 「停止」与「删除」是两件事
//!
//! - `set_monitor_enabled(false)` —— **停止**：记录留着，止损/止盈位与触发状态都不动，
//!   只是不再参与触发判断；恢复后立刻按原价位继续。用户暂时不想被这只票打扰时用它。
//! - `delete_monitor` —— **删除**：这只票不再监控了。
//!
//! 不能用「删除 + 重新开启」代替停止：`save_monitor` 会用**当时的收盘价**重算参考价，
//! 止损/止盈位会整体挪位，触发状态也被清空 —— 那是换了一套规则，不是暂停。

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
///
/// # `enabled` 参数
///
/// - 这只股票**还没有**记录：忽略该参数，一律从「监控中」开始（用户点的是「开启监控」）。
/// - **已有**记录：`None` = 保持原有开关状态。这是「刷新」路径 ——
///   重新算价位不该顺手把用户停掉的监控偷偷打开。
/// - `Some(true / false)` = 用户显式要求开或停。
#[tauri::command]
pub async fn save_monitor(
    db: State<'_, Arc<Database>>,
    code: String,
    market: String,
    name: String,
    enabled: Option<bool>,
) -> Result<Monitor, String> {
    let klines = crate::datasource::kline::fetch_history(&db, &code, Some(120), true)
        .await
        .map_err(|e| format!("拉取日K失败：{e}"))?
        .klines;

    let reference_price = klines
        .last()
        .map(|k| k.close)
        .filter(|c| c.is_finite() && *c > 0.0)
        .ok_or("日K数据为空，无法计算参考价")?;

    let (stop_price, take_price) = crate::monitor::compute_stop_take(&klines, reference_price)
        .ok_or("日K数据不足，无法计算 ATR 止损/止盈位（至少需 15 根）")?;

    // 已有记录时沿用它的开关状态（除非调用方显式指定）
    let existing = db
        .get_monitors()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| m.code == code && m.market == market);
    let enabled = match (existing, enabled) {
        (Some(m), None) => m.enabled,
        (_, explicit) => explicit.unwrap_or(true),
    };

    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let monitor = Monitor {
        id: 0,
        code,
        market,
        name,
        enabled,
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

/// 停止 / 恢复一条监控（保留规则，只是不让它触发）。
///
/// 只翻 `enabled` 开关，**不动**参考价、止损/止盈位与触发状态 —— 恢复后立刻按原价位继续判断。
/// 没有对应记录时返回可读的错误，而不是静默成功。
#[tauri::command]
pub fn set_monitor_enabled(
    db: State<'_, Arc<Database>>,
    code: String,
    market: String,
    enabled: bool,
) -> Result<(), String> {
    let changed = db
        .set_monitor_enabled(&code, &market, enabled)
        .map_err(|e| e.to_string())?;
    if !changed {
        return Err(format!(
            "没有找到 {code} 的监控规则，无法{}",
            if enabled { "恢复" } else { "停止" }
        ));
    }
    Ok(())
}
