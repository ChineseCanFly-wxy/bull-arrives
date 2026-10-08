use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::VecDeque, sync::Mutex};
use tauri::{Emitter, Manager};

const LIMIT: usize = 100;
pub const PENDING_KEY: &str = "important_alerts_pending";
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Snapshot { version: u64, pub entries: VecDeque<Value> }
#[derive(Default)]
pub struct ImportantAlerts(Mutex<Snapshot>);

fn insert(state: &mut Snapshot, payload: Value) -> Result<(), String> {
    if state.entries.iter().any(|row| row["id"] == payload["id"]) { return Ok(()); }
    if state.entries.len() >= LIMIT { return Err("重要提醒已积压100条，请确认已读；新消息仍保留在分类记录中".into()); }
    state.entries.push_front(payload);
    state.version += 1;
    Ok(())
}
fn broadcast(app: &tauri::AppHandle) -> Result<(), String> {
    let snapshot = get_important_alerts(app.clone());
    app.emit_to("important-alerts", "important-alerts-changed", &snapshot).map_err(|e| e.to_string())?;
    if app.get_webview_window("important-alerts").is_none() { return Err("重要提醒窗口不可用".into()); }
    let handle = app.clone();
    // 在窗口线程读取最新状态，避免并发确认与新提醒把窗口错误隐藏。
    app.run_on_main_thread(move || {
        let state = get_important_alerts(handle.clone());
        let db = handle.state::<std::sync::Arc<crate::db::Database>>();
        let enabled = db.get_setting("important_alerts_enabled").ok().flatten().as_deref() != Some("0");
        if let Some(window) = handle.get_webview_window("important-alerts") {
            let visible = enabled && !state.entries.is_empty();
            let result = if visible { window.show() } else { window.hide() };
            if let Err(error) = result { log::warn!("重要提醒窗口显示失败：{error}"); }
            if visible {
                crate::apply_tool_window_style(&window);
                let opacity = db.get_setting("important_alerts_opacity").ok().flatten().and_then(|s| s.parse::<u32>().ok()).unwrap_or(95).clamp(5,100);
                if let Err(error) = crate::apply_ticker_opacity(&window, ((opacity as f64 / 100.0) * 255.0).round() as u8) { log::warn!("应用重要提醒透明度失败：{error}"); }
            }
        }
    }).map_err(|e| e.to_string())
}
fn save(app: &tauri::AppHandle, snapshot: &Snapshot) -> Result<(), String> {
    app.state::<std::sync::Arc<crate::db::Database>>().set_setting(PENDING_KEY, &serde_json::to_string(snapshot).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
pub fn restore(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(raw) = app.state::<std::sync::Arc<crate::db::Database>>().get_setting(PENDING_KEY).map_err(|e| e.to_string())? {
        let snapshot: Snapshot = serde_json::from_str(&raw).map_err(|e| format!("未确认提醒记录损坏，请在分类归档中核对：{e}"))?;
        if snapshot.entries.len() > LIMIT || snapshot.entries.iter().any(|row| !row["id"].is_string() || !row["title"].is_string() || !row["body"].is_string()) { return Err("未确认提醒记录格式无效".into()); }
        *app.state::<ImportantAlerts>().0.lock().unwrap_or_else(|e| e.into_inner()) = snapshot;
    }
    Ok(())
}
pub fn enqueue(app: &tauri::AppHandle, payload: Value) -> Result<(), String> {
    {
        let state = app.state::<ImportantAlerts>();
        let mut inner = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = inner.clone();
        insert(&mut next, payload)?;
        save(app, &next)?;
        *inner = next;
    }
    broadcast(app)
}
#[tauri::command]
pub fn get_important_alerts(app: tauri::AppHandle) -> Snapshot {
    app.state::<ImportantAlerts>().0.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
#[tauri::command]
pub fn dismiss_important_alert(app: tauri::AppHandle, id: String) -> Result<(), String> {
    {
        let state = app.state::<ImportantAlerts>();
        let mut inner = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = inner.clone();
        next.entries.retain(|row| row["id"].as_str() != Some(&id));
        if inner.entries.len() == next.entries.len() { return Ok(()); }
        next.version += 1;
        save(&app, &next)?;
        *inner = next;
    }
    broadcast(&app)
}
#[tauri::command]
pub fn view_important_alert(app: tauri::AppHandle, id: String) -> Result<(), String> {
    if !app.state::<ImportantAlerts>().0.lock().unwrap_or_else(|e| e.into_inner()).entries.iter().any(|row| row["id"].as_str() == Some(&id)) {
        return Err("该提醒已确认，请在分类记录中查看".into());
    }
    crate::commands::window::show_main_window(app.clone())?;
    app.emit("notification-open-history", &id).map_err(|e| e.to_string())
}
pub fn refresh_visibility(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if enabled { broadcast(app) }
    else { app.get_webview_window("important-alerts").ok_or("重要提醒窗口不可用")?.hide().map_err(|e| e.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn important_alerts_preserve_unread_items_deduplicate_and_reject_overflow() {
        let mut state = Snapshot::default();
        for id in 0..LIMIT { insert(&mut state, json!({"id":id.to_string()})).unwrap(); }
        let version = state.version;
        insert(&mut state, json!({"id":"0"})).unwrap();
        assert_eq!(state.version, version);
        assert!(insert(&mut state, json!({"id":"new"})).is_err());
        assert_eq!(state.entries.len(), LIMIT);
        assert_eq!(state.entries.back().unwrap()["id"], "0");
        state.entries.retain(|row| row["id"] != "0");
        insert(&mut state, json!({"id":"new"})).unwrap();
        assert_eq!(state.entries.front().unwrap()["id"], "new");
        let restored: Snapshot = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(restored.version, state.version);
        assert_eq!(restored.entries, state.entries);
    }
}
