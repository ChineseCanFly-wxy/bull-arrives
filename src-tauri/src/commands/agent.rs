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
pub async fn test_agent_connection(db: State<'_, Arc<Database>>) -> Result<AgentStatus, String> {
    Ok(crate::agent::test_connection(&db).await)
}

#[tauri::command]
pub async fn analyze_stock_agent(
    db: State<'_, Arc<Database>>,
    context_fingerprint: String,
    user_question: Option<String>,
) -> Result<AgentAnalysisResponse, String> {
    Ok(crate::agent::analyze_question(&db, &context_fingerprint, user_question.as_deref()).await)
}

#[tauri::command]
pub fn inspect_agent_task(
    db: State<'_, Arc<Database>>,
    context_fingerprint: String,
) -> Result<crate::agent::workbench::AgentInspection, String> {
    crate::agent::workbench::inspect(&db, &context_fingerprint)
}

#[tauri::command]
pub fn get_agent_activity(
    db: State<'_, Arc<Database>>,
    context_fingerprint: String,
) -> Result<crate::agent::workbench::Activity, String> {
    crate::agent::workbench::activity(&db, &context_fingerprint)
}

#[tauri::command]
pub fn get_agent_run_detail(
    db: State<'_, Arc<Database>>,
    run_id: i64,
) -> Result<crate::db::agent::AgentRunDetail, String> {
    db.agent_run_detail(run_id)
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
