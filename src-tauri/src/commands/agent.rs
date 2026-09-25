use crate::agent::interactive::{InteractiveRoot, InteractiveTask};
use crate::agent::live::LiveStatus;
use crate::agent::{AgentAnalysisResponse, AgentStatus, AgentTeamResponse};
use uuid::Uuid;
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
pub fn start_interactive_analysis(
    db: State<'_, Arc<Database>>,
    root: State<'_, InteractiveRoot>,
    context_fingerprint: String,
) -> Result<InteractiveTask, String> {
    crate::agent::interactive::start(&db, &root.0, &context_fingerprint)
}

#[tauri::command]
pub async fn list_interactive_analyses(
    root: State<'_, InteractiveRoot>,
    symbol: String,
) -> Result<Vec<InteractiveTask>, String> {
    let path = root.0.clone();
    tokio::task::spawn_blocking(move || crate::agent::interactive::list(&path, &symbol))
        .await
        .map_err(|e| format!("读取交互任务失败：{e}"))
}

#[tauri::command]
pub fn resume_interactive_analysis(
    db: State<'_, Arc<Database>>,
    root: State<'_, InteractiveRoot>,
    task_id: Uuid,
) -> Result<InteractiveTask, String> {
    crate::agent::interactive::resume(&db, &root.0, task_id)
}

#[tauri::command]
pub fn import_interactive_analysis(
    db: State<'_, Arc<Database>>,
    root: State<'_, InteractiveRoot>,
    task_id: Uuid,
) -> Result<AgentAnalysisResponse, String> {
    crate::agent::interactive::import_result(&db, &root.0, task_id)
}

#[tauri::command]
pub fn inspect_interactive_analysis(
    root: State<'_, InteractiveRoot>,
    task_id: Uuid,
) -> Result<crate::agent::interactive::InteractiveTaskActivity, String> {
    crate::agent::interactive::inspect_activity(&root.0, task_id)
}

#[tauri::command]
pub fn delete_interactive_analysis(
    root: State<'_, InteractiveRoot>,
    task_id: Uuid,
    closed_session: bool,
) -> Result<(), String> {
    crate::agent::interactive::delete_task(&root.0, task_id, closed_session)?;
    crate::agent::live::forget(task_id);
    Ok(())
}

#[tauri::command]
pub fn start_live_analysis(
    db: State<'_, Arc<Database>>,
    root: State<'_, InteractiveRoot>,
    app: tauri::AppHandle,
    context_fingerprint: String,
) -> Result<LiveStatus, String> {
    crate::agent::live::start(&db, &root.0, app, context_fingerprint)
}

#[tauri::command]
pub fn ask_live_analysis(
    db: State<'_, Arc<Database>>,
    root: State<'_, InteractiveRoot>,
    app: tauri::AppHandle,
    task_id: Uuid,
    question: String,
) -> Result<LiveStatus, String> {
    crate::agent::live::ask(&db, &root.0, app, task_id, question)
}

#[tauri::command]
pub fn get_live_analysis(
    db: State<'_, Arc<Database>>,
    root: State<'_, InteractiveRoot>,
    task_id: Uuid,
) -> Result<LiveStatus, String> {
    crate::agent::live::restore(&db, &root.0, task_id)
}

#[tauri::command]
pub fn cancel_live_analysis(task_id: Uuid) -> bool {
    crate::agent::live::cancel(task_id)
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
