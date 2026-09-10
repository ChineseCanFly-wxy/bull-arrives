use crate::db::{holdings::Holding, Database};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub fn get_holdings(db: State<'_, Arc<Database>>) -> Result<Vec<Holding>, String> {
    db.get_holdings().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_holding(
    db: State<'_, Arc<Database>>,
    watch_id: i64,
    cost_price: Option<String>,
    shares: i64,
) -> Result<(), String> {
    db.save_holding(watch_id, cost_price.as_deref(), shares)
}

#[tauri::command]
pub fn delete_holding(
    db: State<'_, Arc<Database>>,
    watch_id: i64,
) -> Result<(), String> {
    db.delete_holding(watch_id).map_err(|error| error.to_string())
}
