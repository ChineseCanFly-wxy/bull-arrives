use crate::db::{
    strategies::{StrategyCardView, StrategyLibrary},
    Database,
};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub fn strategy_library(db: State<'_, Arc<Database>>) -> Result<StrategyLibrary, String> {
    crate::commands::universe::sync_strategy_registry(&db);
    db.strategy_library()
}

#[tauri::command]
pub fn strategy_transition(
    db: State<'_, Arc<Database>>,
    version_id: i64,
    action: String,
    confirmed: bool,
    expected_revision: i64,
) -> Result<StrategyCardView, String> {
    db.transition_strategy(version_id, &action, confirmed, expected_revision)
}
