use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use crate::{db::Database, datasource::DataSourceManager};

#[derive(Default)]
pub struct GroupHotkeys(pub Mutex<[Option<Shortcut>; 2]>);

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

#[tauri::command]
pub fn set_group_hotkey(app: tauri::AppHandle, db: State<'_, Arc<Database>>, direction: String, hotkey: String) -> Result<(),String> {
    let index = match direction.as_str() { "previous"=>0, "next"=>1, _=>return Err("切组方向无效".into()) };
    let key = if index==0 { "group_previous_hotkey" } else { "group_next_hotkey" };
    let candidate = if hotkey.trim().is_empty() { None } else { Some(hotkey.parse::<Shortcut>().map_err(|e|e.to_string())?) };
    let state = app.state::<GroupHotkeys>();
    let mut keys = state.0.lock().unwrap_or_else(|e|e.into_inner());
    if keys[index] == candidate { return db.set_setting(key,&hotkey).map_err(|e|e.to_string()); }
    if let Some(sc) = candidate { app.global_shortcut().register(sc).map_err(|e| e.to_string())?; }
    if let Err(error) = db.set_setting(key,&hotkey) {
        if let Some(sc) = candidate { let _ = app.global_shortcut().unregister(sc); }
        return Err(error.to_string());
    }
    if let Some(old) = keys[index] {
        if let Err(error) = app.global_shortcut().unregister(old) {
            let _ = db.set_setting(key,&old.to_string());
            if let Some(sc) = candidate { let _ = app.global_shortcut().unregister(sc); }
            return Err(error.to_string());
        }
    }
    keys[index] = candidate;
    Ok(())
}

pub fn restore(app: &tauri::AppHandle, db: &Database) {
    let state = app.state::<GroupHotkeys>();
    for (index,key) in ["group_previous_hotkey","group_next_hotkey"].iter().enumerate() {
        let Some(raw) = db.get_setting(key).ok().flatten() else { continue; };
        if raw.trim().is_empty() { continue; }
        match raw.parse::<Shortcut>() {
            Ok(sc) => match app.global_shortcut().register(sc) {
                Ok(()) => state.0.lock().unwrap_or_else(|e|e.into_inner())[index]=Some(sc),
                Err(error) => log::warn!("分组快捷键恢复失败 {}: {}",key,error),
            },
            Err(error) => log::warn!("分组快捷键无效 {}: {}",key,error),
        }
    }
}
