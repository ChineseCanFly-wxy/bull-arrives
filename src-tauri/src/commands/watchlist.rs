use tauri::{Emitter, State};
use std::sync::Arc;
use crate::db::{Database, WatchItem, PriceAlert};
use crate::datasource::DataSourceManager;

#[tauri::command]
pub fn get_watchlist(db: State<'_, Arc<Database>>) -> Result<Vec<WatchItem>, String> {
    db.get_watchlist().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_watch(
    app_handle: tauri::AppHandle,
    db: State<'_, Arc<Database>>,
    code: String,
    market: String,
    name: String,
    group_id: Option<i64>,
    manager: State<'_, Arc<DataSourceManager>>,
) -> Result<(), String> {
    db.add_watch_to_group(&code, &market, &name, group_id.unwrap_or(0))?;
    manager.invalidate_requests();
    let _ = app_handle.emit("watchlist-changed", ());
    Ok(())
}

#[tauri::command]
pub fn remove_watch(
    app_handle: tauri::AppHandle,
    manager: State<'_, Arc<DataSourceManager>>,
    db: State<'_, Arc<Database>>,
    code: String,
    market: String,
) -> Result<(), String> {
    db.remove_watch(&code, &market)
        .map_err(|e| e.to_string())?;
    manager.invalidate_requests();
    let _ = app_handle.emit("watchlist-changed", ());
    Ok(())
}

#[tauri::command]
pub fn reorder_watch(
    app_handle: tauri::AppHandle,
    db: State<'_, Arc<Database>>,
    ids: Vec<i64>,
) -> Result<(), String> {
    db.reorder_watch(&ids).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("watchlist-changed", ());
    Ok(())
}

#[tauri::command]
pub fn move_watch_top(
    app_handle: tauri::AppHandle,
    db: State<'_, Arc<Database>>,
    id: i64,
) -> Result<(), String> {
    db.move_current_group_item(id, "top")?;
    let _ = app_handle.emit("watchlist-changed", ());
    Ok(())
}

#[tauri::command]
pub fn move_watch_up(
    app_handle: tauri::AppHandle,
    db: State<'_, Arc<Database>>,
    id: i64,
) -> Result<(), String> {
    db.move_current_group_item(id, "up")?;
    let _ = app_handle.emit("watchlist-changed", ());
    Ok(())
}

#[tauri::command]
pub fn move_watch_down(
    app_handle: tauri::AppHandle,
    db: State<'_, Arc<Database>>,
    id: i64,
) -> Result<(), String> {
    db.move_current_group_item(id, "down")?;
    let _ = app_handle.emit("watchlist-changed", ());
    Ok(())
}

#[tauri::command]
pub fn get_price_alerts(db: State<'_, Arc<Database>>, code: String, market: String) -> Result<Vec<PriceAlert>, String> {
    db.get_price_alerts(&code, &market).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_price_alert(app_handle: tauri::AppHandle, db: State<'_, Arc<Database>>, alert: PriceAlert) -> Result<(), String> {
    validate_price_alert(&alert)?;
    if !db.watch_exists(&alert.code, &alert.market).map_err(|e| e.to_string())? {
        return Err("只能为自选股保存提醒".to_string());
    }
    db.upsert_price_alert(&alert).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("price-alerts-changed", ());
    Ok(())
}

#[tauri::command]
pub fn delete_price_alert(app_handle: tauri::AppHandle, db: State<'_, Arc<Database>>, code: String, market: String, alert_type: String) -> Result<(), String> {
    db.delete_price_alert(&code, &market, &alert_type).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("price-alerts-changed", ());
    Ok(())
}

const MAX_ALERT_COOLDOWN_MINUTES: i64 = 24 * 60;

fn validate_price_alert(alert: &PriceAlert) -> Result<(), String> {
    if !matches!(alert.alert_type.as_str(), "change_pct" | "fixed_price") {
        return Err("提醒类型无效".to_string());
    }
    if !matches!(alert.repeat_mode.as_str(), "daily" | "crossing" | "cooldown") {
        return Err("重复模式无效".to_string());
    }
    if !alert.threshold.is_finite() {
        return Err("提醒阈值必须是有限数".to_string());
    }
    if alert.alert_type == "fixed_price" && alert.threshold <= 0.0 {
        return Err("目标价格必须大于 0".to_string());
    }
    if alert.alert_type == "change_pct" && alert.threshold == 0.0 {
        return Err("涨跌幅阈值不能为 0".to_string());
    }
    if !(0..=MAX_ALERT_COOLDOWN_MINUTES).contains(&alert.cooldown_minutes) {
        return Err(format!("冷却时间必须在 0 到 {MAX_ALERT_COOLDOWN_MINUTES} 分钟之间"));
    }
    if alert.repeat_mode == "cooldown" && alert.cooldown_minutes == 0 {
        return Err("冷却模式的冷却时间必须大于 0".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn search_stocks(
    manager: State<'_, Arc<DataSourceManager>>,
    keyword: String,
) -> Result<Vec<crate::domain::StockBrief>, String> {
    // Stock search is metadata discovery, so it remains available outside
    // quote polling sessions. Live quotes/details keep their own time gate.
    // ── Tier 1: Sina suggest API (name + fuzzy code search) ──
    match crate::datasource::search::suggest_search(&keyword).await {
        Ok(results) if !results.is_empty() => {
            return Ok(results);
        }
        Ok(_) => log::info!("Sina suggest returned empty for '{}', trying Tencent", keyword),
        Err(e) => log::warn!("Sina suggest failed: {}, falling back to Tencent", e),
    }

    // ── Tier 2: Tencent smartbox API (name + fuzzy code search) ──
    match crate::datasource::search::tencent_suggest_search(&keyword).await {
        Ok(results) if !results.is_empty() => {
            return Ok(results);
        }
        Ok(_) => log::info!("Tencent smartbox returned empty for '{}', falling back to DataSource", keyword),
        Err(e) => log::warn!("Tencent smartbox failed: {}, falling back to DataSource", e),
    }

    // ── Tier 3: DataSource-based exact-code search ──
    let mut results: Vec<crate::domain::StockBrief> = Vec::new();
    let active_name = if let Some(source) = manager.active_source() {
        match source.search(&keyword, "CN").await {
            Ok(r) => results = r,
            Err(e) => log::warn!("Search via {} failed: {}", source.name(), e),
        }
        source.name().to_string()
    } else {
        String::new()
    };

    if results.is_empty() {
        for (name, source) in manager.all_sources() {
            if name != active_name {
                match source.search(&keyword, "CN").await {
                    Ok(fb_results) if !fb_results.is_empty() => {
                        results = fb_results;
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("Fallback search via {} failed: {}", name, e),
                }
            }
        }
    }

    Ok(results)
}
