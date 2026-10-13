use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::VecDeque, sync::Mutex};
use tauri::{Emitter, Manager};
use std::sync::Arc;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

const LIMIT: usize = 100;
pub const PENDING_KEY: &str = "important_alerts_pending";
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Snapshot { version: u64, pub entries: VecDeque<Value> }
#[derive(Default)]
pub struct ImportantAlerts(Mutex<Snapshot>);
#[derive(Default)]
pub struct ImportantAlertHotkey(pub Mutex<Option<Shortcut>>);
const VISIBLE_KEY: &str = "important_alerts_visible";
const HOTKEY_KEY: &str = "important_alerts_hotkey";
pub const DEFAULT_HOTKEY: &str = "Alt+W";

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
        let db = handle.state::<std::sync::Arc<crate::db::Database>>();
        let enabled = db.get_setting("important_alerts_enabled").ok().flatten().as_deref() != Some("0");
        if let Some(window) = handle.get_webview_window("important-alerts") {
            let visible = enabled && db.get_setting(VISIBLE_KEY).ok().flatten().as_deref() != Some("0");
            let currently_visible = window.is_visible().unwrap_or(false);
            let result = if visible == currently_visible { Ok(()) } else if visible { window.show() } else { window.hide() };
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
        let mut snapshot: Snapshot = serde_json::from_str(&raw).map_err(|e| format!("未确认提醒记录损坏，请在分类归档中核对：{e}"))?;
        if snapshot.entries.len() > LIMIT || snapshot.entries.iter().any(|row| !row["id"].is_string() || !row["title"].is_string() || !row["body"].is_string()) { return Err("未确认提醒记录格式无效".into()); }
        snapshot.entries.retain(crate::notifications::is_current_notice);
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

#[tauri::command]
pub fn toggle_important_alerts(app: tauri::AppHandle) -> Result<(), String> {
    let db = app.state::<Arc<crate::db::Database>>();
    if db.get_setting("important_alerts_enabled").map_err(|e| e.to_string())?.as_deref() == Some("0") {
        return Err("请先在悬浮提醒设置中开启重要提醒窗口".into());
    }
    let window = app.get_webview_window("important-alerts").ok_or("重要提醒窗口不可用")?;
    let visible = window.is_visible().map_err(|e| e.to_string())?;
    db.set_setting(VISIBLE_KEY, if visible { "0" } else { "1" }).map_err(|e| e.to_string())?;
    broadcast(&app)
}
#[tauri::command]
pub fn show_important_alerts(app: tauri::AppHandle) -> Result<(), String> {
    app.state::<Arc<crate::db::Database>>().set_setting(VISIBLE_KEY, "1").map_err(|e| e.to_string())?;
    broadcast(&app)
}
#[tauri::command]
pub fn set_important_alerts_hotkey(app: tauri::AppHandle, hotkey: String) -> Result<(), String> {
    let candidate = hotkey.parse::<Shortcut>().map_err(|e| format!("快捷键无效：{e}"))?;
    let state = app.state::<ImportantAlertHotkey>();
    let mut current = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let db = app.state::<Arc<crate::db::Database>>();
    if current.as_ref() == Some(&candidate) { return db.set_setting(HOTKEY_KEY, &hotkey).map_err(|e| e.to_string()); }
    let shortcuts = app.global_shortcut();
    if shortcuts.is_registered(candidate) { return Err("该快捷键已被行情窗口、分组或其他功能使用".into()); }
    let previous = db.get_setting(HOTKEY_KEY).map_err(|e| e.to_string())?;
    shortcuts.register(candidate).map_err(|e| format!("快捷键被占用，原设置保留：{e}"))?;
    if let Err(error) = db.set_setting(HOTKEY_KEY, &hotkey) {
        let _ = shortcuts.unregister(candidate);
        return Err(error.to_string());
    }
    if let Some(old) = *current {
        if let Err(error) = shortcuts.unregister(old) {
            let _ = db.set_setting(HOTKEY_KEY, previous.as_deref().unwrap_or(DEFAULT_HOTKEY));
            let _ = shortcuts.unregister(candidate);
            return Err(format!("原快捷键注销失败，未切换：{error}"));
        }
    }
    *current = Some(candidate);
    Ok(())
}
pub fn restore_hotkey(app: &tauri::AppHandle) {
    let db = app.state::<Arc<crate::db::Database>>();
    let hotkey = db.get_setting(HOTKEY_KEY).ok().flatten().unwrap_or_else(|| DEFAULT_HOTKEY.into());
    if let Err(error) = set_important_alerts_hotkey(app.clone(), hotkey) { log::warn!("重要提醒快捷键注册失败：{error}"); }
}
pub fn handle_hotkey(app: &tauri::AppHandle, shortcut: &Shortcut) -> bool {
    let matches = app.state::<ImportantAlertHotkey>().0.lock().unwrap_or_else(|e| e.into_inner()).as_ref() == Some(shortcut);
    if !matches { return false; }
    if let Err(error) = toggle_important_alerts(app.clone()) { log::warn!("重要提醒窗口切换失败：{error}"); }
    true
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
