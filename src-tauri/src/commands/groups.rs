use std::sync::Arc;
use tauri::{Emitter, State};
use crate::db::{Database, groups::GroupSnapshot};
use crate::datasource::DataSourceManager;

fn changed(app: &tauri::AppHandle, manager: &DataSourceManager) {
    manager.invalidate_requests();
    if let Err(error) = app.emit("watchlist-changed", ()) {
        log::warn!("分组状态广播失败: {}", error);
    }
}

#[tauri::command]
pub fn get_group_snapshot(db: State<'_, Arc<Database>>) -> Result<GroupSnapshot, String> {
    db.get_group_snapshot().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_watch_group(app: tauri::AppHandle, db: State<'_, Arc<Database>>, manager: State<'_, Arc<DataSourceManager>>, name: String) -> Result<i64, String> {
    let id = db.create_watch_group(&name)?;
    changed(&app, &manager);
    Ok(id)
}

#[tauri::command]
pub fn rename_watch_group(app: tauri::AppHandle, db: State<'_, Arc<Database>>, manager: State<'_, Arc<DataSourceManager>>, id: i64, name: String) -> Result<(), String> {
    db.rename_watch_group(id, &name)?;
    changed(&app, &manager);
    Ok(())
}

#[tauri::command]
pub fn select_watch_group(app: tauri::AppHandle, db: State<'_, Arc<Database>>, manager: State<'_, Arc<DataSourceManager>>, id: i64) -> Result<(), String> {
    db.select_watch_group(id)?;
    changed(&app, &manager);
    Ok(())
}

#[tauri::command]
pub fn delete_watch_group(app: tauri::AppHandle, db: State<'_, Arc<Database>>, manager: State<'_, Arc<DataSourceManager>>, id: i64) -> Result<(), String> {
    db.delete_watch_group(id)?;
    changed(&app, &manager);
    Ok(())
}

#[tauri::command]
pub fn set_group_member(app: tauri::AppHandle, db: State<'_, Arc<Database>>, manager: State<'_, Arc<DataSourceManager>>, group_id: i64, watch_id: i64, included: bool) -> Result<(), String> {
    db.set_group_member(group_id, watch_id, included)?;
    changed(&app, &manager);
    Ok(())
}
