use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

/// Keep navigation until the main window consumes it, including during startup.
#[derive(Default)]
pub struct PendingNavigation(pub Mutex<Option<String>>);

pub const QUICK_DESTINATIONS: &[(&str, &str)] = &[
    ("research", "研究中心"),
    ("mainline", "市场主线"),
    ("screener", "市场筛选"),
    ("simulation", "模拟账户"),
    ("settings", "设置"),
];

fn valid_destination(value: &str) -> bool {
    QUICK_DESTINATIONS.iter().any(|(id, _)| *id == value)
        || matches!(value, "settings:market" | "settings:alerts" | "settings:ai"
            | "settings:ticker" | "settings:appearance" | "settings:system")
}

pub fn destination(menu_id: &str) -> Option<&str> {
    menu_id.strip_prefix("open:").filter(|value| valid_destination(value))
}

/// Shared by tray shortcuts and the floating ticker's display-settings shortcut.
#[tauri::command]
pub fn open_navigation(app: tauri::AppHandle, destination: String) -> Result<(), String> {
    if !valid_destination(&destination) { return Err("未知快捷入口".into()); }
    *app.state::<PendingNavigation>().0.lock().unwrap_or_else(|e| e.into_inner()) = Some(destination);
    crate::commands::window::show_main_window(app.clone())?;
    app.emit_to("main", "quick-navigation", ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn take_pending_navigation(state: State<'_, PendingNavigation>) -> Option<String> {
    state.0.lock().unwrap_or_else(|e| e.into_inner()).take()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_destinations_are_explicit_and_consumed_once() {
        assert_eq!(QUICK_DESTINATIONS.len(), 5);
        assert_eq!(QUICK_DESTINATIONS.iter().filter(|(id, _)| id.starts_with("settings")).count(), 1);
        for (id, _) in QUICK_DESTINATIONS {
            let menu_id = format!("open:{id}");
            assert_eq!(destination(&menu_id), Some(*id));
        }
        assert_eq!(destination("open:settings:ticker"), Some("settings:ticker"));
        assert_eq!(destination("open:settings:unknown"), None);
        assert_eq!(destination("quit"), None);
        let state = PendingNavigation::default();
        *state.0.lock().unwrap() = Some("settings:ticker".into());
        assert_eq!(state.0.lock().unwrap().take().as_deref(), Some("settings:ticker"));
        assert!(state.0.lock().unwrap().take().is_none());
    }
}
