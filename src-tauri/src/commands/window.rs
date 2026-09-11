use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use std::sync::Arc;

use crate::db::Database;

#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;
    let was_visible = window.is_visible().map_err(|e| e.to_string())?;
    if !was_visible {
        let db = app.state::<Arc<Database>>();
        apply_main_window_size(&app, &db)?;
    }
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct MainWindowSize {
    pub width: u32,
    pub height: u32,
    pub remember: bool,
}

impl Default for MainWindowSize {
    fn default() -> Self { Self { width: 1000, height: 680, remember: false } }
}

pub fn apply_main_window_size(app: &AppHandle, db: &Database) -> Result<(), String> {
    let config = db.get_setting("main_window_size").map_err(|e| e.to_string())?
        .and_then(|raw| serde_json::from_str::<MainWindowSize>(&raw).ok()).unwrap_or_default();
    if config.remember { return Ok(()); }
    apply_size(app, config.width, config.height)
}

fn apply_size(app: &AppHandle, width: u32, height: u32) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("主窗口不存在")?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let (max_w, max_h) = monitor.as_ref().map(|m| {
        ((m.size().width as f64 / scale).floor() as u32, (m.size().height as f64 / scale).floor() as u32)
    }).unwrap_or((width, height));
    window.unmaximize().map_err(|e| e.to_string())?;
    window.set_size(tauri::LogicalSize::new(width.min(max_w.saturating_sub(20).max(320)), height.min(max_h.saturating_sub(80).max(240)))).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_main_window_size(app: AppHandle, db: State<'_, Arc<Database>>, config: MainWindowSize) -> Result<(), String> {
    if !(640..=7680).contains(&config.width) || !(480..=4320).contains(&config.height) {
        return Err("宽度范围 640–7680，高度范围 480–4320（逻辑像素）".into());
    }
    let window = app.get_webview_window("main").ok_or("主窗口不存在")?;
    let old_size = window.inner_size().map_err(|e| e.to_string())?;
    apply_size(&app, config.width, config.height)?;
    let value = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    if let Err(error) = db.set_setting("main_window_size", &value) {
        let _ = window.set_size(old_size);
        return Err(error.to_string());
    }
    Ok(())
}

/// 设置/更改悬浮窗（ticker）的全局快捷键。
/// 解析失败、注册失败、DB 写入失败都会返回错误给前端。
#[tauri::command]
pub fn set_ticker_hotkey(
    app: AppHandle,
    db: State<'_, Arc<Database>>,
    hotkey: String,
) -> Result<(), String> {
    let new_sc = hotkey
        .parse::<Shortcut>()
        .map_err(|e| e.to_string())?;

    let state = app.state::<crate::HotkeyState>();
    let mut cur = state.0.lock().unwrap_or_else(|e| e.into_inner());
    if cur.as_ref() == Some(&new_sc) {
        return db.set_setting("ticker_hotkey", &hotkey).map_err(|e| e.to_string());
    }
    let previous_value = db.get_setting("ticker_hotkey").map_err(|e| e.to_string())?;
    let gs = app.global_shortcut();
    // 先验证新键可注册，冲突时保留仍可用的旧键。
    gs.register(new_sc).map_err(|e| e.to_string())?;
    if let Err(error) = db.set_setting("ticker_hotkey", &hotkey) {
        if let Err(rollback) = gs.unregister(new_sc) {
            log::error!("撤销候选快捷键失败: {}", rollback);
        }
        return Err(error.to_string());
    }
    if let Some(old) = *cur {
        if let Err(error) = gs.unregister(old) {
            let rollback_key = previous_value.unwrap_or_else(|| old.to_string());
            let db_result = db.set_setting("ticker_hotkey", &rollback_key);
            let unregister_result = gs.unregister(new_sc);
            if let Err(e) = db_result { log::error!("恢复快捷键设置失败: {}", e); }
            if let Err(e) = unregister_result { log::error!("注销候选快捷键失败: {}", e); }
            return Err(format!("旧快捷键注销失败，未切换：{}", error));
        }
    }
    *cur = Some(new_sc);
    log::info!("[hotkey] set to '{}'", hotkey);
    Ok(())
}

/// 设置悬浮窗（ticker）整体透明度，范围为 5–100（百分比）。
/// 值会被 clamp 到 [5, 100]，映射为 Windows 分层窗口的 alpha (0–255) 后生效。
#[tauri::command]
pub fn set_ticker_opacity(
    app: AppHandle,
    db: State<'_, Arc<Database>>,
    opacity: u32,
) -> Result<(), String> {
    let opacity = opacity.clamp(5, 100);
    let window = app
        .get_webview_window("ticker")
        .ok_or_else(|| "Ticker window not found".to_string())?;
    let alpha = ((opacity as f32 / 100.0) * 255.0).round() as u8;
    crate::apply_ticker_opacity(&window, alpha)?;
    db.set_setting("ticker_opacity", &opacity.to_string())
        .map_err(|e| e.to_string())?;
    log::info!("[ticker] opacity set to {}%", opacity);
    Ok(())
}

/// 根据当前可见行情行数调整悬浮窗高度，最多占用显示器可用高度。
#[tauri::command]
pub fn resize_ticker_window(app: AppHandle, visible_rows: u32) -> Result<(), String> {
    let window = app
        .get_webview_window("ticker")
        .ok_or_else(|| "Ticker window not found".to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let max_height = monitor
        .as_ref()
        .map(|m| (m.size().height as f64 / scale).floor() as u32)
        .unwrap_or(600)
        .saturating_sub(40)
        .max(38);
    let rows = visible_rows.clamp(1, 200);
    let height = (rows.saturating_mul(19)).max(38).min(max_height);
    window
        .set_size(tauri::LogicalSize::new(230_u32, height))
        .map_err(|e| e.to_string())?;

    if let (Some(monitor), Ok(position), Ok(size)) = (
        monitor,
        window.outer_position(),
        window.outer_size(),
    ) {
        let origin = monitor.position();
        let bounds = monitor.size();
        let max_x = origin.x + bounds.width as i32 - size.width as i32;
        let max_y = origin.y + bounds.height as i32 - size.height as i32;
        let x = position.x.clamp(origin.x, max_x.max(origin.x));
        let y = position.y.clamp(origin.y, max_y.max(origin.y));
        if x != position.x || y != position.y {
            window
                .set_position(tauri::PhysicalPosition::new(x, y))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
