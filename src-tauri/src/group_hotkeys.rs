use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use crate::{db::Database, datasource::DataSourceManager};

/// 分组快捷键在 settings 表中的两个键名。
const PREV_KEY: &str = "group_previous_hotkey";
const NEXT_KEY: &str = "group_next_hotkey";
const SETTING_KEYS: [&str; 2] = [PREV_KEY, NEXT_KEY];

/// 系统 / 桌面已占用的组合键。
///
/// Windows 侧 `RegisterHotKey` 遇到冲突会直接返回错误，无需黑名单；
/// macOS 侧 `RegisterEventHotKey` 不报错（系统会把按键同时分发给多个应用），
/// 所以只能靠这份名单在注册前拦下最常见的系统占用。
const RESERVED_COMBOS: &[&str] = &[
    // 窗口/系统级
    "alt+f4", "alt+tab", "control+alt+delete",
    // Windows 徽标键
    "super+l", "super+d", "super+e", "super+r", "super+i", "super+s",
    "super+a", "super+v", "super+tab", "super+space",
    // macOS ⌘ 系列
    "super+q", "super+w", "super+h", "super+m", "super+tab", "super+space",
    "super+backquote", "control+super+q", "shift+super+q",
    // macOS 调度中心 / 切换桌面
    "control+left", "control+right", "control+up", "control+down",
];

#[derive(Default)]
pub struct GroupHotkeys(pub Mutex<[Option<Shortcut>; 2]>);

/// 单个方向的可视状态。`registered` 取自实际注册结果，
/// 用于区分「已保存」与「真的生效」——保存成功但没生效时界面会直接提示。
#[derive(serde::Serialize)]
pub struct GroupHotkeyEntry {
    pub hotkey: String,
    pub registered: bool,
}

#[derive(serde::Serialize)]
pub struct GroupHotkeyStatus {
    pub previous: GroupHotkeyEntry,
    pub next: GroupHotkeyEntry,
}

fn direction_index(direction: &str) -> Result<usize, String> {
    match direction {
        "previous" => Ok(0),
        "next" => Ok(1),
        _ => Err("切组方向无效".into()),
    }
}

fn direction_label(index: usize) -> &'static str {
    if index == 0 { "上一分组" } else { "下一分组" }
}

/// 比较两个组合键是否指同一物理按键（id 由 mods + key 派生，无需单独比较）。
fn same_shortcut(a: &Shortcut, b: &Shortcut) -> bool {
    a.mods == b.mods && a.key == b.key
}

/// 至少含一个修饰键：全局单键会吞掉系统里所有同类输入。
fn ensure_has_modifier(sc: &Shortcut) -> Result<(), String> {
    if sc.mods.is_empty() {
        return Err("组合键必须包含 Ctrl / Alt / Shift / ⌘（Super）中的至少一个修饰键".into());
    }
    Ok(())
}

/// 注册前拦下系统保留组合，避免出现「保存成功但按了没反应」。
fn ensure_not_reserved(sc: &Shortcut) -> Result<(), String> {
    for raw in RESERVED_COMBOS {
        if let Ok(reserved) = raw.parse::<Shortcut>() {
            if same_shortcut(&reserved, sc) {
                return Err(format!("「{}」已被系统占用，请换一个组合键", sc));
            }
        }
    }
    Ok(())
}

/// 注册失败时给用户看得懂的原因。
fn friendly_register_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("already registered") || lower.contains("registereventhotkey failed") {
        "该快捷键已被其他程序或系统占用，请更换组合键".to_string()
    } else {
        format!("快捷键注册失败：{error}")
    }
}

fn build_status(db: &Database, keys: &[Option<Shortcut>; 2]) -> GroupHotkeyStatus {
    let entry = |index: usize| GroupHotkeyEntry {
        hotkey: db.get_setting(SETTING_KEYS[index]).ok().flatten().unwrap_or_default(),
        registered: keys[index].is_some(),
    };
    GroupHotkeyStatus { previous: entry(0), next: entry(1) }
}

/// 读取两个方向当前的真实状态（已保存的组合键 + 是否注册成功）。
#[tauri::command]
pub fn get_group_hotkey_status(app: tauri::AppHandle, db: State<'_, Arc<Database>>) -> GroupHotkeyStatus {
    let state = app.state::<GroupHotkeys>();
    let keys = state.0.lock().unwrap_or_else(|e| e.into_inner());
    build_status(&db, &keys)
}

pub fn handle(app: &tauri::AppHandle, shortcut: &Shortcut) -> bool {
    let state = app.state::<GroupHotkeys>();
    let index = state.0.lock().unwrap_or_else(|e| e.into_inner()).iter().position(|s| s.as_ref() == Some(shortcut));
    let Some(index) = index else { return false; };
    let db = app.state::<Arc<Database>>();
    match db.get_group_snapshot() {
        Ok(snapshot) => {
            let ids: Vec<i64> = std::iter::once(0).chain(snapshot.groups.iter().map(|g| g.id)).collect();
            let current = ids.iter().position(|id| *id == snapshot.active_group_id).unwrap_or(0);
            let target = if index == 0 { (current + ids.len() - 1) % ids.len() } else { (current + 1) % ids.len() };
            if let Err(error) = db.select_watch_group(ids[target]) { log::warn!("快捷键切组失败: {}",error); }
            else {
                app.state::<Arc<DataSourceManager>>().invalidate_requests();
                let _ = app.emit("watchlist-changed", ());
            }
        }
        Err(error) => log::warn!("快捷键读取分组失败: {}",error),
    }
    true
}

/// 设置 / 清除某个方向的分组快捷键，并返回两个方向的最新状态。
///
/// 校验顺序：修饰键 → 系统保留组合 → 与另一方向重复 → 与悬浮窗重复 → 交系统注册（占用即报错）。
/// 任一步失败都会保留原有快捷键不动，前端直接展示错误原因。
#[tauri::command]
pub fn set_group_hotkey(app: tauri::AppHandle, db: State<'_, Arc<Database>>, direction: String, hotkey: String) -> Result<GroupHotkeyStatus, String> {
    let index = direction_index(&direction)?;
    let key = SETTING_KEYS[index];
    let trimmed = hotkey.trim().to_string();

    let candidate = if trimmed.is_empty() { None } else {
        Some(trimmed.parse::<Shortcut>().map_err(|_| "快捷键格式无法识别，请重新录制".to_string())?)
    };
    if let Some(sc) = candidate.as_ref() {
        ensure_has_modifier(sc)?;
        ensure_not_reserved(sc)?;
    }

    // 悬浮窗快捷键同样占用系统注册空间，先独立读出来，避免与 GroupHotkeys 互锁。
    let ticker = *app.state::<crate::HotkeyState>().0.lock().unwrap_or_else(|e| e.into_inner());

    let state = app.state::<GroupHotkeys>();
    let mut keys = state.0.lock().unwrap_or_else(|e| e.into_inner());

    // 组合键没变化：清空时也走这条路，只需确保落库。
    if keys[index] == candidate {
        db.set_setting(key, &trimmed).map_err(|e| e.to_string())?;
        return Ok(build_status(&db, &keys));
    }

    if let Some(sc) = candidate.as_ref() {
        if let Some(other) = keys[1 - index] {
            if same_shortcut(&other, sc) {
                return Err(format!("与「{}」的快捷键重复，请换一个组合键", direction_label(1 - index)));
            }
        }
        if let Some(ticker) = ticker {
            if same_shortcut(&ticker, sc) {
                return Err("该组合键已用于显示/隐藏悬浮窗，请更换".into());
            }
        }
        // 真正注册一次：被系统或其他程序占用会在这里返回错误，这是最可靠的冲突实测。
        app.global_shortcut().register(*sc).map_err(|e| friendly_register_error(&e.to_string()))?;
    }

    let previous_text = db.get_setting(key).ok().flatten().unwrap_or_default();
    if let Err(error) = db.set_setting(key, &trimmed) {
        if let Some(sc) = candidate { let _ = app.global_shortcut().unregister(sc); }
        return Err(error.to_string());
    }
    if let Some(old) = keys[index] {
        if let Err(error) = app.global_shortcut().unregister(old) {
            // 回滚到改动前的状态，避免出现「新键没生效、旧键也没了」。
            let _ = db.set_setting(key, &previous_text);
            if let Some(sc) = candidate { let _ = app.global_shortcut().unregister(sc); }
            return Err(format!("旧快捷键注销失败，未切换：{error}"));
        }
    }
    keys[index] = candidate;
    log::info!("[hotkey] group {} -> '{}'", direction_label(index), trimmed);
    Ok(build_status(&db, &keys))
}

pub fn restore(app: &tauri::AppHandle, db: &Database) {
    let state = app.state::<GroupHotkeys>();
    for (index, key) in SETTING_KEYS.iter().enumerate() {
        let Some(raw) = db.get_setting(key).ok().flatten() else { continue; };
        if raw.trim().is_empty() { continue; }
        match raw.parse::<Shortcut>() {
            Ok(sc) => match app.global_shortcut().register(sc) {
                Ok(()) => state.0.lock().unwrap_or_else(|e| e.into_inner())[index]=Some(sc),
                Err(error) => log::warn!("分组快捷键恢复失败 {}: {}",key,error),
            },
            Err(error) => log::warn!("分组快捷键无效 {}: {}",key,error),
        }
    }
}
