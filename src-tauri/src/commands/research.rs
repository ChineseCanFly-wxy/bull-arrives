use crate::{
    datasource::eastmoney_universe::{
        self as market, Board, FilterCapabilities, MarketFilter, PresetInfo,
    },
    db::{
        research_loop::{assessment, ExperimentView, ResearchConfig},
        simulation::{AccountInput, Target},
        Database,
    },
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use tauri::State;
static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires logged-in Claude Code and existing debug launcher; no market network"]
    async fn claude_proposal_registers_once_with_hard_exclusions() {
        let root = std::env::temp_dir().join(format!("bull-research-smoke-{}", std::process::id()));
        let db = Database::open(root.clone()).unwrap();
        let value=crate::agent::discover_strategy(&db,serde_json::json!({"market":{"advancers":2000,"decliners":2000},"source":"synthetic smoke fixture","existing":[]})).await.unwrap();
        let id = register_proposal(&db, value.clone(), &ResearchConfig::default(), "test").unwrap();
        let rows = db.research_experiments().unwrap();
        let e = rows.iter().find(|e| e.id == id).unwrap();
        assert!(
            e.filter.exclude_st
                && e.filter.exclude_delisting
                && e.filter.exclude_suspended
                && e.filter.exclude_limit_locked
        );
        assert_eq!(e.filter.boards, vec![Board::ShMain, Board::SzMain]);
        assert!(e.account_id.is_none());
        assert!(register_proposal(&db, value, &ResearchConfig::default(), "test").is_err());
        assert!(!db.research_config().unwrap().auto_research);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn proposal_rejects_unknown_rules_and_inverted_ranges() {
        let valid = serde_json::json!({"name":"趋势候选","hypothesis":"基于主板流动性与趋势回踩构造假设，破位或量能不足即失效，尚未验证盈利能力。","rule":"trend_follow","price_min":3.0,"price_max":100.0,"turnover_min":1.0,"turnover_max":15.0,"amount_min_wan":5000.0});
        let mut p: Proposal = serde_json::from_value(valid.clone()).unwrap();
        assert!(p.validate().is_ok());
        p.price_min = 101.0;
        assert!(p.validate().is_err());
        p.rule = "execute_python".into();
        assert!(p.validate().is_err());
        let mut bad = valid;
        bad["code"] = serde_json::json!("arbitrary code");
        assert!(serde_json::from_value::<Proposal>(bad).is_err());
    }
}

#[derive(Serialize)]
pub struct ResearchDashboard {
    pub config: ResearchConfig,
    pub experiments: Vec<ExperimentView>,
    pub local_ready: bool,
    pub agent_installed: bool,
    pub last_auto_message: String,
    pub busy: bool,
}

#[derive(Serialize)]
pub struct ModeResearchReport {
    pub symbol: String,
    pub source: String,
    pub stale: bool,
    pub signals: Vec<crate::quant::modes::ModeSignal>,
}

#[tauri::command]
pub async fn research_mode_report(
    db: State<'_, Arc<Database>>,
    symbol: String,
) -> Result<ModeResearchReport, String> {
    if db.get_setting("local_history_enabled").ok().flatten().as_deref() != Some("1") {
        return Err("请先在设置中启用本地历史数据".into());
    }
    let url = db.get_setting("local_history_url").map_err(|e| e.to_string())?
        .unwrap_or_else(|| "http://127.0.0.1:7899".into());
    let config = crate::datasource::history::LocalHistoryConfig::new(url);
    let daily = crate::datasource::history::fetch_daily(&config, &symbol, None, None).await?;
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .format("%Y-%m-%d").to_string();
    let complete: Vec<_> = daily.klines.into_iter().filter(|bar| bar.date < today).collect();
    let day = complete.last().ok_or("本地数据库缺少已完成日 K")?.date.clone();
    let stale = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
        .and_then(|now| chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
            .map(|last| (now - last).num_days() > 7))
        .map_err(|e| e.to_string())?;
    let minute = crate::datasource::history::fetch_minute_day(&config, &symbol, &day).await?;
    Ok(ModeResearchReport {
        symbol,
        source: "stockdb 本地日 K + 历史 1 分钟 K".into(),
        stale,
        signals: vec![
            crate::quant::modes::swing(&complete),
            crate::quant::modes::intraday(&minute, &day),
            crate::quant::modes::long_term(&complete),
        ],
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    name: String,
    hypothesis: String,
    rule: String,
    price_min: f64,
    price_max: f64,
    turnover_min: f64,
    turnover_max: f64,
    amount_min_wan: f64,
}
impl Proposal {
    fn validate(&self) -> Result<(), String> {
        crate::quant::playbook::TradeRule::try_from_id(&self.rule)?;
        if self.name.trim().is_empty()
            || self.name.chars().count() > 40
            || !(20..=800).contains(&self.hypothesis.chars().count())
            || [
                self.price_min,
                self.price_max,
                self.turnover_min,
                self.turnover_max,
                self.amount_min_wan,
            ]
            .iter()
            .any(|v| !v.is_finite())
            || self.price_min < 1.0
            || self.price_max > 1000.0
            || self.price_min > self.price_max
            || self.turnover_min < 0.0
            || self.turnover_max > 30.0
            || self.turnover_min > self.turnover_max
            || !(1000.0..=1_000_000.0).contains(&self.amount_min_wan)
        {
            return Err("AI 候选未通过名称、逻辑、规则或参数校验".into());
        }
        Ok(())
    }
}

#[tauri::command]
pub fn research_dashboard(db: State<'_, Arc<Database>>) -> Result<ResearchDashboard, String> {
    dashboard(&db)
}
pub fn dashboard(db: &Database) -> Result<ResearchDashboard, String> {
    Ok(ResearchDashboard {
        config: db.research_config()?,
        experiments: db.experiment_views()?,
        local_ready: db
            .get_setting("local_history_enabled")
            .ok()
            .flatten()
            .as_deref()
            == Some("1"),
        agent_installed: crate::agent::status(db).installed,
        last_auto_message: db
            .get_setting("research_auto_message")
            .ok()
            .flatten()
            .unwrap_or_default(),
        busy: GATE.get().is_some_and(|g| g.available_permits() == 0),
    })
}
#[tauri::command]
pub fn save_research_config(
    db: State<'_, Arc<Database>>,
    config: ResearchConfig,
) -> Result<(), String> {
    config.validate()?;
    db.set_setting(
        "research_loop_config",
        &serde_json::to_string(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn research_discover(db: State<'_, Arc<Database>>) -> Result<i64, String> {
    discover(&db).await
}
pub async fn discover(db: &Database) -> Result<i64, String> {
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "研究任务正在运行，请稍后")?;
    let config = db.research_config()?;
    config.validate()?;
    let existing = db.research_experiments()?;
    if existing
        .iter()
        .filter(|e| {
            matches!(
                e.state.as_str(),
                "candidate" | "observing" | "extended" | "adopted"
            )
        })
        .count()
        >= config.max_active
    {
        return Err("已达到候选及验证数量上限，请先处理现有实验".into());
    }
    let snapshot = market::market_snapshot_with_fallback(std::time::Duration::from_secs(60), None)
        .await
        .map_err(|e| e.to_string())?;
    if snapshot.stale {
        return Err("市场快照陈旧，暂不生成新候选".into());
    }
    let experiments = db.experiment_views()?;
    let context = serde_json::json!({"market":crate::dynamic_filter::market_context(&snapshot.rows),"source":snapshot.source,"retrieved_at":chrono::Utc::now().to_rfc3339(),"existing":experiments.iter().take(20).map(|e|serde_json::json!({"name":e.experiment.name,"rule":e.experiment.rule,"state":e.experiment.state,"filter":e.experiment.filter,"assessment":e.experiment.last_message,"elapsed_days":e.elapsed_days,"remaining_samples":e.remaining_samples,"metrics":e.detail.as_ref().map(|d|&d.metrics)})).collect::<Vec<_>>()});
    let value = crate::agent::discover_strategy(db, context).await?;
    register_proposal(db, value, &config, "claude_code")
}

fn register_proposal(
    db: &Database,
    value: serde_json::Value,
    config: &ResearchConfig,
    source: &str,
) -> Result<i64, String> {
    let existing = db.research_experiments()?;
    if existing.len() >= 100 {
        return Err("实验记录已达到 100 条，本版暂停新增，保留现有实验与统计".into());
    }
    if existing
        .iter()
        .filter(|e| {
            matches!(
                e.state.as_str(),
                "candidate" | "observing" | "extended" | "adopted"
            )
        })
        .count()
        >= config.max_active
    {
        return Err("已达到候选及验证数量上限".into());
    }
    let proposal: Proposal =
        serde_json::from_value(value).map_err(|_| "AI 候选字段不完整或有未知字段")?;
    proposal.validate()?;
    let filter = MarketFilter {
        boards: vec![Board::ShMain, Board::SzMain],
        price_min: Some(proposal.price_min),
        price_max: Some(proposal.price_max),
        turnover_min: Some(proposal.turnover_min),
        turnover_max: Some(proposal.turnover_max),
        amount_min_wan: Some(proposal.amount_min_wan),
        ..Default::default()
    };
    let definition =
        serde_json::to_string(&serde_json::json!({"rule":proposal.rule,"filter":filter}))
            .map_err(|e| e.to_string())?;
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(definition.as_bytes()));
    if existing.iter().any(|e| {
        e.rule == proposal.rule
            && serde_json::to_string(&e.filter).ok() == serde_json::to_string(&filter).ok()
    }) {
        return Err("AI 提出了已存在的同一规则与参数，本次没有重复登记".into());
    }
    let preset = PresetInfo {
        id: format!("research_{}", &hash[..20]),
        label: proposal.name,
        description: proposal.hypothesis,
        filter,
        rule: proposal.rule,
        builtin: false,
        strategy_version_id: None,
        strategy_version: 0,
        strategy_status: String::new(),
    };
    let id = db.register_experiment(&preset, config, source)?;
    Ok(id)
}

#[tauri::command]
pub async fn research_start(
    db: State<'_, Arc<Database>>,
    experiment_id: i64,
) -> Result<(), String> {
    start(&db, experiment_id).await
}
pub async fn start(db: &Database, id: i64) -> Result<(), String> {
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "研究任务正在运行，请稍后")?;
    let experiment = db
        .research_experiments()?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or("实验不存在")?;
    if experiment.account_id.is_some() || experiment.state != "candidate" {
        return Err("该实验已有冻结账户，不能重复启动".into());
    }
    let card = db
        .strategy_library()?
        .cards
        .into_iter()
        .find(|c| c.version_id == experiment.version_id)
        .ok_or("冻结策略版本不存在")?;
    for stage in ["static", "causal"] {
        if !card
            .stages
            .iter()
            .any(|s| s.stage == stage && s.status == "passed")
        {
            return Err(format!("前置检查 {stage} 未通过，不能启动研究实验"));
        }
    }
    if db
        .get_setting("local_history_enabled")
        .ok()
        .flatten()
        .as_deref()
        != Some("1")
    {
        return Err("先在设置中启用本地历史数据并更新未复权日线，再启动模拟验证".into());
    }
    let snapshot = market::market_snapshot_with_fallback(std::time::Duration::from_secs(60), None)
        .await
        .map_err(|e| e.to_string())?;
    if snapshot.stale {
        return Err("市场快照陈旧，暂停选股".into());
    }
    let caps = FilterCapabilities::for_source(snapshot.source);
    let skipped = experiment.filter.skipped_conditions(caps);
    if !skipped.is_empty() {
        return Err(format!("当前数据源缺少策略字段：{}", skipped.join("、")));
    }
    let mut rows = experiment.filter.apply_with(&snapshot.rows, caps);
    rows.sort_by(|a, b| {
        b.amount
            .total_cmp(&a.amount)
            .then_with(|| a.code.cmp(&b.code))
    });
    rows.truncate(experiment.config.stock_count);
    if rows.is_empty() {
        return Err("当前无符合冻结条件的标的，候选保留，等待下次市场快照".into());
    }
    let targets: Vec<_> = rows
        .iter()
        .map(|r| Target {
            id: 0,
            account_id: 0,
            symbol: format!(
                "{}{}",
                if r.board == Board::ShMain { "sh" } else { "sz" },
                r.code
            ),
            name: r.name.clone(),
            rule: experiment.rule.clone(),
            limit_bps: 1000,
        })
        .collect();
    let url = db
        .get_setting("local_history_url")
        .ok()
        .flatten()
        .unwrap_or("http://127.0.0.1:7899".into());
    let local = crate::datasource::history::LocalHistoryConfig::new(url);
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .date_naive();
    let mut live_plans = Vec::new();
    for target in &targets {
        let data =
            crate::datasource::history::fetch_daily(&local, &target.symbol, None, None).await?;
        let date = data
            .raw_klines
            .last()
            .and_then(|b| chrono::NaiveDate::parse_from_str(&b.date, "%Y-%m-%d").ok())
            .ok_or("标的缺少未复权日线")?;
        if data.raw_klines.len() < 60
            || data.klines.len() < 60
            || (today - date).num_days() > 7
            || date > today
        {
            return Err(format!(
                "{} 历史样本不足或数据日期不合适，请更新本地历史",
                target.symbol
            ));
        }
        let end = data
            .klines
            .iter()
            .rposition(|b| b.date < today.to_string())
            .ok_or("缺少已完成日线")?;
        let plan = crate::quant::playbook::plan(
            &data.klines[..=end],
            crate::quant::playbook::TradeRule::try_from_id(&target.rule)?,
        )
        .ok_or("无法建立可执行买卖区间")?;
        let factor = data.raw_klines[end].close / data.klines[end].close;
        let price = |v: f64| crate::simulation_live::scaled(v * factor);
        live_plans.push(crate::db::simulation_live::LivePlan {
            symbol: target.symbol.clone(),
            buy_low: price(plan.buy_low)?,
            buy_high: price(plan.buy_high)?,
            stop: price(plan.stop_loss)?,
            take: price(plan.take_profit)?,
            limit_bps: target.limit_bps,
            basis_date: data.klines[end].date.clone(),
            reference_close: price(data.klines[end].close)?,
            position_pct: plan.position_pct,
        });
    }
    let c = &experiment.config;
    let account_input = AccountInput {
        id: None,
        name: format!("研究 {} · {}", id, experiment.name),
        initial_cash: c.initial_cash.clone(),
        mode: "auto".into(),
        auto_enabled: false,
        manual_source_enabled: false,
        rule_source_enabled: true,
        ai_source_enabled: false,
        commission_bps: c.commission_bps,
        min_commission: c.min_commission.clone(),
        stamp_tax_bps: c.stamp_tax_bps,
        transfer_fee_bps: c.transfer_fee_bps,
        slippage_bps: c.slippage_bps,
        targets,
    };
    let account = db.save_sim_account(&account_input)?;
    if c.execution_mode != "daily" {
        if let Err(error) = db.enable_live_account(account.id, &live_plans) {
            let _ = db.delete_sim_account(account.id);
            return Err(error);
        }
    }
    let selection=serde_json::to_string(&serde_json::json!({"retrieved_at":chrono::Utc::now().to_rfc3339(),"source":snapshot.source,"selection":"冻结快照条件筛选后按成交额排序","rows":rows})).map_err(|e|e.to_string())?;
    if let Err(error) = db.link_experiment(id, account.id, &selection) {
        let _ = db.delete_sim_account(account.id);
        return Err(error);
    }
    if c.execution_mode == "parallel" {
        let mut other = account_input;
        other.name = format!("日线对照 {}", id);
        let comparison = match db.save_sim_account(&other) {
            Ok(a) => a,
            Err(error) => {
                db.set_experiment_state(id, "paused", "日线对照创建失败，实时账户一起暂停")?;
                return Err(error);
            }
        };
        if let Err(e) = db.link_comparison(id, comparison.id) {
            let _ = db.delete_sim_account(comparison.id);
            db.set_experiment_state(id, "paused", "对照账户绑定失败，请检查后重试")?;
            return Err(e);
        }
    }
    Ok(())
}

pub fn evaluate(db: &Database, id: i64) -> Result<(), String> {
    let view = db
        .experiment_views()?
        .into_iter()
        .find(|v| v.experiment.id == id)
        .ok_or("实验不存在")?;
    if !matches!(
        view.experiment.state.as_str(),
        "observing" | "extended" | "adopted"
    ) {
        return Ok(());
    }
    let Some(d) = view.detail else { return Ok(()) };
    let (state, message) = assessment(
        &view.experiment.config,
        view.elapsed_days,
        d.metrics.sample_count,
        d.metrics.total_return_bps,
        d.metrics.max_drawdown_bps,
        d.metrics.benchmark_return_bps,
        view.curve.len(),
    );
    let state = if view.experiment.state == "adopted" && state == "qualified" {
        "adopted"
    } else {
        state
    };
    if state != view.experiment.state || message != view.experiment.last_message {
        db.set_experiment_state_if(id, state, &message, Some(&view.experiment.state))?;
    }
    Ok(())
}
#[tauri::command]
pub async fn research_action(
    db: State<'_, Arc<Database>>,
    manager: State<'_, Arc<crate::datasource::DataSourceManager>>,
    experiment_id: i64,
    action: String,
) -> Result<(), String> {
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "研究任务正在运行，请稍后再操作")?;
    let e = db
        .research_experiments()?
        .into_iter()
        .find(|e| e.id == experiment_id)
        .ok_or("实验不存在")?;
    match action.as_str() {
        "pause" if matches!(e.state.as_str(), "observing" | "extended" | "adopted") => {
            db.set_experiment_state(e.id, "paused", "用户暂停；保留净值、持仓及待成交指令")
        }
        "resume" if e.state == "paused" => {
            db.set_experiment_state(e.id, "observing", "用户恢复原版本验证")
        }
        "reject" => db.set_experiment_state(e.id, "rejected", "用户淘汰候选，全部历史保留"),
        "adopt" if e.state == "qualified" => {
            let v = db
                .experiment_views()?
                .into_iter()
                .find(|v| v.experiment.id == e.id)
                .ok_or("实验不存在")?;
            if v.verdict != "qualified" {
                return Err("当前数据不满足考核门槛".into());
            }
            db.set_experiment_state(
                e.id,
                "adopted",
                "用户采纳为研究策略，继续模拟跟踪；未获得实盘准入",
            )
        }
        "run" => {
            let account = e.account_id.ok_or("请先启动验证")?;
            let result = if db.live_account(account)? {
                super::simulation_live::run(&db, &manager, account).await
            } else {
                super::simulation::run_account(&db, account).await
            };
            if let Err(error) = result {
                account_failed(&db, account, &error);
                return Err(error);
            }
            if let Some(comparison) = db.comparison_account(e.id)? {
                super::simulation::run_account(&db, comparison).await?;
            }
            evaluate(&db, e.id)
        }
        _ => Err("当前状态不允许该操作".into()),
    }
}

pub async fn scheduled_tick(db: &Database) {
    if GATE.get().is_some_and(|g| g.available_permits() == 0) {
        return;
    }
    if let Ok(experiments) = db.research_experiments() {
        for e in experiments {
            if let Err(err) = evaluate(db, e.id) {
                log::warn!("[research] evaluate: {err}");
            }
        }
    }
    let Ok(config) = db.research_config() else {
        return;
    };
    if !config.auto_research || db.get_setting("ai_enabled").ok().flatten().as_deref() == Some("0")
    {
        return;
    }
    let now = chrono::Utc::now();
    let local = now.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
    use chrono::Timelike;
    if local.hour() < 16 || local.hour() >= 22 {
        return;
    }
    let schedule = db.get_setting("quote_schedule").ok().flatten();
    let Ok(policy) =
        crate::datasource::market_policy::MarketRequestPolicy::from_quote_schedule_json(
            schedule.as_deref(),
        )
    else {
        return;
    };
    if !policy.is_trading_day_at(now) {
        return;
    }
    let day = local.format("%Y-%m-%d").to_string();
    if db
        .get_setting("research_auto_day")
        .ok()
        .flatten()
        .as_deref()
        == Some(day.as_str())
    {
        return;
    }
    let _ = db.set_setting("research_auto_day", &day);
    let result = async {
        let pending = db
            .research_experiments()?
            .into_iter()
            .find(|e| e.state == "candidate" && e.account_id.is_none());
        let id = if let Some(e) = pending {
            e.id
        } else {
            discover(db).await?
        };
        start(db, id).await?;
        Ok::<_, String>(format!("{day} 已完成候选研究、选股并启动前向验证"))
    }
    .await;
    let message = result
        .unwrap_or_else(|e| format!("{day} 自动研究未完成：{e}（今日不自动重试，可手动处理）"));
    let _ = db.set_setting("research_auto_message", &message);
}

pub fn account_failed(db: &Database, account: i64, error: &str) {
    if error.contains("正在运行") {
        return;
    }
    if let Ok(items) = db.research_experiments() {
        for e in items {
            if db.comparison_account(e.id).ok().flatten() == Some(account) {
                let _ = db.set_experiment_state_if(
                    e.id,
                    "paused",
                    &format!("日线对照失败，整组暂停：{error}"),
                    Some(&e.state),
                );
                return;
            }
        }
    }
    if let Ok(items) = db.research_experiments() {
        if let Some(e) = items.into_iter().find(|e| {
            e.account_id == Some(account)
                && matches!(e.state.as_str(), "observing" | "extended" | "adopted")
        }) {
            let _ =
                db.set_experiment_state(e.id, "paused", &format!("模拟运行失败，已暂停：{error}"));
        }
    }
}

fn workspace() -> Result<std::path::PathBuf, String> {
    let root = dirs::data_local_dir()
        .ok_or("无法定位用户数据目录")?
        .join("bull-arrives")
        .join("research-workspace");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root)
}

#[derive(Serialize)]
pub struct WorkspaceInfo {
    pub path: String,
    pub skills: Vec<String>,
    pub prompt: String,
}
fn research_brief() -> String {
    "# Bull Arrives 交互研究\n\n在专用研究目录使用用户点名的已安装 Skill 或合适的研究方法。提出可证伪的 A 股假设，先完成有来源的 research_report.md 和实际历史检验，再考虑前向模拟。没有数据时写缺失，不编造收益。\n\n可研究新算法和生成 study.py；这些研究不能直接进入现有执行引擎。应用目前只支持 trend_follow、mean_reversion、breakout 的确定性交易规则。新算法不得伪装成这三种规则。\n\n确实使用既有执行规则时，可保存 candidate.json，字段严格如下：\n```json\n{\"name\":\"策略名称\",\"hypothesis\":\"说明假设、适用状态、失效条件及未验证项，20至800字\",\"rule\":\"trend_follow\",\"price_min\":3,\"price_max\":100,\"turnover_min\":1,\"turnover_max\":15,\"amount_min_wan\":5000}\n```\n\n禁止修改应用数据库、账本、订单、程序源码和原始数据。候选导入只登记待验证实验，不表示通过历史可用性门禁。研究和订单全部为模拟，不构成实盘交易。\n".into()
}
#[tauri::command]
pub fn research_workspace(db: State<'_, Arc<Database>>) -> Result<WorkspaceInfo, String> {
    prepare_workspace(&db, None)
}

fn prepare_workspace(db: &Database, topic: Option<&str>) -> Result<WorkspaceInfo, String> {
    let catalog: serde_json::Value = serde_json::from_str(include_str!("../../research-catalog.json"))
        .map_err(|e| e.to_string())?;
    let chosen = topic.map(|id| {
        catalog["strategies"].as_array().and_then(|rows| rows.iter().find(|row| row["id"] == id))
            .ok_or("未知研究方向")
    }).transpose()?;
    let engine = db.get_setting("local_history_engine_path").ok().flatten();
    let engine_dir = engine.as_deref().and_then(|path| std::path::Path::new(path).parent());
    let endpoint = db.get_setting("local_history_url").ok().flatten()
        .unwrap_or_else(|| "http://127.0.0.1:7899".into());
    let prompt = format!("{}\n\n## 本轮 A 股研究资料\n\n读取当前目录 RESEARCH_CATALOG.json，其中含一手来源、研究假设与缺失数据。选择方向：{}。\n\nstockdb 历史地址：{endpoint}；发行目录：{}。先读取发行目录下 `调用方式/python/AI策略python开发接口文档.md`，缺失时读取资料库中的官网文档；网站文档只作接口参考，Bull Arrives 保留自身 SQLite 账本，不遵循外部文档要求改存储。stockdb 仅提供历史；当前报价与盘口由应用既有通道提供。\n\n## 可复核成果\n\n1. 先核对原始论文的市场、样本期、策略定义与成本，多空学术组合不能冒充 A 股多头可实现收益。\n2. 在研究目录写 `research_report.md`：列出买入/退出原因、证据来源、数据截止日、冻结参数、训练与样本外分段、费用、基准、交易数、失败情况、无法取得的组合回撤。未经实际执行不能填收益。\n3. 需要 Python 检验时，先生成可读的 `study.py` 并解释执行内容，在 Claude 默认交互权限下运行；只读本机历史，不调用公共远端 StockDB 测试节点，不写 data/mydb 或模拟账本。保留执行命令和结果，用户可复现。\n4. 财报用公告可用时间，资讯用首次观测时间；用价格调整总收益或明确报告公司行动遗漏；避免未来函数、存续偏差与重叠交易伪组合。\n5. 任意新算法只保存研究报告与脚本，不能把价值/残差反转/事件策略硬改名为现有 trend_follow 或 mean_reversion。只有确实使用既有规则时才输出 candidate.json；该文件只是待验证候选，不代表通过历史门禁。\n", research_brief(),
        chosen.map(|row| row["name"].as_str().unwrap_or("")).unwrap_or("从资料库选择"),
        engine_dir.map(|path| path.to_string_lossy().into_owned()).unwrap_or_else(|| "尚未配置，请用户定位历史 SDK".into()));
    let root = workspace()?;
    std::fs::write(root.join("RESEARCH_CATALOG.json"), include_str!("../../research-catalog.json"))
        .map_err(|e| e.to_string())?;
    let brief = root.join("BULL_RESEARCH.md");
    std::fs::write(&brief, &prompt).map_err(|e| e.to_string())?;
    let mut skills = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if let Ok(entries) = std::fs::read_dir(home.join(".claude/skills")) {
            for entry in entries.flatten() {
                if entry.path().join("SKILL.md").is_file() {
                    skills.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    skills.sort();
    Ok(WorkspaceInfo {
        path: root.to_string_lossy().into_owned(),
        skills,
        prompt,
    })
}
#[tauri::command]
pub fn open_research_claude(db: State<'_, Arc<Database>>, topic: Option<String>) -> Result<WorkspaceInfo, String> {
    let info = prepare_workspace(&db, topic.as_deref())?;
    let status = crate::agent::status(&db);
    let path = status
        .path
        .filter(|_| status.installed)
        .ok_or(status.message)?;
    #[cfg(windows)]
    {
        // Windows Terminal owns the interactive console; a GUI parent has no usable stdin.
        let terminal = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("Microsoft/WindowsApps/wt.exe"))
            .filter(|p| p.is_file())
            .ok_or_else(|| {
                format!(
                    "未检测到 Windows Terminal，请在终端进入 {} 后运行 claude",
                    info.path
                )
            })?;
        let mut command = std::process::Command::new(terminal);
        command.args(["-w","new","new-tab","--title","Bull Arrives 研究","-d"]).arg(&info.path).arg(path).args(["--permission-mode","default","请先读取当前目录 BULL_RESEARCH.md 和 RESEARCH_CATALOG.json，说明所选研究方向的来源、历史数据是否足够、研究与回测计划，然后等待我确认具体假设。"]);
        command
            .spawn()
            .map_err(|e| format!("无法打开 Claude 交互窗口：{e}"))?;
        Ok(info)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(format!(
            "请在终端进入 {} 后运行 claude；当前自动开窗仅支持 Windows",
            info.path
        ))
    }
}
#[cfg(test)]
mod catalog_tests {
    #[test]
    fn catalog_has_unique_topics_and_primary_links() {
        let value: serde_json::Value = serde_json::from_str(include_str!("../../research-catalog.json")).unwrap();
        let topics = value["strategies"].as_array().unwrap();
        let mut ids = std::collections::HashSet::new();
        for item in topics {
            let id = item["id"].as_str().unwrap();
            assert!(ids.insert(id));
            assert!(item["url"].as_str().unwrap().starts_with("https://"));
            assert!(!item["limitation"].as_str().unwrap().is_empty());
        }
    }
}
#[tauri::command]
pub fn import_research_candidate(db: State<'_, Arc<Database>>) -> Result<i64, String> {
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "研究任务正在运行")?;
    let file = workspace()?.join("candidate.json");
    let metadata = std::fs::symlink_metadata(&file)
        .map_err(|_| "尚无 candidate.json，请让 Claude 完成候选文件")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16384 {
        return Err("候选必须是 16 KiB 以内的普通 JSON 文件".into());
    }
    let value = serde_json::from_str(&std::fs::read_to_string(file).map_err(|e| e.to_string())?)
        .map_err(|_| "候选 JSON 格式错误")?;
    register_proposal(&db, value, &db.research_config()?, "claude_interactive")
}
