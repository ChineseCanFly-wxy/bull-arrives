use crate::agent::{AgentAnalysisResponse, AgentStatus, AgentTeamResponse};
use crate::db::predictions::PredictionCalibration;
use crate::db::Database;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub fn get_agent_status(db: State<'_, Arc<Database>>) -> AgentStatus {
    crate::agent::status(&db)
}

#[tauri::command]
pub async fn analyze_stock_agent(
    db: State<'_, Arc<Database>>,
    context_fingerprint: String,
) -> Result<AgentAnalysisResponse, String> {
    Ok(crate::agent::analyze(&db, &context_fingerprint).await)
}

#[tauri::command]
pub fn cancel_agent_analysis() -> bool {
    crate::agent::cancel()
}

#[tauri::command]
pub async fn analyze_stock_team(
    db: State<'_, Arc<Database>>,
    context_fingerprint: String,
) -> Result<AgentTeamResponse, String> {
    Ok(crate::agent::analyze_team(&db, &context_fingerprint).await)
}

#[tauri::command]
pub fn get_prediction_calibration(
    db: State<'_, Arc<Database>>,
) -> Result<PredictionCalibration, String> {
    db.prediction_calibration()
}
