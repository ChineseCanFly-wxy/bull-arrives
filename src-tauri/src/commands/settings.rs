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
    if key == "quote_schedule_enabled" && value != "0" && value != "1" {
        return Err("行情时间限制开关只能为 0 或 1".into());
    }
    // AI / 量化智能相关开关同样只允许 0/1，避免前端写入其它值后判断语义含糊。
    if (key == "ai_enabled" || key == "ai_monitor_enabled") && value != "0" && value != "1" {
        return Err("AI 智能开关只能为 0 或 1".into());
    }
    let policy_on_enable = if key == "quote_schedule_enabled" && value == "1" {
        let schedule = db.get_setting("quote_schedule").map_err(|e| e.to_string())?;
        Some(crate::datasource::market_policy::MarketRequestPolicy::from_quote_schedule_json(schedule.as_deref())?)
    } else {
        None
    };
    if key == "ticker_display_mode" && value != "carousel" && value != "fixed" {
        return Err("悬浮窗展示方式只能为 carousel 或 fixed".into());
    }
    if key == "universe_source" && !matches!(value.as_str(), "auto" | "sina" | "eastmoney") {
        return Err("全市场数据源只能为 auto / sina / eastmoney".into());
    }
    if key == "universe_page_size" {
        let size = value
            .parse::<u32>()
            .map_err(|_| "筛选结果每页条数必须为整数")?;
        if !(1..=100).contains(&size) {
            return Err("筛选结果每页条数必须在 1–100 之间".into());
        }
    }
    if key == "ticker_page_size" {
        let size = value.parse::<u32>().map_err(|_| "悬浮窗每页数量必须为整数")?;
        if !(1..=20).contains(&size) {
            return Err("悬浮窗每页数量必须在 1–20 之间".into());
        }
    }
    db.set_setting(&key, &value).map_err(|e| e.to_string())?;
    if let Some(policy) = policy { manager.set_request_policy(policy); }
    if let Some(policy) = policy_on_enable { manager.set_request_policy(policy); }
    if key == "quote_schedule_enabled" {
        manager.set_request_policy_enabled(value == "1");
    }
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

/// 前端日志通道：把 JS 侧的错误写进 bull-arrives.log。
///
/// 背景：前端 `console.error` 只进 WebView 的开发者控制台，用户看不到、我们也拿不到。
/// 排查「界面没反应但后端日志一片干净」这类问题时，必须有这条通道。
#[tauri::command]
pub fn log_frontend(level: String, message: String) {
    let trimmed = message.trim();
    // 单条上限，防止异常数据把日志撑爆
    let message = if trimmed.len() > 2000 { &trimmed[..2000] } else { trimmed };
    match level.as_str() {
        "error" => log::error!("[前端] {message}"),
        "warn" => log::warn!("[前端] {message}"),
        _ => log::info!("[前端] {message}"),
    }
}

/// 构建信息。用于在界面上确认「当前跑的是不是最新版本」——
/// 之前排查问题时反复卡在「用户跑的到底是哪个 exe」上，直接显示出来最省事。
#[derive(serde::Serialize)]
pub struct BuildInfo {
    pub version: String,
    /// 可执行文件的修改时间，等价于构建完成时间
    pub built_at: String,
    pub profile: String,
    /// 可执行文件完整路径，便于发现「其实在跑另一个副本」
    pub exe_path: String,
}

#[tauri::command]
pub fn get_build_info() -> BuildInfo {
    let exe = std::env::current_exe().ok();
    let built_at = exe
        .as_ref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok())
        .map(|time| {
            chrono::DateTime::<chrono::Local>::from(time)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "未知".to_string());

    BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        built_at,
        profile: if cfg!(debug_assertions) { "debug" } else { "release" }.to_string(),
        exe_path: exe
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "未知".to_string()),
    }
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
