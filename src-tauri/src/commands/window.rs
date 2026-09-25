use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

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
    fn default() -> Self {
        Self {
            width: 1000,
            height: 680,
            remember: false,
        }
    }
}

pub fn apply_main_window_size(app: &AppHandle, db: &Database) -> Result<(), String> {
    let config = db
        .get_setting("main_window_size")
        .map_err(|e| e.to_string())?
        .and_then(|raw| serde_json::from_str::<MainWindowSize>(&raw).ok())
        .unwrap_or_default();
    if config.remember {
        return Ok(());
    }
    apply_size(app, config.width, config.height)
}

fn apply_size(app: &AppHandle, width: u32, height: u32) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("主窗口不存在")?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let (max_w, max_h) = monitor
        .as_ref()
        .map(|m| {
            (
                (m.size().width as f64 / scale).floor() as u32,
                (m.size().height as f64 / scale).floor() as u32,
            )
        })
        .unwrap_or((width, height));
    window.unmaximize().map_err(|e| e.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(
            width.min(max_w.saturating_sub(20).max(320)),
            height.min(max_h.saturating_sub(80).max(240)),
        ))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_main_window_size(
    app: AppHandle,
    db: State<'_, Arc<Database>>,
    config: MainWindowSize,
) -> Result<(), String> {
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
    let new_sc = hotkey.parse::<Shortcut>().map_err(|e| e.to_string())?;

    let state = app.state::<crate::HotkeyState>();
    let mut cur = state.0.lock().unwrap_or_else(|e| e.into_inner());
    if cur.as_ref() == Some(&new_sc) {
        return db
            .set_setting("ticker_hotkey", &hotkey)
            .map_err(|e| e.to_string());
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
            if let Err(e) = db_result {
                log::error!("恢复快捷键设置失败: {}", e);
            }
            if let Err(e) = unregister_result {
                log::error!("注销候选快捷键失败: {}", e);
            }
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
    // 即使悬浮窗暂时不存在，也要先保存用户选择；窗口重新创建时会从数据库恢复。
    db.set_setting("ticker_opacity", &opacity.to_string())
        .map_err(|e| e.to_string())?;
    if let Some(window) = app.get_webview_window("ticker") {
        let alpha = ((opacity as f32 / 100.0) * 255.0).round() as u8;
        crate::apply_ticker_opacity(&window, alpha)
            .map_err(|e| format!("透明度已保存，但当前悬浮窗应用失败：{e}"))?;
    }
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

    if let (Some(monitor), Ok(position), Ok(size)) =
        (monitor, window.outer_position(), window.outer_size())
    {
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

/// 快速自选面板的尺寸（逻辑像素），与 `tauri.conf.json` 里的窗口配置保持一致。
const QUICK_ADD_WIDTH: u32 = 320;
const QUICK_ADD_HEIGHT: u32 = 300;
/// 面板与悬浮窗之间的间隙（物理像素）。
const QUICK_ADD_GAP: i32 = 8;

/// 显示「快速自选」小窗：不用打开主界面就能搜索添加自选股、并直接删掉。
///
/// 位置策略是「贴着悬浮窗放」——悬浮窗通常在屏幕右侧偏下，所以优先摆在它正上方；
/// 上方空间不够就改到下方；都放不下时退回上方，最后由 clamp 保证不会跑出显示器。
/// 面板必须能拿到键盘焦点（要输入搜索词），因此**不能**加 `WS_EX_NOACTIVATE`，
/// 只去掉任务栏图标（`WS_EX_TOOLWINDOW`）。
#[tauri::command]
pub fn open_ticker_quick_add(app: AppHandle) -> Result<(), String> {
    let quick = app
        .get_webview_window("ticker-quick")
        .ok_or_else(|| "快速自选窗口不存在".to_string())?;

    if let Some(ticker) = app.get_webview_window("ticker") {
        if let (Ok(t_pos), Ok(t_size)) = (ticker.outer_position(), ticker.outer_size()) {
            let monitor = ticker.current_monitor().ok().flatten();
            let (mon_x, mon_y, mon_w, mon_h) = monitor
                .as_ref()
                .map(|m| {
                    let origin = m.position();
                    let bounds = m.size();
                    (
                        origin.x,
                        origin.y,
                        bounds.width as i32,
                        bounds.height as i32,
                    )
                })
                .unwrap_or((0, 0, 1920, 1080));

            // 面板尺寸按它自己的缩放比换算成物理像素，才能和悬浮窗的位置直接比较。
            let scale = quick.scale_factor().unwrap_or(1.0);
            let panel_w = (QUICK_ADD_WIDTH as f64 * scale).round() as i32;
            let panel_h = (QUICK_ADD_HEIGHT as f64 * scale).round() as i32;
            let ticker_bottom = t_pos.y + t_size.height as i32;

            let above = t_pos.y - panel_h - QUICK_ADD_GAP;
            let below = ticker_bottom + QUICK_ADD_GAP;
            let fits_above = above >= mon_y;
            let fits_below = below + panel_h <= mon_y + mon_h;
            let target_y = if fits_above || !fits_below {
                above
            } else {
                below
            };

            // 右对齐悬浮窗（面板比悬浮窗宽，左对齐容易戳出屏幕右侧）。
            let target_x = t_pos.x + t_size.width as i32 - panel_w;

            let max_x = (mon_x + mon_w - panel_w).max(mon_x);
            let max_y = (mon_y + mon_h - panel_h).max(mon_y);
            let x = target_x.clamp(mon_x, max_x);
            let y = target_y.clamp(mon_y, max_y);
            let _ = quick.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }

    quick.show().map_err(|e| e.to_string())?;
    let _ = quick.set_always_on_top(true);
    let _ = quick.set_skip_taskbar(true);
    // 只去掉任务栏图标；这里刻意不加非激活样式，否则输入框拿不到键盘焦点。
    crate::apply_tool_window_style(&quick);
    quick.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// 隐藏「快速自选」小窗（Esc / 失焦 / 关闭按钮都走这里）。
#[tauri::command]
pub fn close_ticker_quick_add(app: AppHandle) -> Result<(), String> {
    let quick = app
        .get_webview_window("ticker-quick")
        .ok_or_else(|| "快速自选窗口不存在".to_string())?;
    quick.hide().map_err(|e| e.to_string())
}
