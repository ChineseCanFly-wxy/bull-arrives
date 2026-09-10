use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Mutex};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const MAX_PENDING: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopToastPayload {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[derive(Default)]
struct ToastStateInner {
    ready: bool,
    current: Option<DesktopToastPayload>,
    pending: VecDeque<DesktopToastPayload>,
}

#[derive(Default)]
pub struct DesktopToastState(Mutex<ToastStateInner>);

fn ensure_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("notification-toast") {
        return Ok(window);
    }
    let window = WebviewWindowBuilder::new(
        app,
        "notification-toast",
        WebviewUrl::App("toast.html".into()),
    )
    .title("QuantDesktop 提醒")
    .inner_size(360.0, 142.0)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .build()
    .map_err(|e| format!("创建桌面提醒窗失败: {e}"))?;
    let _ = window.set_skip_taskbar(true);
    crate::apply_nonactivating_tool_window_style(&window);
    Ok(window)
}

fn position_bottom_right(window: &tauri::WebviewWindow) {
    if let Ok(Some(monitor)) = window
        .current_monitor()
        .or_else(|_| window.primary_monitor())
    {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let width = (360.0 * scale).round() as i32;
        let height = (142.0 * scale).round() as i32;
        let x = area.position.x + area.size.width as i32 - width - (16.0 * scale) as i32;
        let y = area.position.y + area.size.height as i32 - height - (16.0 * scale) as i32;
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

fn show(app: &tauri::AppHandle, payload: &DesktopToastPayload) -> Result<(), String> {
    let window = ensure_window(app)?;
    position_bottom_right(&window);
    window
        .emit("desktop-toast-show", payload)
        .map_err(|e| format!("发送桌面提醒内容失败: {e}"))?;
    window
        .show()
        .map_err(|e| format!("显示桌面提醒窗失败: {e}"))?;
    crate::apply_nonactivating_tool_window_style(&window);
    Ok(())
}

fn insert(inner: &mut ToastStateInner, payload: DesktopToastPayload) -> Result<bool, String> {
    if inner.current.is_none() {
        inner.current = Some(payload);
        return Ok(inner.ready);
    }
    if inner.pending.len() >= MAX_PENDING {
        return Err("桌面提醒队列已满，消息仅保留在提醒记录中".into());
    }
    inner.pending.push_back(payload);
    Ok(false)
}

pub fn enqueue(app: &tauri::AppHandle, payload: DesktopToastPayload) -> Result<(), String> {
    ensure_window(app)?;
    let to_show = {
        let state = app.state::<DesktopToastState>();
        let mut inner = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if insert(&mut inner, payload)? {
            inner.current.clone()
        } else {
            None
        }
    };
    if let Some(payload) = to_show {
        show(app, &payload)?;
    }
    Ok(())
}

#[tauri::command]
pub fn desktop_toast_ready(app: tauri::AppHandle) -> Result<(), String> {
    let current = {
        let state = app.state::<DesktopToastState>();
        let mut inner = state.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.ready = true;
        inner.current.clone()
    };
    if let Some(payload) = current {
        show(&app, &payload)?;
    }
    Ok(())
}

/// Close or time out exactly the currently displayed id, then advance once.
/// A stale timer cannot hide a newer toast because its id no longer matches.
#[tauri::command]
pub fn dismiss_desktop_toast(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let next = {
        let state = app.state::<DesktopToastState>();
        let mut inner = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.current.as_ref().map(|item| item.id.as_str()) != Some(id.as_str()) {
            return Ok(());
        }
        inner.current = inner.pending.pop_front();
        inner.current.clone()
    };
    if let Some(payload) = next {
        show(&app, &payload)?;
    } else if let Some(window) = app.get_webview_window("notification-toast") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn view_desktop_toast(app: tauri::AppHandle, id: String) -> Result<(), String> {
    {
        let state = app.state::<DesktopToastState>();
        let mut inner = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.current.as_ref().map(|item| item.id.as_str()) != Some(id.as_str()) {
            return Ok(());
        }
        inner.current = None;
        inner.pending.clear();
    }
    crate::commands::window::show_main_window(app.clone())?;
    if let Some(window) = app.get_webview_window("notification-toast") {
        let _ = window.hide();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(id: usize) -> DesktopToastPayload {
        DesktopToastPayload {
            id: id.to_string(),
            title: "t".into(),
            body: "b".into(),
        }
    }

    #[test]
    fn enqueue_is_bounded_and_keeps_current_separate() {
        let mut inner = ToastStateInner::default();
        assert!(!insert(&mut inner, payload(0)).unwrap());
        for id in 1..=MAX_PENDING {
            assert!(!insert(&mut inner, payload(id)).unwrap());
        }
        assert!(insert(&mut inner, payload(99)).is_err());
        assert_eq!(inner.current.unwrap().id, "0");
        assert_eq!(inner.pending.len(), MAX_PENDING);
    }

    #[test]
    fn stale_id_does_not_match_new_current() {
        let mut inner = ToastStateInner::default();
        insert(&mut inner, payload(1)).unwrap();
        inner.pending.push_back(payload(2));
        inner.current = inner.pending.pop_front();
        assert_ne!(inner.current.as_ref().unwrap().id, "1");
    }
}
