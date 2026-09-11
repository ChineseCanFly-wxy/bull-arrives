pub mod domain;
pub mod db;
pub mod datasource;
pub mod cache;
pub mod commands;
pub mod notifications;
pub mod notification_identity;
pub mod desktop_toast;
pub mod alerts;
pub mod group_hotkeys;

use std::fs::File;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use simplelog::{CombinedLogger, WriteLogger, TermLogger, LevelFilter, Config, TerminalMode, ColorChoice};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, Runtime,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use db::Database;
use datasource::DataSourceManager;
use cache::QuoteCache;

/// Windows-only utility: add WS_EX_TOOLWINDOW to a window's extended style.
/// This permanently hides the window from the taskbar (survives Explorer
/// restarts), unlike the COM-based ITaskbarList::DeleteTab approach used by
/// Tauri's `set_skip_taskbar`.
#[cfg(target_os = "windows")]
mod windows_util {
    use std::ffi::c_void;
    type HWND = *mut c_void;

    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOOLWINDOW: isize = 0x80;
    const WS_EX_NOACTIVATE: isize = 0x08000000;
    const WS_EX_LAYERED: isize = 0x00080000;

    const LWA_ALPHA: u32 = 0x0002;

    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    extern "system" {
        fn GetWindowLongPtrW(hwnd: HWND, nIndex: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: HWND, nIndex: i32, dwNewLong: isize) -> isize;
        fn SetWindowPos(
            hwnd: HWND,
            hwndInsertAfter: HWND,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            uFlags: u32,
        ) -> i32;
        fn SetLayeredWindowAttributes(
            hwnd: HWND,
            cr_key: u32,
            alpha: u8,
            flags: u32,
        ) -> i32;
    }

    /// Set WS_EX_TOOLWINDOW on a window identified by its raw HWND.
    /// Idempotent — skips if the style is already set.
    pub unsafe fn set_tool_window(hwnd: isize) {
        let hwnd_ptr = hwnd as HWND;
        let ex_style = GetWindowLongPtrW(hwnd_ptr, GWL_EXSTYLE);
        if ex_style == 0 {
            log::warn!("[ticker] GetWindowLongPtrW returned 0 — skipping WS_EX_TOOLWINDOW");
            return;
        }
        if ex_style & WS_EX_TOOLWINDOW != 0 {
            return; // already applied
        }
        SetWindowLongPtrW(hwnd_ptr, GWL_EXSTYLE, ex_style | WS_EX_TOOLWINDOW);
        SetWindowPos(
            hwnd_ptr,
            std::ptr::null_mut(),
            0, 0, 0, 0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        log::info!("[ticker] WS_EX_TOOLWINDOW applied — permanently hidden from taskbar");
    }

    pub unsafe fn set_nonactivating_tool_window(hwnd: isize) {
        let hwnd_ptr = hwnd as HWND;
        let ex_style = GetWindowLongPtrW(hwnd_ptr, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd_ptr, GWL_EXSTYLE, ex_style | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE);
        SetWindowPos(hwnd_ptr, std::ptr::null_mut(), 0, 0, 0, 0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }

    /// Set the overall window opacity (0–255) via a layered window.
    /// 255 = fully opaque, lower = more transparent.  Idempotent.
    pub unsafe fn set_opacity(hwnd: isize, alpha: u8) -> Result<(), String> {
        let hwnd_ptr = hwnd as HWND;
        let ex_style = GetWindowLongPtrW(hwnd_ptr, GWL_EXSTYLE);
        if ex_style == 0 {
            return Err("读取悬浮窗样式失败".into());
        }
        if ex_style & WS_EX_LAYERED == 0 {
            SetWindowLongPtrW(hwnd_ptr, GWL_EXSTYLE, ex_style | WS_EX_LAYERED);
        }
        if SetLayeredWindowAttributes(hwnd_ptr, 0, alpha, LWA_ALPHA) == 0 {
            return Err(format!("设置悬浮窗透明度失败: {}", std::io::Error::last_os_error()));
        }
        log::info!("[ticker] opacity set to {}/255", alpha);
        Ok(())
    }
}

/// Apply WS_EX_TOOLWINDOW to a Tauri window so it stays hidden from the
/// Windows taskbar even after Explorer restarts.  No-op on non-Windows.
fn apply_tool_window_style<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::HasWindowHandle;
        if let Ok(handle) = window.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                unsafe {
                    windows_util::set_tool_window(h.hwnd.get() as isize);
                }
            }
        }
    }
    let _ = window; // suppress unused warning on non-Windows
}

pub(crate) fn apply_nonactivating_tool_window_style<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::HasWindowHandle;
        if let Ok(handle) = window.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                unsafe { windows_util::set_nonactivating_tool_window(h.hwnd.get() as isize); }
            }
        }
    }
    let _ = window;
}

/// Apply overall opacity (0–255) to a Tauri window via WS_EX_LAYERED.
/// No-op on non-Windows.
pub(crate) fn apply_ticker_opacity<R: Runtime>(window: &tauri::WebviewWindow<R>, alpha: u8) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::HasWindowHandle;
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
            return unsafe { windows_util::set_opacity(h.hwnd.get() as isize, alpha) };
        }
        return Err("不支持的原生窗口句柄".into());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, alpha);
        Err("当前平台暂不支持整体窗口透明度".into())
    }
}

/// Holds the currently registered global hotkey (or None if not registered).
/// The global-shortcut plugin calls a single handler for all registered
/// shortcuts; the handler compares the event's shortcut against this value
/// to decide whether to toggle the ticker.
pub struct HotkeyState(pub Mutex<Option<Shortcut>>);

/// Toggle the ticker window (show/hide) and persist the new visibility.
/// Shared by the tray menu and the global hotkey handler.
pub fn toggle_ticker_window<R: Runtime>(app: &tauri::AppHandle<R>, db: &Database) {
    let Some(window) = app.get_webview_window("ticker") else { return };
    let was_visible = window.is_visible().unwrap_or(false);
    if was_visible {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        // Re-hide from taskbar after show
        let _ = window.set_skip_taskbar(true);
        apply_tool_window_style(&window);
        // Re-apply saved opacity after show (layered style may need refresh)
        let ticker_opacity: u32 = db
            .get_setting("ticker_opacity")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);
        let alpha = ((ticker_opacity as f32 / 100.0) * 255.0).round() as u8;
        if let Err(error) = apply_ticker_opacity(&window, alpha) { log::warn!("透明度恢复失败: {}", error); }
        // Try saved position first, fall back to bottom-right
        let mon = window.primary_monitor().ok().flatten();
        let (mon_w, mon_h) = mon
            .as_ref()
            .map(|m| { let s = m.size(); (s.width as i32, s.height as i32) })
            .unwrap_or((1920, 1080));
        let win_size = window.outer_size().unwrap_or(tauri::PhysicalSize::new(
            crate::datasource::TICKER_WIDTH,
            crate::datasource::TICKER_HEIGHT,
        ));
        let tw = win_size.width as i32;
        let th = win_size.height as i32;

        let mut restored = false;
        if let Ok(Some(x)) = db.get_setting("ticker_x") {
            if let Ok(Some(y)) = db.get_setting("ticker_y") {
                if let (Ok(sx), Ok(sy)) = (x.parse::<i32>(), y.parse::<i32>()) {
                    if sx + tw > 0 && sy + th > 0 && sx < mon_w && sy < mon_h {
                        let _ = window.set_position(
                            tauri::PhysicalPosition::new(sx, sy),
                        );
                        restored = true;
                    }
                }
            }
        }
        if !restored {
            let x = (mon_w).saturating_sub(tw + 10);
            let y = (mon_h).saturating_sub(th + 60);
            let _ = window.set_position(
                tauri::PhysicalPosition::new(x, y),
            );
        }
    }
    // Persist the ticker's visibility so it restores the same state on next
    // launch (default = visible).
    let _ = db.set_setting(
        "ticker_visible",
        if was_visible { "0" } else { "1" },
    );
}

/// Runtime flag indicating whether the app is in portable mode
/// (triggered by the presence of `portable.dat` next to the executable).
#[derive(Debug, Clone, Copy)]
pub struct PortableMode(pub bool);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(group_hotkeys::GroupHotkeys::default())
        .manage(notifications::NotificationHistory::default())
        .manage(desktop_toast::DesktopToastState::default())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if group_hotkeys::handle(app, shortcut) { return; }
                    let state = app.state::<HotkeyState>();
                    let is_active = state
                        .0
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|active| active == shortcut)
                        .unwrap_or(false);
                    if !is_active {
                        return;
                    }
                    let db = app.state::<Arc<Database>>().inner().clone();
                    toggle_ticker_window(app, &db);
                })
                .build(),
        )
        .setup(|app| {
            // Data directory:
            // - Portable mode (portable.dat exists next to exe) → <exe_dir>/data/
            // - Normal mode → %APPDATA%/bull-arrives/
            let (app_dir, is_portable) = std::env::current_exe()
                .ok()
                .and_then(|exe| {
                    let marker = exe.with_file_name("portable.dat");
                    marker.exists().then(|| {
                        let dir = exe.parent()
                            .map(|p| p.join("data"))
                            .unwrap_or_else(|| std::path::PathBuf::from("data"));
                        (dir, true)
                    })
                })
                .unwrap_or_else(|| {
                    let dir = dirs::data_dir()
                        .expect("Failed to get system data directory")
                        .join("bull-arrives");
                    (dir, false)
                });

            // Initialize logger — writes to both stderr (dev) and bull-arrives.log (file)
            std::fs::create_dir_all(&app_dir).expect("Failed to create app data directory");
            let log_file = File::create(app_dir.join("bull-arrives.log"))
                .expect("Failed to create log file");
            CombinedLogger::init(vec![
                TermLogger::new(
                    LevelFilter::Info,
                    Config::default(),
                    TerminalMode::Mixed,
                    ColorChoice::Auto,
                ),
                WriteLogger::new(LevelFilter::Info, Config::default(), log_file),
            ])
            .expect("Failed to initialize logger");
            log::info!("Bull Arrives v{} starting", env!("CARGO_PKG_VERSION"));
            log::info!(
                "Data directory: {:?} (portable: {})",
                app_dir, is_portable
            );

            let db = Arc::new(Database::open(app_dir).expect("Failed to open database"));
            log::info!("Database opened successfully");

            // Initialize data source manager (Sina registered first as default)
            let mut ds_manager = DataSourceManager::new();
            ds_manager.register(Box::new(
                crate::datasource::tencent::TencentAdapter::new(),
            ));
            ds_manager.register(Box::new(
                crate::datasource::sina::SinaAdapter::new(),
            ));

            // Restore last used data source from settings.
            // Use set_active_initial to avoid triggering a duplicate wakeup fetch
            // on startup (the scheduler's main loop handles the first fetch).
            if let Ok(Some(active)) = db.get_setting("active_datasource") {
                match ds_manager.set_active_initial(&active) {
                    Ok(()) => log::info!("Restored data source: {}", active),
                    Err(e) => log::warn!("Failed to restore data source '{}': {}", active, e),
                }
            }

            let quote_schedule_enabled = db
                .get_setting("quote_schedule_enabled")
                .ok()
                .flatten()
                .as_deref()
                == Some("1");
            match db.get_setting("quote_schedule") {
                Ok(value) => {
                    if let Err(error) = ds_manager.set_request_policy_json(value.as_deref()) {
                        if quote_schedule_enabled {
                            log::error!("行情时段配置损坏，暂停请求：{}", error);
                            ds_manager.set_request_policy(datasource::market_policy::MarketRequestPolicy::paused());
                        } else {
                            log::warn!("行情时段配置损坏，但时间限制未启用：{}", error);
                        }
                    }
                }
                Err(error) if quote_schedule_enabled => {
                    log::error!("行情时段配置读取失败，暂停请求：{}", error);
                    ds_manager.set_request_policy(datasource::market_policy::MarketRequestPolicy::paused());
                }
                Err(error) => log::warn!("行情时段配置读取失败，但时间限制未启用：{}", error),
            }
            ds_manager.set_request_policy_enabled(quote_schedule_enabled);
            let ds_manager = Arc::new(ds_manager);

            // Initialize cache and restore from SQLite
            let cache = Arc::new(QuoteCache::new(db.clone()));
            cache.restore_from_db();
            log::info!("Quote cache initialized and restored from DB");

            // Manage state
            app.manage(db.clone());
            app.manage(ds_manager.clone());
            app.manage(cache.clone());
            app.manage(PortableMode(is_portable));
            app.manage(HotkeyState(Mutex::new(None)));
            app.manage(notifications::NotificationDelivery::new(app.handle().clone()));

            // Start background polling.
            // 0 = AUTO (follow the trading session's recommended interval);
            // any positive value pins the interval the user chose.
            let interval: u64 = db
                .get_setting("refresh_interval")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
                .min(60);

            let polling_config = Arc::new(cache::PollingConfig::new(interval));
            app.manage(polling_config.clone());

            crate::cache::Scheduler::spawn(
                ds_manager,
                cache,
                db.clone(),
                app.handle().clone(),
                polling_config,
            );

            // ── System Tray ──
            let show_item = MenuItemBuilder::with_id("show", "显示主界面").build(app)?;
            let toggle_ticker = MenuItemBuilder::with_id("toggle_ticker", "显示/隐藏行情条").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;

            // Portable mode: skip the "check update" tray item — updates
            // are managed by the user (download & replace the zip).
            let menu = if is_portable {
                MenuBuilder::new(app)
                    .item(&show_item)
                    .item(&toggle_ticker)
                    .separator()
                    .item(&quit_item)
                    .build()?
            } else {
                let check_update_item =
                    MenuItemBuilder::with_id("check_update", "检查更新").build(app)?;
                MenuBuilder::new(app)
                    .item(&show_item)
                    .item(&toggle_ticker)
                    .separator()
                    .item(&check_update_item)
                    .item(&quit_item)
                    .build()?
            };

            let _tray = TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .expect("Default window icon not embedded — check tauri.conf.json icons config"),
                )
                .tooltip("Bull Arrives")
                .menu(&menu)
                .on_menu_event({
                    let db = db.clone();
                    move |app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if app.get_webview_window("main").is_some() {
                                if let Err(error) = commands::window::show_main_window(app.clone()) {
                                    log::warn!("显示主窗口失败: {}", error);
                                }
                            }
                        }
                        "toggle_ticker" => {
                            toggle_ticker_window(app, &db);
                        }
                        "check_update" => {
                            // Portable mode: the "check update" tray item is hidden,
                            // but guard here as a safety net.
                            if app.state::<PortableMode>().0 {
                                log::info!("[updater] Tray check_update ignored — portable mode");
                                return;
                            }
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                match crate::commands::updater::do_check_update(&handle).await
                                {
                                    Ok(Some(info)) => {
                                        let _ = handle.emit("update-available", &info);
                                    }
                                    Ok(None) => {
                                        log::info!("[updater] Manual check: already up to date");
                                        let _ = handle.emit("update-check-complete", "up-to-date");
                                    }
                                    Err(e) => {
                                        log::warn!("[updater] Manual check failed: {}", e);
                                        let _ = handle.emit("update-check-complete", "error");
                                    }
                                }
                            });
                        }
                        "quit" => {
                            if let Some(w) = app.get_webview_window("main") { let _ = w.close(); }
                            if let Some(w) = app.get_webview_window("ticker") { let _ = w.close(); }
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                                handle.exit(0);
                            });
                        }
                        _ => {}
                    }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                if let Err(error) = commands::window::show_main_window(app.clone()) {
                                    log::warn!("显示主窗口失败: {}", error);
                                }
                            }
                        }
                    }
                })
                .build(app)?;

            // Main window: hide on close, save/restore position and size
            if let Some(main) = app.get_webview_window("main") {
                let main_clone = main.clone();
                let db_clone = db.clone();
                // Debounced geometry save: Moved/Resized fire on every pixel
                // during drag, but we only persist once the user stops moving
                // the window for 800ms (drag-end behaviour).  This avoids
                // hundreds of DB writes during a single resize/move gesture.
                let save_counter = Arc::new(AtomicU64::new(0));
                let _ = main.on_window_event(move |event| {
                    match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            let is_min = main_clone.is_minimized().unwrap_or(false);
                            let is_vis = main_clone.is_visible().unwrap_or(false);
                            if is_vis && !is_min {
                                let is_max = main_clone.is_maximized().unwrap_or(false);
                                let _ = db_clone.set_setting("window_maximized", if is_max { "1" } else { "0" });
                                if !is_max {
                                    if let Ok(pos) = main_clone.outer_position() {
                                        if let Err(e) = db_clone.set_setting("window_x", &pos.x.to_string()) {
                                            log::warn!("Failed to save window_x on close: {}", e);
                                        }
                                        if let Err(e) = db_clone.set_setting("window_y", &pos.y.to_string()) {
                                            log::warn!("Failed to save window_y on close: {}", e);
                                        }
                                    }
                                }
                                if let Ok(size) = main_clone.outer_size() {
                                    if let Err(e) = db_clone.set_setting("window_width", &size.width.to_string()) {
                                        log::warn!("Failed to save window_width on close: {}", e);
                                    }
                                    if let Err(e) = db_clone.set_setting("window_height", &size.height.to_string()) {
                                        log::warn!("Failed to save window_height on close: {}", e);
                                    }
                                }
                            }
                            let _ = main_clone.hide();
                        }
                        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                            if main_clone.is_minimized().unwrap_or(false)
                                || !main_clone.is_visible().unwrap_or(false)
                            {
                                return;
                            }
                            // fetch_add returns the PREVIOUS value, so +1 to get
                            // the value WE just set (checked by the debounce task).
                            let count = save_counter.fetch_add(1, Ordering::SeqCst) + 1;
                            let main = main_clone.clone();
                            let db = db_clone.clone();
                            let counter = save_counter.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                                // If counter changed, another event fired during
                                // the wait — the user is still dragging, skip.
                                if counter.load(Ordering::SeqCst) != count {
                                    return;
                                }
                                if let Ok(pos) = main.outer_position() {
                                    let _ = db.set_setting("window_x", &pos.x.to_string());
                                    let _ = db.set_setting("window_y", &pos.y.to_string());
                                }
                                if let Ok(size) = main.outer_size() {
                                    if size.width > 0 && size.height > 0 {
                                        let _ = db.set_setting("window_width", &size.width.to_string());
                                        let _ = db.set_setting("window_height", &size.height.to_string());
                                    }
                                }
                            });
                        }
                        _ => {}
                    }
                });

                // Restore saved window position and size.
                // Validate against actual monitor geometry — skip saved values
                // that would place the window off-screen.
                let (mon_w, mon_h) = main.primary_monitor()
                    .ok()
                    .flatten()
                    .map(|m| {
                        let s = m.size();
                        (s.width as i32, s.height as i32)
                    })
                    .unwrap_or((1920, 1080));

                // Read default window size from tauri.conf.json
                let (default_w, default_h) = app.config()
                    .app
                    .windows
                    .iter()
                    .find(|w| w.label == "main")
                    .map(|w| (w.width as u32, w.height as u32))
                    .unwrap_or((1388, 1009));

                // Restore saved geometry if valid
                let (mut saved_w, mut saved_h) = (0u32, 0u32);
                let (mut saved_x, mut saved_y) = (0i32, 0i32);
                let mut has_size = false;
                let mut has_pos = false;

                if let Ok(Some(w)) = db.get_setting("window_width") {
                    if let Ok(Some(h)) = db.get_setting("window_height") {
                        if let (Ok(w_val), Ok(h_val)) = (w.parse::<u32>(), h.parse::<u32>()) {
                            if w_val >= 400 && w_val <= mon_w as u32
                                && h_val >= 300 && h_val <= mon_h as u32
                            {
                                saved_w = w_val;
                                saved_h = h_val;
                                has_size = true;
                            }
                        }
                    }
                }
                if let Ok(Some(x)) = db.get_setting("window_x") {
                    if let Ok(Some(y)) = db.get_setting("window_y") {
                        if let (Ok(x_val), Ok(y_val)) = (x.parse::<i32>(), y.parse::<i32>()) {
                            if x_val + 200 < mon_w && x_val > -50
                                && y_val + 100 < mon_h && y_val > -50
                            {
                                saved_x = x_val.max(0);
                                saved_y = y_val.max(0);
                                has_pos = true;
                            }
                        }
                    }
                }

                let was_max = db.get_setting("window_maximized")
                    .ok()
                    .flatten()
                    .map(|v| v == "1")
                    .unwrap_or(false);

                // Show first so the native NSWindow is realized before applying
                // geometry (required for correct sizing on macOS).
                let remember_size = db.get_setting("main_window_size").ok().flatten()
                    .and_then(|raw| serde_json::from_str::<commands::window::MainWindowSize>(&raw).ok())
                    .map(|config| config.remember).unwrap_or(false);
                if !remember_size {
                    if let Err(error) = commands::window::apply_main_window_size(app.handle(), &db) {
                        log::warn!("主窗口打开尺寸恢复失败: {}", error);
                    }
                }
                let _ = main.show();
                if !remember_size {
                    let _ = main.center();
                } else if was_max {
                    let w = if has_size { saved_w } else { default_w };
                    let h = if has_size { saved_h } else { default_h };
                    let _ = main.set_size(tauri::PhysicalSize::new(w, h));
                    let _ = main.maximize();
                } else if has_pos {
                    let w = if has_size { saved_w } else { default_w };
                    let h = if has_size { saved_h } else { default_h };
                    let _ = main.set_size(tauri::PhysicalSize::new(w, h));
                    let _ = main.set_position(tauri::PhysicalPosition::new(saved_x, saved_y));
                } else {
                    // No saved geometry: use config defaults and center
                    let _ = main.set_size(tauri::PhysicalSize::new(default_w, default_h));
                    let _ = main.center();
                }
                let _ = main.set_focus();
            }

            // Ticker window: save position on move (clamped), restore on startup
            if let Some(ticker) = app.get_webview_window("ticker") {
                let _ = ticker.set_always_on_top(true);

                // Capture monitor bounds and ticker size for clamping on move
                let mon = ticker.primary_monitor().ok().flatten();
                let (mon_w, mon_h) = mon
                    .as_ref()
                    .map(|m| { let s = m.size(); (s.width as i32, s.height as i32) })
                    .unwrap_or((1920, 1080));
                let ticker_size = ticker.outer_size().unwrap_or(tauri::PhysicalSize::new(
                    crate::datasource::TICKER_WIDTH,
                    crate::datasource::TICKER_HEIGHT,
                ));
                let tw = ticker_size.width as i32;
                let th = ticker_size.height as i32;

                // Save ticker position on move.  Only persist if enough of the
                // ticker is actually visible — if the user drags it way off
                // screen, we skip saving so the next launch falls back to the
                // default bottom-right position.
                let db_clone = db.clone();
                let _ = ticker.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        // How much of the ticker is inside the monitor bounds?
                        let visible_left = pos.x.max(0);
                        let visible_right = (pos.x + tw).min(mon_w);
                        let visible_w = (visible_right - visible_left).max(0);
                        let visible_top = pos.y.max(0);
                        let visible_bottom = (pos.y + th).min(mon_h);
                        let visible_h = (visible_bottom - visible_top).max(0);

                        // Require at least 50×20 px visible — otherwise it's
                        // too far off-screen to be easily found.
                        if visible_w < 50 || visible_h < 20 {
                            return;
                        }

                        let clamped_x = pos.x.max(0).min(mon_w - tw);
                        let clamped_y = pos.y.max(0).min(mon_h - th);
                        if let Err(e) = db_clone.set_setting("ticker_x", &clamped_x.to_string()) {
                            log::warn!("Failed to save ticker_x: {}", e);
                        }
                        if let Err(e) = db_clone.set_setting("ticker_y", &clamped_y.to_string()) {
                            log::warn!("Failed to save ticker_y: {}", e);
                        }
                    }
                });

                // Restore saved position, fall back to bottom-right
                let (mut saved_x, mut saved_y) = (0i32, 0i32);
                let mut has_pos = false;
                if let Ok(Some(x)) = db.get_setting("ticker_x") {
                    if let Ok(Some(y)) = db.get_setting("ticker_y") {
                        if let (Ok(x_val), Ok(y_val)) = (x.parse::<i32>(), y.parse::<i32>()) {
                            saved_x = x_val;
                            saved_y = y_val;
                            has_pos = true;
                        }
                    }
                }
                if has_pos
                    && saved_x + tw > 0
                    && saved_y + th > 0
                    && saved_x < mon_w
                    && saved_y < mon_h
                {
                    let _ = ticker.set_position(tauri::PhysicalPosition::new(saved_x, saved_y));
                } else {
                    let x = (mon_w).saturating_sub(tw + 10);
                    let y = (mon_h).saturating_sub(th + 60);
                    let _ = ticker.set_position(tauri::PhysicalPosition::new(x, y));
                }

                // Remove ticker from taskbar at both levels:
                //   set_skip_taskbar  → ITaskbarList::DeleteTab (immediate, one-shot)
                //   apply_tool_window → WS_EX_TOOLWINDOW (survives Explorer restart)
                let _ = ticker.set_skip_taskbar(true);
                apply_tool_window_style(&ticker);

                // Restore ticker opacity from settings (default fully opaque).
                let ticker_opacity: u32 = db
                    .get_setting("ticker_opacity")
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(100);
                let alpha = ((ticker_opacity as f32 / 100.0) * 255.0).round() as u8;
                if let Err(error) = apply_ticker_opacity(&ticker, alpha) { log::warn!("透明度恢复失败: {}", error); }

                // Restore visibility from last session (config starts hidden).
                // Default to visible unless the user explicitly hid the ticker.
                let ticker_hidden = db
                    .get_setting("ticker_visible")
                    .ok()
                    .flatten()
                    .map(|v| v == "0")
                    .unwrap_or(false);
                if ticker_hidden {
                    let _ = ticker.hide();
                } else {
                    let _ = ticker.show();
                    // 显示原生窗口后重新应用，避免分层窗口样式在首次 show 时被刷新。
                    apply_tool_window_style(&ticker);
                    if let Err(error) = apply_ticker_opacity(&ticker, alpha) { log::warn!("透明度恢复失败: {}", error); }
                }
            }

            // Register the global hotkey for toggling the ticker.
            // Default: Alt+Q.  Persisted value wins if present and parseable.
            let default_hotkey = "Alt+Q";
            let hotkey_str = db
                .get_setting("ticker_hotkey")
                .ok()
                .flatten()
                .unwrap_or_else(|| default_hotkey.to_string());
            match hotkey_str.parse::<Shortcut>() {
                Ok(sc) => {
                    if let Err(e) = app.global_shortcut().register(sc) {
                        log::warn!("[hotkey] Failed to register '{}': {}", hotkey_str, e);
                    } else {
                        app.state::<HotkeyState>().0.lock().unwrap().replace(sc);
                        log::info!("[hotkey] Registered ticker toggle: '{}'", hotkey_str);
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[hotkey] Stored hotkey '{}' is invalid ({}); falling back to default",
                        hotkey_str, e
                    );
                    if let Ok(sc) = default_hotkey.parse::<Shortcut>() {
                        if let Err(e) = app.global_shortcut().register(sc) {
                            log::warn!("[hotkey] Default '{}' also failed: {}", default_hotkey, e);
                        } else {
                            app.state::<HotkeyState>().0.lock().unwrap().replace(sc);
                            let _ = db.set_setting("ticker_hotkey", default_hotkey);
                        }
                    }
                }
            }

            group_hotkeys::restore(app.handle(), &db);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::quote::get_quotes,
            commands::quote::get_indices,
            commands::quote::get_depth,
            commands::quote::get_intraday,
            commands::quote::get_kline,
            commands::watchlist::get_watchlist,
            commands::watchlist::add_watch,
            commands::watchlist::remove_watch,
            commands::watchlist::reorder_watch,
            commands::watchlist::move_watch_top,
            commands::watchlist::move_watch_up,
            commands::watchlist::move_watch_down,
            commands::watchlist::search_stocks,
            notifications::get_notification_history,
            notifications::clear_notification_history,
            notifications::test_notification,
            notification_identity::get_notification_identity_status,
            notification_identity::register_notification_identity,
            desktop_toast::desktop_toast_ready,
            desktop_toast::dismiss_desktop_toast,
            desktop_toast::view_desktop_toast,
            group_hotkeys::set_group_hotkey,
            commands::holdings::get_holdings,
            commands::holdings::save_holding,
            commands::holdings::delete_holding,
            commands::groups::get_group_snapshot,
            commands::groups::create_watch_group,
            commands::groups::rename_watch_group,
            commands::groups::select_watch_group,
            commands::groups::delete_watch_group,
            commands::groups::set_group_member,
            commands::watchlist::get_price_alerts,
            commands::watchlist::save_price_alert,
            commands::watchlist::delete_price_alert,
            commands::settings::get_settings,
            commands::settings::set_setting,
            commands::settings::switch_datasource,
            commands::settings::list_datasources,
            commands::settings::get_portable_mode,
            commands::settings::set_refresh_interval,
            commands::settings::get_market_session,
            commands::window::show_main_window,
            commands::window::set_main_window_size,
            commands::window::set_ticker_hotkey,
            commands::window::set_ticker_opacity,
            commands::window::resize_ticker_window,
            commands::updater::check_update,
            commands::updater::install_update,
            commands::updater::is_trading_session,
        ])
        .build(tauri::generate_context!())
        .expect("Failed to build application")
        .run(|app_handle, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Err(error) = commands::window::show_main_window(app_handle.clone()) {
                    log::warn!("Dock 重新打开主窗口失败: {}", error);
                }
            }

            #[cfg(not(target_os = "macos"))]
            let _ = (app_handle, event);
        });
}
