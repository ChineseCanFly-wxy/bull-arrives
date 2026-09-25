use super::*;
use std::sync::Mutex;

pub fn workflow(task: &str, role: &str) -> &'static str {
    match (task, role) {
        ("strategy_discovery", _) => include_str!("../../prompts/strategy-discovery.md"),
        ("single_stock_analysis", _) => include_str!("../../prompts/single-stock.md"),
        ("multi_role_analysis", "technical") => include_str!("../../prompts/technical.md"),
        ("multi_role_analysis", "bull") => include_str!("../../prompts/bull.md"),
        ("multi_role_analysis", "bear") => include_str!("../../prompts/bear.md"),
        ("multi_role_analysis", "risk") => include_str!("../../prompts/risk.md"),
        ("news_summary", _) => include_str!("../../prompts/news.md"),
        ("dynamic_filter", _) => include_str!("../../prompts/filter.md"),
        _ => include_str!("../../prompts/connection.md"),
    }
}

pub fn prompt_text(task: &str, role: &str) -> String {
    format!("{}\n\n{}", PROMPT, workflow(task, role))
}

pub fn prompt_hash(task: &str, role: &str) -> String {
    sha256(prompt_text(task, role).as_bytes())
}

#[derive(Clone, Serialize)]
pub struct PromptView {
    pub id: String,
    pub label: String,
    pub content: String,
    pub hash: String,
}

#[derive(Serialize)]
pub struct AgentInspection {
    pub prompts: Vec<PromptView>,
    pub input_json: String,
    pub schema_json: String,
    pub missing_data: Vec<String>,
    pub timeout_seconds: u64,
    pub budget_usd: String,
    pub executable: Option<String>,
    pub history_summary: String,
}

pub fn inspect(db: &Database, fingerprint: &str) -> Result<AgentInspection, String> {
    let input_json = db
        .get_agent_snapshot(fingerprint)?
        .ok_or("快照不存在，请重新打开分析")?;
    let input: FrozenInput = serde_json::from_str(&input_json).map_err(|_| "快照结构无效")?;
    if frozen_fingerprint(&input)? != fingerprint {
        return Err("快照指纹校验失败".into());
    }
    let prompts = [
        ("single_stock_analysis", "", "单股解读"),
        ("multi_role_analysis", "technical", "技术分析师"),
        ("multi_role_analysis", "bull", "多方研究员"),
        ("multi_role_analysis", "bear", "空方研究员"),
        ("multi_role_analysis", "risk", "风控综合"),
        ("news_summary", "", "资讯摘要"),
        ("dynamic_filter", "", "动态筛选"),
    ]
    .into_iter()
    .map(|(task, role, label)| PromptView {
        id: format!("{task}:{role}"),
        label: label.into(),
        content: prompt_text(task, role),
        hash: prompt_hash(task, role),
    })
    .collect();
    let mut missing_data = vec![
        "实时盘口与逐笔成交".into(),
        "财报、公告、新闻与资金流（本次个股任务未提供）".into(),
        "个人持仓和账户资金".into(),
    ];
    if input.recent_klines.is_empty() {
        missing_data.push("逐根 K 线未提供（旧快照请重新打开分析）".into());
    }
    let history_summary = match (input.recent_klines.first(), input.recent_klines.last()) {
        (Some(first), Some(last)) => format!(
            "{} 根前复权日 K · {} 至 {} · {}",
            input.recent_klines.len(),
            first.date,
            last.date,
            input.source
        ),
        _ => "无逐根 K 线，只有计算结果".into(),
    };
    for (field, label) in [
        ("ma20", "短期均线"),
        ("ma60", "长期均线"),
        ("rsi12", "RSI"),
        ("volume_ratio", "量比"),
    ] {
        if !input.evidence.contains_key(field) {
            missing_data.push(format!("{label}：数据不足"));
        }
    }
    if input
        .quantitative_analysis
        .history
        .as_ref()
        .map_or(true, |h| h.stale)
    {
        missing_data.push("历史来源缺失或已标记陈旧".into());
    }
    if input
        .quantitative_analysis
        .backtest
        .as_ref()
        .map_or(true, |b| !b.trust.eligible)
    {
        missing_data.push("回测缺失或尚未通过实战准入门禁".into());
    }
    Ok(AgentInspection {
        prompts,
        input_json,
        schema_json: OUTPUT_SCHEMA.into(),
        missing_data,
        timeout_seconds: db
            .get_setting("agent_timeout_seconds")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(15, 300),
        budget_usd: agent_budget(db),
        executable: status(db).path,
        history_summary,
    })
}

#[derive(Clone, Serialize)]
pub struct LiveRun {
    pub fingerprint: String,
    pub task: String,
    pub role: String,
    pub round: u8,
    pub process_id: Option<u32>,
    pub elapsed_ms: u64,
    pub timeout_seconds: u64,
}

static LIVE: OnceLock<Mutex<Option<(LiveRun, Instant)>>> = OnceLock::new();
fn live() -> &'static Mutex<Option<(LiveRun, Instant)>> {
    LIVE.get_or_init(|| Mutex::new(None))
}
pub fn set_live(value: LiveRun) {
    *live().lock().unwrap_or_else(|e| e.into_inner()) = Some((value, Instant::now()));
}
pub fn set_pid(pid: u32) {
    if let Some((value, _)) = live().lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        value.process_id = Some(pid);
    }
}
pub fn clear_live() {
    *live().lock().unwrap_or_else(|e| e.into_inner()) = None;
}
pub fn current(fingerprint: &str) -> Option<LiveRun> {
    live()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|(v, _)| v.fingerprint == fingerprint)
        .map(|(v, started)| {
            let mut value = v.clone();
            value.elapsed_ms = started.elapsed().as_millis() as u64;
            value
        })
}

#[derive(Serialize)]
pub struct Activity {
    pub current: Option<LiveRun>,
    pub runs: Vec<crate::db::agent::AgentRunRecord>,
}

pub fn activity(db: &Database, fingerprint: &str) -> Result<Activity, String> {
    let current = current(fingerprint);
    let mut runs = db.agent_run_history(fingerprint)?;
    if current.is_none() {
        for run in &mut runs {
            if run.status == "running" {
                run.status = "interrupted".into();
            }
        }
    }
    Ok(Activity { current, runs })
}
