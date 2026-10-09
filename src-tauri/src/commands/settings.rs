use crate::agent::interactive::InteractiveRoot;
use crate::cache::PollingConfig;
use crate::datasource::history::{self, LocalHistoryConfig};
use crate::datasource::market_clock::MarketSession;
use crate::datasource::DataSourceManager;
use crate::db::Database;
use crate::stockdb::StockDbManager;
use crate::PortableMode;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;

fn validate_notification_setting(key: &str, value: &str) -> Result<(), String> {
    if matches!(key, "alerts_enabled" | "research_notifications_enabled" | "mainline_notifications_enabled" | "model_trade_notifications_enabled" | "model_condition_notifications_enabled" | "risk_notifications_enabled" | "daily_briefs_enabled" | "important_alerts_enabled" | "important_alerts_native_enabled" | "news_system_notifications_enabled" | "notification_desktop_always") && value != "0" && value != "1" {
        return Err("提醒开关只能为 0 或 1".into());
    }
    if matches!(key, "important_alerts_text_color" | "important_alerts_background") && !(value.len() == 7 && value.starts_with('#') && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)) {
        return Err("颜色必须为 #RRGGBB".into());
    }
    if key == "important_alerts_opacity" && !value.parse::<u32>().is_ok_and(|v| (5..=100).contains(&v)) {
        return Err("透明度必须为 5–100 的整数".into());
    }
    Ok(())
}

fn validate_visual_style(value: &str) -> Result<(), String> {
    if matches!(value, "classic" | "modern" | "elegant") { Ok(()) }
    else { Err("界面风格只能为 classic、modern 或 elegant".into()) }
}

fn validate_manual_news_and_provider_setting(key: &str, value: &str) -> Result<(), String> {
    if key == "news_notification_mode" && value != "direct" {
        return Err("资讯只支持原文直接通知；AI 解读请手动点击".into());
    }
    if matches!(key, "quote_schedule" | "quote_schedule_enabled") {
        return Err("行情时间限制已移除；行情自动按市场状态调整刷新频率".into());
    }
    if matches!(key, "news_ai_daily_limit" | "news_ai_keywords") {
        return Err("资讯自动 AI 解读设置已移除".into());
    }
    if key == "agent_provider" && value != "claude" {
        return Err("本项目仅支持 Claude Code".into());
    }
    if key.starts_with("agent_codex_") {
        return Err("Codex 设置已移除；请使用 Claude Code".into());
    }
    Ok(())
}

#[cfg(test)]
mod manual_news_setting_tests {
    use super::validate_manual_news_and_provider_setting as validate;

    #[test]
    fn independent_notification_settings_reject_invalid_switches_colors_and_opacity() {
        for key in ["alerts_enabled", "mainline_notifications_enabled", "model_trade_notifications_enabled", "model_condition_notifications_enabled", "research_notifications_enabled", "risk_notifications_enabled", "daily_briefs_enabled", "important_alerts_enabled", "important_alerts_native_enabled", "news_system_notifications_enabled"] {
            assert!(super::validate_notification_setting(key,"0").is_ok());
            assert!(super::validate_notification_setting(key,"1").is_ok());
            assert!(super::validate_notification_setting(key,"true").is_err());
        }
        for key in ["important_alerts_text_color", "important_alerts_background"] {
            assert!(super::validate_notification_setting(key,"#aA09Ff").is_ok());
            for value in ["red", "#fff", "#GG0000", "#aA09Ff;", "#色000"] { assert!(super::validate_notification_setting(key,value).is_err()); }
        }
        for value in ["5","95","100"] { assert!(super::validate_notification_setting("important_alerts_opacity",value).is_ok()); }
        for value in ["0","4","101","NaN","5.5", "99999999999999"] { assert!(super::validate_notification_setting("important_alerts_opacity",value).is_err()); }
    }
    #[test]
    fn visual_style_accepts_saved_elegant_and_rejects_unknown_values() {
        for style in ["classic", "modern", "elegant"] { assert!(super::validate_visual_style(style).is_ok()); }
        for style in ["", "trading", "ELEGANT", "unknown"] { assert!(super::validate_visual_style(style).is_err()); }
    }

    #[test]
    fn rejects_removed_auto_modes_and_codex_without_restricting_manual_claude() {
        assert!(validate("news_notification_mode", "direct").is_ok());
        for mode in ["ai", "hybrid", "unexpected"] {
            assert!(validate("news_notification_mode", mode).is_err());
        }
        assert!(validate("quote_schedule_enabled", "1").is_err());
        assert!(validate("quote_schedule_enabled", "0").is_err());
        assert!(validate("quote_schedule", "not-json").is_err());
        assert!(validate("news_ai_daily_limit", "0").is_err());
        assert!(validate("news_ai_keywords", "").is_err());
        assert!(validate("agent_provider", "claude").is_ok());
        assert!(validate("agent_provider", "codex").is_err());
        assert!(validate("agent_codex_model", "").is_err());
        assert!(validate("agent_claude_path", "").is_ok());
        assert!(validate("agent_budget_usd", "10.00").is_ok());
    }
}

fn validate_local_history_url(value: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(value.trim()).map_err(|_| "本地历史服务地址格式无效")?;
    if url.scheme() != "http" || url.username() != "" || url.password().is_some() {
        return Err("本地历史服务只允许无凭据的 http 地址".into());
    }
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err("本地历史服务只允许 127.0.0.1 / localhost / ::1".into());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("本地历史服务地址不能包含路径、查询参数或片段".into());
    }
    url.set_path("");
    Ok(url.to_string().trim_end_matches('/').to_string())
}

#[derive(serde::Serialize)]
pub struct LocalHistoryStatus {
    pub state: String,
    pub message: String,
    pub source_format: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub sample_count: Option<usize>,
    pub candidates: Vec<String>,
}

fn local_history_candidates(configured: Option<&str>) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(path) = configured.filter(|path| !path.trim().is_empty()) {
        paths.push(std::path::PathBuf::from(path.trim()));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("stockdb").join("stockdb"));
        if let Some(parent) = cwd.parent() {
            paths.push(parent.join("stockdb").join("stockdb"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.to_path_buf());
            paths.push(dir.join("stockdb"));
        }
    }
    let mut found = Vec::new();
    for path in paths {
        if path.join("stockdb.exe").is_file() {
            let display = std::fs::canonicalize(&path)
                .unwrap_or(path)
                .display()
                .to_string();
            if !found.contains(&display) {
                found.push(display);
            }
        }
    }
    found
}

async fn check_local_history(url: &str, candidates: Vec<String>) -> LocalHistoryStatus {
    let url = match validate_local_history_url(url) {
        Ok(url) => url,
        Err(message) => {
            return LocalHistoryStatus {
                state: "unavailable".into(),
                message,
                source_format: None,
                start_date: None,
                end_date: None,
                sample_count: None,
                candidates,
            }
        }
    };
    match history::fetch_daily(
        &LocalHistoryConfig::new(url),
        "600519",
        None,
        None,
    )
    .await
    {
        Ok(result) if result.sample_count > 0 => LocalHistoryStatus {
            state: "connected".into(),
            message: "本地 stockdb 可用".into(),
            source_format: Some(format!("{:?}", result.protocol)),
            start_date: result.start_date,
            end_date: result.end_date,
            sample_count: Some(result.sample_count),
            candidates,
        },
        Ok(_) => LocalHistoryStatus {
            state: "unavailable".into(),
            message: "服务已连接，但测试股票没有历史数据".into(),
            source_format: None,
            start_date: None,
            end_date: None,
            sample_count: Some(0),
            candidates,
        },
        Err(message) => LocalHistoryStatus {
            state: if candidates.is_empty() {
                "not_found"
            } else {
                "not_started"
            }
            .into(),
            message,
            source_format: None,
            start_date: None,
            end_date: None,
            sample_count: None,
            candidates,
        },
    }
}

#[tauri::command]
pub async fn get_local_history_status(
    db: State<'_, Arc<Database>>,
) -> Result<LocalHistoryStatus, String> {
    let url = db
        .get_setting("local_history_url")
        .ok()
        .flatten()
        .unwrap_or_else(|| "http://127.0.0.1:7899".into());
    let configured = db.get_setting("local_history_engine_dir").ok().flatten();
    Ok(check_local_history(&url, local_history_candidates(configured.as_deref())).await)
}

#[tauri::command]
pub async fn test_local_history(url: String) -> LocalHistoryStatus {
    check_local_history(&url, Vec::new()).await
}

#[tauri::command]
pub async fn scan_local_history(
    db: State<'_, Arc<Database>>,
) -> Result<LocalHistoryStatus, String> {
    let url = db
        .get_setting("local_history_url")
        .ok()
        .flatten()
        .unwrap_or_else(|| "http://127.0.0.1:7899".into());
    let configured = db.get_setting("local_history_engine_dir").ok().flatten();
    Ok(check_local_history(&url, local_history_candidates(configured.as_deref())).await)
}

#[tauri::command]
pub fn get_settings(db: State<'_, Arc<Database>>) -> Result<HashMap<String, String>, String> {
    let pairs = db.get_all_settings().map_err(|e| e.to_string())?;
    Ok(pairs.into_iter().collect())
}

#[tauri::command]
pub fn set_setting(
    app: tauri::AppHandle,
    operations: State<'_, Arc<crate::operations_log::OperationsLog>>,
    db: State<'_, Arc<Database>>,
    stockdb: State<'_, Arc<StockDbManager>>,
    key: String,
    value: String,
) -> Result<(), String> {
    if matches!(
        key.as_str(),
        "local_history_enabled"
            | "local_history_engine_dir"
            | "local_history_engine_path"
            | "local_history_updater_path"
    ) {
        return Err("本地 stockdb 配置必须通过专用设置操作修改".into());
    }
    validate_notification_setting(&key, &value)?;
    if key==crate::important_alerts::PENDING_KEY { return Err("未确认提醒由后台维护，不能手动覆盖".into()); }
    if key==crate::stockdb_schedule::RECORD_KEY{return Err("更新进度由后台任务维护，不能手动覆盖".into());}
    if key=="research_data_alerts_enabled"{return Err("缺行情提醒已移除；更新失败原因可在数据状态与自动操作记录查看".into());}
    if key=="local_history_auto_update_enabled" && !matches!(value.as_str(),"0"|"1"){return Err("自动更新开关只能为0或1".into());}
    if key=="local_history_auto_update_time"{crate::stockdb_schedule::validate_time(&value)?;}
    // AI / 量化智能相关开关同样只允许 0/1，避免前端写入其它值后判断语义含糊。
    if (key == "ai_enabled"
        || key == "ai_monitor_enabled"
        || key == "local_history_enabled"
        || key == "news_notifications_enabled"
        || key == crate::operations_log::SETTING)
        && value != "0"
        && value != "1"
    {
        return Err("开关值只能为 0 或 1".into());
    }
    validate_manual_news_and_provider_setting(&key, &value)?;
    let value = if key == "local_history_url" {
        validate_local_history_url(&value)?
    } else if key == "local_history_engine_dir" && !value.trim().is_empty() {
        let path = std::path::Path::new(value.trim());
        if !path.is_dir() || !path.join("stockdb.exe").is_file() {
            return Err("引擎目录不存在，或目录中没有 stockdb.exe".into());
        }
        value.trim().to_string()
    } else {
        value
    };
    if key == "theme" && !matches!(value.as_str(), "light" | "dark") {
        return Err("明暗模式只能为 light 或 dark".into());
    }
    if key == "visual_style" { validate_visual_style(&value)?; }
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
        let size = value
            .parse::<u32>()
            .map_err(|_| "悬浮窗每页数量必须为整数")?;
        if !(1..=20).contains(&size) {
            return Err("悬浮窗每页数量必须在 1–20 之间".into());
        }
    }
    let value = if key == "agent_budget_usd" {
        let budget = value.parse::<f64>().map_err(|_| "Agent 单次预算必须为有效金额")?;
        if !budget.is_finite() || !(10.0..=50.0).contains(&budget) {
            return Err("Agent 单次预算必须在 10–50 美元之间".into());
        }
        format!("{budget:.2}")
    } else if key == "agent_claude_path" && !value.trim().is_empty() {
        crate::agent::validate_claude_path(&value)?
    } else if key == "agent_timeout_seconds" {
        let seconds = value.parse::<u64>().map_err(|_| "Agent 超时必须为整数秒")?;
        if !(15..=300).contains(&seconds) {
            return Err("Agent 超时必须在 15–300 秒之间".into());
        }
        seconds.to_string()
    } else if key == "agent_run_root" {
        // 工作目录必须存在、可写，且不含 8.3 短名；空值表示回到自动。
        crate::agent::validate_run_root(&value)?
    } else {
        value
    };
    if key=="universe_preset" && !matches!(value.as_str(),"all"|"custom") {
        return Err("旧市场策略已退役，筛选条件只允许基础池或手动条件".into());
    }
    if key == "local_history_url" {
        return stockdb.set_service_url(&value);
    }
    db.set_setting(&key, &value).map_err(|e| e.to_string())?;
    if key == "important_alerts_enabled" { crate::important_alerts::refresh_visibility(&app, value == "1")?; }
    if key == "important_alerts_opacity" {
        if let Some(window) = tauri::Manager::get_webview_window(&app, "important-alerts") {
            let opacity = value.parse::<u32>().map_err(|e| e.to_string())?;
            crate::apply_ticker_opacity(&window, ((opacity as f64 / 100.0) * 255.0).round() as u8).map_err(|e| format!("透明度已保存，但窗口应用失败：{e}"))?;
        }
    }
    if key == crate::operations_log::SETTING {
        operations.set_enabled(value=="1");
    }
    // 每次重新开启先建立当前资讯水位，避免停用期间积压内容集中弹出。
    if key == "news_notifications_enabled" && value == "1" {
        db.set_setting("news_flash_initialized", "0")
            .map_err(|e| e.to_string())?;
        db.set_setting("news_announcement_initialized", "0")
            .map_err(|e| e.to_string())?;
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

#[derive(serde::Serialize)]
pub struct DataPaths {
    pub data_dir: String,
    pub database: String,
    pub interactive_tasks: String,
}

#[tauri::command]
pub fn get_data_paths(root: State<'_, InteractiveRoot>) -> Result<DataPaths, String> {
    let data_dir = root.0.parent().ok_or("数据目录不存在")?;
    Ok(DataPaths {
        data_dir: data_dir.to_string_lossy().into_owned(),
        database: data_dir.join("bull-arrives.db").to_string_lossy().into_owned(),
        interactive_tasks: root.0.to_string_lossy().into_owned(),
    })
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
        if secs == 0 {
            "auto".to_string()
        } else {
            format!("{}s", secs)
        }
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
    let message = if trimmed.len() > 2000 {
        &trimmed[..2000]
    } else {
        trimmed
    };
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
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .to_string(),
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
    /// 交易日历当前依据（由指数行情推断，不再是手工录入的年度表）。
    pub calendar: String,
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
        calendar: crate::datasource::trading_calendar::status_text(),
    }
}
