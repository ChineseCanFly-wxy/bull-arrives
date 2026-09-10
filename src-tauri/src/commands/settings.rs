use tauri::State;
use std::collections::HashMap;
use std::sync::Arc;
use crate::cache::PollingConfig;
use crate::db::Database;
use crate::datasource::DataSourceManager;
use crate::datasource::market_clock::MarketSession;
use crate::PortableMode;

#[tauri::command]
pub fn get_settings(db: State<'_, Arc<Database>>) -> Result<HashMap<String, String>, String> {
    let pairs = db.get_all_settings().map_err(|e| e.to_string())?;
    Ok(pairs.into_iter().collect())
}

#[tauri::command]
pub fn set_setting(
    db: State<'_, Arc<Database>>,
    manager: State<'_, Arc<DataSourceManager>>,
    key: String,
    value: String,
) -> Result<(), String> {
    let policy = if key == "quote_schedule" {
        Some(crate::datasource::market_policy::MarketRequestPolicy::from_quote_schedule_json(Some(&value))?)
    } else { None };
    if key == "alerts_enabled" && value != "0" && value != "1" {
        return Err("提醒总开关只能为 0 或 1".into());
    }
    db.set_setting(&key, &value).map_err(|e| e.to_string())?;
    if let Some(policy) = policy { manager.set_request_policy(policy); }
    Ok(())
}

#[tauri::command]
pub fn switch_datasource(
    manager: State<'_, Arc<DataSourceManager>>,
    db: State<'_, Arc<Database>>,
    name: String,
) -> Result<(), String> {
    // Persist to DB first so a restart doesn't revert to the old source.
    db.set_setting("active_datasource", &name)
        .map_err(|e| e.to_string())?;
    manager.set_active(&name)
}

#[tauri::command]
pub fn list_datasources(manager: State<'_, Arc<DataSourceManager>>) -> Vec<(String, String)> {
    manager
        .list_sources()
        .into_iter()
        .map(|(id, name)| (id.to_string(), name.to_string()))
        .collect()
}

/// Query whether the app is running in portable mode (portable.dat next to exe).
#[tauri::command]
pub fn get_portable_mode(portable: State<'_, PortableMode>) -> bool {
    portable.0
}

/// Set the quote polling interval in seconds.
/// `secs == 0` means AUTO — follow the trading session's recommended cadence.
/// Any value in 1..=60 pins the interval.  The scheduler is woken immediately
/// so the change applies without waiting for the current sleep to elapse.
#[tauri::command]
pub fn set_refresh_interval(
    db: State<'_, Arc<Database>>,
    config: State<'_, Arc<PollingConfig>>,
    secs: u64,
) -> Result<(), String> {
    let secs = if secs == 0 { 0 } else { secs.clamp(1, 60) };
    db.set_setting("refresh_interval", &secs.to_string())
        .map_err(|e| e.to_string())?;
    config.set_interval_secs(secs);
    log::info!(
        "[scheduler] refresh interval set to {}",
        if secs == 0 { "auto".to_string() } else { format!("{}s", secs) }
    );
    Ok(())
}

/// Current market session plus the interval the scheduler is actually using.
/// Used by the UI to explain what "auto" resolves to right now.
#[derive(serde::Serialize)]
pub struct MarketSessionInfo {
    pub session: String,
    pub interval_secs: u64,
    pub is_trading: bool,
}

#[tauri::command]
pub fn get_market_session(config: State<'_, Arc<PollingConfig>>) -> MarketSessionInfo {
    let session = MarketSession::current();
    let configured = config.interval_secs();
    MarketSessionInfo {
        session: session.name().to_string(),
        interval_secs: if configured == 0 {
            session.recommended_interval()
        } else {
            configured
        },
        is_trading: matches!(
            session,
            MarketSession::MorningTrade | MarketSession::AfternoonTrade
        ),
    }
}
