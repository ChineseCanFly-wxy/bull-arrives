use crate::stockdb::{StockDbManager, StockDbStatus};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub fn get_stockdb_status(manager: State<'_, Arc<StockDbManager>>) -> StockDbStatus {
    manager.status()
}

#[tauri::command]
pub async fn scan_stockdb(
    manager: State<'_, Arc<StockDbManager>>,
) -> Result<StockDbStatus, String> {
    manager.scan().await
}

#[tauri::command]
pub async fn select_stockdb_engine(
    manager: State<'_, Arc<StockDbManager>>,
    path: String,
) -> Result<StockDbStatus, String> {
    manager.select_engine(PathBuf::from(path)).await
}

#[tauri::command]
pub async fn select_stockdb_updater(
    manager: State<'_, Arc<StockDbManager>>,
    path: String,
) -> Result<StockDbStatus, String> {
    manager.select_updater(PathBuf::from(path)).await
}

#[tauri::command]
pub async fn set_local_history_enabled(
    manager: State<'_, Arc<StockDbManager>>,
    enabled: bool,
) -> Result<StockDbStatus, String> {
    manager.set_enabled(enabled).await
}

#[tauri::command]
pub async fn run_stockdb_update(
    manager: State<'_, Arc<StockDbManager>>,
) -> Result<String, String> {
    manager.update().await
}
