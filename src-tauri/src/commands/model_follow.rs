//! Executable follow plans bound to the existing frozen daily models.
use crate::{
    datasource::DataSourceManager,
    db::{
        model_follow::{AutomaticRecord, FollowBinding, FollowOrderMeta, AUTOMATIC_ENABLED_KEY},
        simulation::{OrderInput, SimDetail, SimOrder},
        Database,
    },
    simulation::{Side, SCALE},
    simulation_live::{scaled, LiveTick},
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};
use tauri::State;
#[cfg(test)]
use crate::db::simulation::AccountInput;
static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
static SETUP_GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
fn gate() -> &'static tokio::sync::Semaphore {
    GATE.get_or_init(|| tokio::sync::Semaphore::new(1))
}
// Tokio's queued acquire prevents the next background pass from overtaking a user action.
async fn manual_permit(
    semaphore: &tokio::sync::Semaphore,
) -> Result<tokio::sync::SemaphorePermit<'_>, String> {
    tokio::time::timeout(std::time::Duration::from_secs(30), semaphore.acquire())
        .await
        .map_err(|_| "原模型或行情仍在更新，请刷新后重试；未提交委托".to_string())?
        .map_err(|_| "跟随调度已停止".to_string())
}
pub(crate) async fn adjust_account_capital(db: &Database, input: &crate::db::simulation::CapitalAdjustmentInput) -> Result<crate::db::simulation::SimAccount, String> {
    let _permit = manual_permit(gate()).await?;
    db.adjust_sim_capital(input)
}

async fn snapshot_slot(
    manager: &DataSourceManager,
    symbol: Option<&String>,
) -> Option<Result<LiveTick, String>> {
    let symbol = symbol?;
    Some(
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            super::simulation_live::snapshot(manager, symbol),
        )
        .await
        .unwrap_or_else(|_| Err("盘口读取超时，等待下一轮新报价".into())),
    )
}
async fn held_ticks(
    manager: &DataSourceManager,
    symbols: &[String],
) -> Result<(Vec<LiveTick>, Vec<String>), String> {
    if symbols.len() > 10 {
        return Err("跟随账户关联股票超过10只，等待名额核对".into());
    }
    let results = tokio::join!(
        snapshot_slot(manager, symbols.first()),
        snapshot_slot(manager, symbols.get(1)),
        snapshot_slot(manager, symbols.get(2)),
        snapshot_slot(manager, symbols.get(3)),
        snapshot_slot(manager, symbols.get(4)),
        snapshot_slot(manager, symbols.get(5)),
        snapshot_slot(manager, symbols.get(6)),
        snapshot_slot(manager, symbols.get(7)),
        snapshot_slot(manager, symbols.get(8)),
        snapshot_slot(manager, symbols.get(9)),
    );
    let mut ticks = Vec::new();
    let mut messages = Vec::new();
    for (symbol, result) in symbols.iter().zip([
        results.0, results.1, results.2, results.3, results.4, results.5, results.6, results.7,
        results.8, results.9,
    ]) {
        match result {
            Some(Ok(tick)) => ticks.push(tick),
            Some(Err(error)) => messages.push(format!("{symbol}：{error}")),
            None => {}
        }
    }
    Ok((ticks, messages))
}
fn cst() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(28800).unwrap()
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextRow {
    pub symbol: String,
    pub close: Option<f64>,
    pub adjusted_close: Option<f64>,
    pub atr14: Option<f64>,
    #[serde(default)]
    pub ma10: Option<f64>,
    pub factor: f64,
    pub amount: Option<f64>,
    pub valid: bool,
    pub is_st: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MarketContext {
    pub as_of: String,
    pub csi300_return5: Option<f64>,
    pub csi300_ma60_deviation: Option<f64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FollowContext {
    pub schema: String,
    pub as_of: String,
    pub model_id: String,
    pub model_sha256: String,
    pub source_run_sha256: String,
    pub source_runner_sha256: String,
    pub context_runner_sha256: String,
    pub data_sha256: Value,
    pub session_dates: Vec<String>,
    pub ranked_symbols: Vec<String>,
    #[serde(default)]
    pub market: Option<MarketContext>,
    pub rows: Vec<ContextRow>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowStart {
    pub source_run_id: i64,
    pub initial_cash_cny: f64,
    #[serde(default)]
    pub max_positions: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowSetup {
    pub model_id: String,
    pub initial_cash_cny: f64,
    #[serde(default)]
    pub max_positions: Option<i64>,
}

fn validate_follow_values(cash: f64, maximum: i64) -> Result<(), String> {
    if !(1..=10).contains(&maximum)
        || !cash.is_finite()
        || cash < 1000.
        || cash > 100000000.
        || (cash * 100. - (cash * 100.).round()).abs() > 1e-5
    {
        return Err("资金应为1000至1亿元、最多两位小数，最多持股数应为1至10".into());
    }
    Ok(())
}

fn validate_setup(input: &FollowSetup) -> Result<i64, String> {
    if !matches!(
        input.model_id.as_str(),
        "breadth22_h20"
            | "index26_h20"
            | "breadth22_excess_csi20"
            | "breadth22_rank20"
            | "breadth22_open_downside20"
    ) {
        return Err("一键启动仅支持已有五个冻结模型".into());
    }
    let maximum = input
        .max_positions
        .unwrap_or_else(|| recommended_slots(&input.model_id));
    validate_follow_values(input.initial_cash_cny, maximum)?;
    Ok(maximum)
}

fn money_scaled(value: f64) -> Result<i64, String> {
    if !value.is_finite()
        || value < 0.
        || value * SCALE as f64 > crate::simulation::MAX_MONEY as f64
    {
        return Err("资金或成交额超出安全范围".into());
    }
    Ok((value * SCALE as f64).round() as i64)
}
fn sell_limit(tick: &LiveTick) -> Result<i64, String> {
    let bid = tick
        .depth
        .bids
        .iter()
        .filter(|l| l.price.is_finite() && l.price > 0. && l.volume > 0)
        .max_by(|a, b| a.price.total_cmp(&b.price))
        .ok_or("当前无可见买一，等待可成交盘口")?;
    Ok(((scaled(bid.price)? as i128 * 9990 / 10000) / 100 * 100) as i64)
}
fn recommended_slots(model: &str) -> i64 {
    execution_evidence()["models"][model]["recommendation"]["default_max_positions"]
        .as_i64()
        .filter(|n| (1..=10).contains(n))
        .unwrap_or(3)
}
#[derive(Deserialize)]
struct ModelExecution {
    holding_sessions: i64,
    entry_atr_multiple: f64,
    max_gap_pct: f64,
}
fn model_execution(model: &str) -> Result<ModelExecution, String> {
    if !super::research_jobs::DAILY_MODEL_IDS.contains(&model) {
        return Err("该模型不属于当前已验证的日线自动交易；盘中信号模型须单独验证并使用独立执行入口".into());
    }
    serde_json::from_value(
        execution_evidence()["models"][model]["recommendation"]["execution"].clone(),
    )
    .map_err(|_| "本模型缺少独立核对的执行配置".into())
}
fn date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| "模型日期无效".into())
}
fn source(db: &Database, id: i64) -> Result<Value, String> {
    let v = super::research::check_bound_run(&db.model_run_bundle(id)?)?;
    if v["mode"] != "forward"
        || v["comparison"] != "baseline"
        || v["holding_days"] != 20
        || !v["model_id"].as_str().is_some_and(|id| super::research_jobs::DAILY_MODEL_IDS.contains(&id))
    {
        return Err("跟随账户只使用已有五个冻结技术模型的20日基准前向账户".into());
    }
    Ok(v)
}
fn validate_context(raw: &str, run: &Value) -> Result<FollowContext, String> {
    let envelope: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let body = envelope["content"].as_str().ok_or("执行上下文缺少内容")?;
    if envelope["schema"] != "model-follow-context-bundle-v1"
        || envelope["content_sha256"] != hex::encode(Sha256::digest(body.as_bytes()))
    {
        return Err("执行上下文指纹不符".into());
    }
    let ctx: FollowContext = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if ctx.schema != "frozen-model-execution-v1"
        || ctx.as_of != run["as_of"]
        || ctx.model_id != run["model_id"]
        || ctx.model_sha256 != run["model_sha256"]
        || ctx.source_run_sha256 != run["content_sha256"]
        || ctx.source_runner_sha256 != run["runner_sha256"]
        || ctx.data_sha256 != run["data_sha256"]
        || ctx.context_runner_sha256
            != hex::encode(Sha256::digest(include_bytes!(
                "../../../research/research-center-runner/follow_context.py"
            )))
    {
        return Err("操作计划与原模型、日线数据或程序版本不一致".into());
    }
    if ctx.session_dates.is_empty()
        || ctx.session_dates.last() != Some(&ctx.as_of)
        || ctx.session_dates.windows(2).any(|w| w[0] >= w[1])
        || ctx.rows.len() > 10000
    {
        return Err("执行上下文股票/交易日轴无效".into());
    }
    for d in &ctx.session_dates {
        date(d)?;
    }
    let mut symbols = HashSet::new();
    for r in &ctx.rows {
        if !crate::simulation_live::is_a_share(&r.symbol)
            || r.symbol.starts_with("bj")
            || !symbols.insert(r.symbol.clone())
            || !r.factor.is_finite()
            || r.factor <= 0.
            || [r.close, r.adjusted_close, r.atr14, r.amount]
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || *v < 0.)
        {
            return Err("执行上下文含非法股票、复权或价格".into());
        }
    }
    let watches = run["signal_watch"].as_array().ok_or("原模型无观察信号")?;
    let scores: HashMap<_, _> = watches
        .iter()
        .map(|w| {
            (
                w["symbol"].as_str().unwrap_or(""),
                w["score"].as_f64().unwrap_or(f64::NEG_INFINITY),
            )
        })
        .collect();
    let ranked: HashSet<_> = ctx.ranked_symbols.iter().map(String::as_str).collect();
    if ranked.len() != ctx.ranked_symbols.len()
        || ranked.len() != scores.len()
        || ranked
            .iter()
            .any(|s| !symbols.contains(*s) || !scores.contains_key(*s))
        || ctx
            .ranked_symbols
            .windows(2)
            .any(|w| scores[w[0].as_str()] < scores[w[1].as_str()])
    {
        return Err("操作排序不属于原模型完整信号".into());
    }
    Ok(ctx)
}
fn export_context(
    config: super::research::ModelRunnerConfig,
    bundle: String,
    run: Value,
) -> Result<FollowContext, String> {
    super::research::trusted_runner(&config)?;
    let runner = std::path::Path::new(&config.research_root)
        .join("research/research-center-runner/follow_context.py");
    if Sha256::digest(std::fs::read(&runner).map_err(|_| "缺少本版本的冻结模型操作程序")?)
        != Sha256::digest(include_bytes!(
            "../../../research/research-center-runner/follow_context.py"
        ))
    {
        return Err("操作程序版本不同，拒绝执行".into());
    }
    let key = uuid::Uuid::new_v4();
    let dir = std::env::temp_dir();
    let input = dir.join(format!("bull-follow-{key}-input.json"));
    let output = dir.join(format!("bull-follow-{key}-context.json"));
    let log = dir.join(format!("bull-follow-{key}.log"));
    std::fs::write(&input, bundle).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut command = std::process::Command::new(&config.python);
        command
            .arg(&runner)
            .arg("--bundle")
            .arg(&input)
            .arg("--snapshot")
            .arg(&config.snapshot)
            .arg("--index")
            .arg(&config.index)
            .arg("--output")
            .arg(&output)
            .env("PYTHONIOENCODING", "utf-8")
            .current_dir(&config.research_root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&log).map_err(|e| e.to_string())?);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("模型操作程序无法启动：{e}"))?;
        let _job = super::research::own_research_process(&mut child)?;
        let began = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if began.elapsed().as_secs() > 240 {
                let _ = child.kill();
                let _ = child.wait();
                return Err("模型操作上下文计算超时，保留原账户".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        if !status.success() {
            return Err(format!(
                "原模型操作数据尚不可用：{}",
                std::fs::read_to_string(&log)
                    .unwrap_or_default()
                    .chars()
                    .rev()
                    .take(1200)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
        }
        validate_context(
            &std::fs::read_to_string(&output).map_err(|e| e.to_string())?,
            &run,
        )
    })();
    for path in [&input, &output, &log] {
        let _ = std::fs::remove_file(path);
    }
    result
}
async fn sync_context(db: &Database, binding: &mut FollowBinding) -> Result<(), String> {
    let run = source(db, binding.source_run_id)?;
    if context_is_current(binding, &run) {
        return Ok(());
    }
    let _permit = super::research::research_gate()
        .try_acquire()
        .map_err(|_| "研究数据更新中，等待原模型完成")?;
    let config = super::research::model_config(db)?;
    let raw = db.model_run_bundle(binding.source_run_id)?;
    let ctx = tokio::task::spawn_blocking(move || export_context(config, raw, run))
        .await
        .map_err(|e| e.to_string())??;
    apply_follow_context(db, binding, ctx)
}

fn apply_follow_context(db: &Database, binding: &mut FollowBinding, ctx: FollowContext) -> Result<(), String> {
    for o in db
        .get_sim_detail(binding.account_id)?
        .orders
        .iter()
        .filter(|o| matches!(o.status.as_str(), "pending" | "awaiting_confirmation"))
    {
        db.reject_follow_order(o.id, "原模型计划或时点程序更新，旧操作单已过期")?;
    }
    binding.candidate_cursor = 0;
    binding.execution_policy = recommended_execution(
        &binding.model_id,
        binding.max_positions,
        &binding.allocation_policy,
    );
    binding.as_of = ctx.as_of.clone();
    binding.source_sha256 = ctx.source_run_sha256.clone();
    binding.context_json = serde_json::to_string(&ctx).map_err(|e| e.to_string())?;
    binding.state = "waiting_session".into();
    binding.message = "原模型计划及独立时点条件已同步，等待下一交易日生成可执行操作单".into();
    db.save_follow_binding(binding)
}
fn context(binding: &FollowBinding) -> Result<FollowContext, String> {
    serde_json::from_str(&binding.context_json)
        .map_err(|_| "跟随上下文损坏，请更新原模型数据".into())
}
fn next_session(day: NaiveDate) -> Result<NaiveDate, String> {
    for n in 1..=16 {
        let d = day + chrono::Duration::days(n);
        if crate::datasource::trading_calendar::is_trading_day_at(Utc::now(), d)? {
            return Ok(d);
        }
    }
    Err("无法确认下个市场交易日".into())
}
fn held_sessions(ctx: &FollowContext, entry: &str, today: &str) -> usize {
    let completed = ctx
        .session_dates
        .iter()
        .filter(|d| d.as_str() >= entry && d.as_str() <= today)
        .count();
    let current = usize::from(
        today > ctx.as_of.as_str()
            && today >= entry
            && date(today)
                .ok()
                .and_then(|d| crate::datasource::trading_calendar::is_trading_day_at(Utc::now(), d).ok())
                == Some(true),
    );
    completed + current
}
fn pending(o: &SimOrder) -> bool {
    matches!(o.status.as_str(), "pending" | "awaiting_confirmation")
}
fn timing_evidence() -> &'static Value {
    static EVIDENCE: OnceLock<Value> = OnceLock::new();
    EVIDENCE.get_or_init(|| {
        serde_json::from_str(include_str!("../../model-follow-timing.json"))
            .expect("independently audited timing evidence")
    })
}
fn recommended_execution(model: &str, slots: i64, allocation: &str) -> String {
    let evidence = &timing_evidence()["models"][model];
    let config = &evidence["active_configuration"];
    if evidence["admitted"] == true
        && model == "index26_h20"
        && config["max_positions"].as_i64() == Some(slots)
        && config["allocation_policy"].as_str() == Some(allocation)
        && config["entry_policy"] == "index_support"
        && config["exit_policy"] == "original"
        && config["holding_sessions"] == 20
    {
        "index_support".into()
    } else {
        "baseline".into()
    }
}
pub(crate) fn ensure_execution_policy(binding: &FollowBinding) -> Result<(), String> {
    if timing_evidence()["models"].get(&binding.model_id).is_none() {
        return Err("本模型缺少时点研究证据".into());
    }
    let expected = recommended_execution(
        &binding.model_id,
        binding.max_positions,
        &binding.allocation_policy,
    );
    if binding.execution_policy != expected {
        return Err("本模型时点配置已更新或缺少对应数量/仓位的模拟证据，请等待新计划".into());
    }
    Ok(())
}
fn entry_note(policy: &str) -> &'static str {
    if policy == "index_support" {
        "完成日原指数模型信号合格，且沪深300近5日收益>0、60日均线偏离>=0、个股复权收盘>=10日均线，才在下一交易日尝试入场；不满足时继续等待。"
    } else {
        "原冻结模型完成日信号合格，按自己的原分数排序；下一交易日检查现金、名额、高开和盘口。"
    }
}
fn entry_condition(
    binding: &FollowBinding,
    ctx: &FollowContext,
    row: &ContextRow,
) -> Result<bool, String> {
    ensure_execution_policy(binding)?;
    if ctx.model_id != binding.model_id
        || ctx.source_run_sha256 != binding.source_sha256
        || ctx.as_of != binding.as_of
    {
        return Err("入场条件与原模型计划不一致，等待同步".into());
    }
    if binding.execution_policy == "baseline" {
        return Ok(true);
    }
    let market = ctx
        .market
        .as_ref()
        .ok_or("缺少本模型入场市场数据，等待原模型更新")?;
    if market.as_of != ctx.as_of {
        return Err("大盘入场数据与模型完成日不一致".into());
    }
    let r5 = market
        .csi300_return5
        .filter(|v| v.is_finite())
        .ok_or("沪深300近5日条件数据不足")?;
    let deviation = market
        .csi300_ma60_deviation
        .filter(|v| v.is_finite())
        .ok_or("沪深30060日均线条件数据不足")?;
    let close = row
        .adjusted_close
        .filter(|v| v.is_finite() && *v > 0.)
        .ok_or("个股复权收盘条件数据不足")?;
    let ma10 = row
        .ma10
        .filter(|v| v.is_finite() && *v > 0.)
        .ok_or("个股10日均线条件数据不足")?;
    Ok(r5 > 0. && deviation >= 0. && close >= ma10)
}
pub(crate) fn ensure_entry_condition(binding: &FollowBinding, symbol: &str) -> Result<(), String> {
    let ctx = context(binding)?;
    if ctx.context_runner_sha256
        != format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../research/research-center-runner/follow_context.py"
            ))
        )
    {
        return Err("模型时点程序已更新，等待重新生成操作单".into());
    }
    if !ctx.ranked_symbols.iter().any(|s| s == symbol) {
        return Err("股票不在原冻结模型的合格候选中".into());
    }
    let row = ctx
        .rows
        .iter()
        .find(|r| r.symbol == symbol)
        .ok_or("缺少股票入场证据")?;
    if !entry_condition(binding, &ctx, row)? {
        return Err("原模型信号合格，但本模型个性化入场条件尚未满足".into());
    }
    Ok(())
}
fn timing_research(binding: &FollowBinding) -> Option<Value> {
    let evidence = timing_evidence();
    let mut item = evidence["models"][&binding.model_id].as_object()?.clone();
    let config = &evidence["models"][&binding.model_id]["active_configuration"];
    let matches = config["max_positions"].as_i64() == Some(binding.max_positions)
        && config["allocation_policy"].as_str() == Some(binding.allocation_policy.as_str())
        && config["entry_policy"].as_str() == Some(binding.execution_policy.as_str());
    item.insert("current_configuration_matches".into(), json!(matches));
    if !matches {
        item.insert("active_policy".into(), json!("original"));
        item.insert(
            "decision".into(),
            json!("当前配置与该研究配置不一致；本轮证据不适用，实际按账户执行规则。"),
        );
    }
    for key in [
        "start",
        "end",
        "early_period",
        "later_period",
        "ledger_count",
        "annual_slice_count",
        "selection_note",
    ] {
        item.insert(key.into(), evidence[key].clone());
    }
    Some(Value::Object(item))
}
fn context_is_current(binding: &FollowBinding, run: &Value) -> bool {
    run["content_sha256"].as_str() == Some(binding.source_sha256.as_str())
        && binding.execution_policy
            == recommended_execution(
                &binding.model_id,
                binding.max_positions,
                &binding.allocation_policy,
            )
        && context(binding).is_ok_and(|ctx| {
            ctx.context_runner_sha256
                == format!(
                    "{:x}",
                    Sha256::digest(include_bytes!(
                        "../../../research/research-center-runner/follow_context.py"
                    ))
                )
        })
}
fn recommended_allocation(model: &str, slots: i64) -> String {
    if model == "breadth22_rank20"
        && slots == 5
        && allocation_evidence(model, slots)
            .is_some_and(|e| e["admitted"] == true && e["selected_policy_id"] == "equal50")
    {
        "rank5_equal50".into()
    } else {
        "baseline8".into()
    }
}
pub(crate) fn allocation_targets(binding: &FollowBinding) -> Result<(i64, i64), String> {
    if !(1..=10).contains(&binding.max_positions) {
        return Err("持股数量须为1至10".into());
    }
    match binding.allocation_policy.as_str() {
        "baseline8" => Ok((8, binding.max_positions * 8)),
        "rank5_equal50"
            if binding.model_id == "breadth22_rank20"
                && binding.max_positions == 5
                && recommended_allocation(&binding.model_id, 5) == "rank5_equal50" =>
        {
            Ok((10, 50))
        }
        _ => Err("仓位配置没有对应模型/数量的已核对模拟证据".into()),
    }
}
fn entry_budget(
    db: &Database,
    b: &FollowBinding,
    d: &SimDetail,
    ctx: &FollowContext,
    equity: i64,
) -> Result<i64, String> {
    let (per, total) = allocation_targets(b)?;
    let per_budget = equity.checked_mul(per).ok_or("预算超限")? / 100;
    if b.allocation_policy == "baseline8" {
        return Ok(per_budget);
    }
    let mut invested = 0i64;
    for p in &d.positions {
        let price = scaled(
            ctx.rows
                .iter()
                .find(|r| r.symbol == p.symbol)
                .and_then(|r| r.close)
                .ok_or("持仓完成日价格缺失")?,
        )?;
        invested = invested
            .checked_add(price.checked_mul(p.quantity).ok_or("持仓金额超限")?)
            .ok_or("持仓金额超限")?;
    }
    let remaining = (equity.checked_mul(total).ok_or("总预算超限")? / 100)
        .saturating_sub(invested)
        .saturating_sub(reserved_cash(db, d, None)?)
        .max(0);
    Ok(per_budget.min(remaining))
}
fn lots(symbol: &str, budget: i64, price: i64) -> i64 {
    if price <= 0 {
        return 0;
    }
    let n = budget.max(0) / price;
    if symbol.starts_with("sh688") || symbol.starts_with("sh689") {
        if n >= 200 {
            n
        } else {
            0
        }
    } else {
        n / 100 * 100
    }
}
// User simulation: commission/stamp tax only; transfer and price slippage are zero.
pub(crate) fn follow_fee(gross: i64, side: Side) -> Result<i64, String> {
    crate::simulation::fee(
        gross,
        side,
        crate::simulation::FeeConfig {
            commission_bps: 3,
            min_commission: 5 * SCALE,
            stamp_tax_bps: 5,
            transfer_fee_bps: 0,
            slippage_bps: 0,
        },
    )
}
fn sized_quantity(symbol: &str, budget: i64, price: i64) -> Result<i64, String> {
    let mut qty = lots(symbol, budget, price);
    let step = if symbol.starts_with("sh688") || symbol.starts_with("sh689") {
        1
    } else {
        100
    };
    while qty > 0 {
        let gross = qty.checked_mul(price).ok_or("金额超限")?;
        if gross + follow_fee(gross, Side::Buy)? <= budget {
            break;
        }
        qty -= step;
        if step == 1 && qty < 200 {
            qty = 0;
        }
    }
    Ok(qty)
}
fn reserved_cash(db: &Database, detail: &SimDetail, except: Option<i64>) -> Result<i64, String> {
    let mut total = 0i64;
    for o in detail
        .orders
        .iter()
        .filter(|o| pending(o) && o.side == "buy" && Some(o.id) != except)
    {
        let m = db.follow_order_meta(o.id)?;
        total = total
            .checked_add(o.quantity * m.limit_price + m.estimated_fee)
            .ok_or("预留资金超限")?;
    }
    Ok(total)
}
fn occupied(detail: &SimDetail) -> usize {
    let mut s: HashSet<_> = detail.positions.iter().map(|p| p.symbol.as_str()).collect();
    for o in detail
        .orders
        .iter()
        .filter(|o| pending(o) && o.side == "buy")
    {
        s.insert(&o.symbol);
    }
    s.len()
}
fn equity(detail: &SimDetail, ctx: &FollowContext) -> Option<i64> {
    let mut e = detail.account.current_cash.parse::<i64>().ok()?;
    for p in &detail.positions {
        let row = ctx.rows.iter().find(|r| r.symbol == p.symbol)?;
        let price = scaled(row.close?).ok()?;
        e = e.checked_add(price.checked_mul(p.quantity)?)?;
    }
    Some(e)
}
fn exit_reason(
    db: &Database,
    detail: &SimDetail,
    ctx: &FollowContext,
    symbol: &str,
    today: &str,
) -> Result<Option<String>, String> {
    let rules = model_execution(&ctx.model_id)?;
    let position = detail
        .positions
        .iter()
        .find(|p| p.symbol == symbol)
        .ok_or("持仓不存在")?;
    if held_sessions(ctx, &position.acquired_date, today) >= rules.holding_sessions as usize {
        return Ok(Some(format!(
            "达到本模型{}个交易日持有期限",
            rules.holding_sessions
        )));
    }
    let row = ctx
        .rows
        .iter()
        .find(|r| r.symbol == symbol)
        .ok_or("原模型数据未覆盖持仓")?;
    if row.is_st != Some(0) {
        return Ok(Some("原模型收盘数据标记ST或风险状态未知，尝试退出".into()));
    }
    let entry = detail
        .orders
        .iter()
        .find(|o| o.symbol == symbol && o.side == "buy" && o.status == "filled")
        .ok_or("持仓缺少成交证据")?;
    let meta = db.follow_order_meta(entry.id)?;
    let entry_price = entry
        .price
        .as_deref()
        .ok_or("入场价缺失")?
        .parse::<i64>()
        .map_err(|_| "入场价无效")? as f64
        / SCALE as f64;
    if row
        .adjusted_close
        .is_some_and(|v| v < entry_price * meta.factor - rules.entry_atr_multiple * meta.atr)
    {
        return Ok(Some(format!(
            "本模型已完成日收盘跌破入场价减{}倍入场ATR，下一交易日退出",
            rules.entry_atr_multiple
        )));
    }
    Ok(None)
}
fn changed_entry_factor(
    db: &Database,
    detail: &SimDetail,
    ctx: &FollowContext,
) -> Result<Option<String>, String> {
    for p in &detail.positions {
        let Some(entry) = detail
            .orders
            .iter()
            .find(|o| o.symbol == p.symbol && o.side == "buy" && o.status == "filled")
        else {
            return Ok(Some(p.symbol.clone()));
        };
        let meta = db.follow_order_meta(entry.id)?;
        if ctx
            .rows
            .iter()
            .find(|r| r.symbol == p.symbol)
            .is_none_or(|r| (r.factor / meta.factor - 1.).abs() > 1e-8)
        {
            return Ok(Some(p.symbol.clone()));
        }
    }
    Ok(None)
}
fn inconsistent_holding(
    detail: &SimDetail,
    ctx: &FollowContext,
    ticks: &[LiveTick],
    now: DateTime<Utc>,
) -> Option<String> {
    ticks
        .iter()
        .filter(|t| {
            crate::simulation_live::validate(t, now).is_ok()
                && detail.positions.iter().any(|p| p.symbol == t.quote.code)
        })
        .find(|t| {
            ctx.rows
                .iter()
                .find(|r| r.symbol == t.quote.code)
                .is_none_or(|r| {
                    r.close
                        .is_none_or(|p| (t.quote.prev_close - p).abs() > 0.011)
                })
        })
        .map(|t| t.quote.code.clone())
}
fn expire(db: &Database, account: i64, now: DateTime<Utc>) -> Result<(), String> {
    for o in db
        .get_sim_detail(account)?
        .orders
        .iter()
        .filter(|o| pending(o))
    {
        let m = db.follow_order_meta(o.id)?;
        if now.timestamp() >= m.valid_until {
            db.reject_follow_order(o.id, "操作单已过期；未确认或未成交不计为买卖")?;
        }
    }
    Ok(())
}
fn stage(
    db: &Database,
    binding: &FollowBinding,
    ctx: &FollowContext,
    tick: &LiveTick,
    side: Side,
    quantity: i64,
    limit: i64,
    reason: &str,
    until: i64,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let symbol = &tick.quote.code;
    let today = now.with_timezone(&cst()).date_naive().to_string();
    let mut key = format!(
        "follow:{}:{}:{}:{}",
        binding.account_id,
        today,
        symbol,
        side.as_str()
    );
    if side == Side::Sell {
        key.push_str(&format!(":{}", tick.quote.timestamp));
    }
    if db
        .get_sim_detail(binding.account_id)?
        .orders
        .iter()
        .any(|o| o.idempotency_key == key)
    {
        return Ok(());
    }
    let row = ctx
        .rows
        .iter()
        .find(|r| r.symbol == *symbol)
        .ok_or("计划缺持仓数据")?;
    db.stage_follow_order(
        &OrderInput {
            account_id: binding.account_id,
            idempotency_key: key,
            symbol: symbol.clone(),
            name: tick.quote.name.clone(),
            side: side.as_str().into(),
            quantity,
            signal_date: today,
            source: "rule".into(),
            rule: None,
            stop_bps: 0,
            take_bps: 0,
            limit_bps: crate::market_rules::ensure_simulatable(symbol, &tick.quote.name)?,
            max_hold_days: 0,
            ai_generated: false,
        },
        &FollowOrderMeta {
            order_id: 0,
            limit_price: limit,
            estimated_fee: follow_fee(limit.checked_mul(quantity).ok_or("金额超限")?, side)?,
            factor: row.factor,
            atr: row.atr14.unwrap_or(0.),
            quote_at: tick.quote.timestamp,
            valid_until: until,
            context_as_of: ctx.as_of.clone(),
            context_sha256: ctx.source_run_sha256.clone(),
            allocation_policy: binding.allocation_policy.clone(),
            execution_policy: binding.execution_policy.clone(),
            automatic_submission: binding.auto_execute,
        },
        reason,
        now.timestamp()
            .max(tick.quote.timestamp)
            .max(tick.depth.timestamp),
    )
    .map(|_| ())
}
fn should_reprice_sell(order: &SimOrder, meta: &FollowOrderMeta, tick: &LiveTick) -> bool {
    order.side == "sell"
        && order.status == "pending"
        && tick.quote.timestamp > meta.quote_at
        && tick.depth.timestamp > meta.quote_at
        && sell_limit(tick).is_ok_and(|limit| limit < meta.limit_price)
}
async fn drive(
    db: &Database,
    manager: &DataSourceManager,
    binding: &mut FollowBinding,
) -> Result<(), String> {
    db.enable_follow_automation(binding)?;
    let now = Utc::now();
    expire(db, binding.account_id, now)?;
    if !binding.enabled {
        binding.state = "paused".into();
        binding.message = "跟随已暂停，保留现金、持仓与记录".into();
        return db.save_follow_binding(binding);
    }
    if !automatic_enabled(db)? {
        for order in db.get_sim_detail(binding.account_id)?.orders.iter().filter(|order| pending(order)) {
            db.reject_follow_order(order.id, "自动模型交易总开关已暂停，未成交委托撤销")?;
        }
        binding.state = "automatic_paused".into();
        binding.message = "自动模型交易已统一暂停，持仓和各账户独立设置保留".into();
        return db.save_follow_binding(binding);
    }
    sync_context(db, binding).await?;
    let ctx = context(binding)?;
    let rules = model_execution(&ctx.model_id)?;
    if let Err(reason) = crate::datasource::a_share_calendar::continuous(now) {
        if binding.state != "company_action_review" {
            binding.state = "waiting_session".into();
            binding.message = reason;
        }
        return db.save_follow_binding(binding);
    }
    let today = now.with_timezone(&cst()).date_naive();
    if next_session(date(&ctx.as_of)?)? != today {
        binding.state = "waiting_model_data".into();
        binding.message = format!(
            "原模型只更新至 {}，不能把旧信号补作今天的买卖；等待上一交易日完整数据",
            ctx.as_of
        );
        return db.save_follow_binding(binding);
    }
    let detail = db.get_sim_detail(binding.account_id)?;
    let day = today.to_string();
    if binding.allocation_date.as_deref() != Some(&day) {
        binding.candidate_cursor = 0;
        binding.allocation_date = Some(day.clone());
        binding.allocation_equity = equity(&detail, &ctx);
        db.save_follow_binding(binding)?;
    }
    if let Some(symbol) = changed_entry_factor(db, &detail, &ctx)? {
        binding.valuation_blocked = true;
        binding.valuation_block_reason = Some("factor_or_evidence_changed".into());
        binding.state = "company_action_review".into();
        binding.message = format!(
            "{symbol} 持仓复权/公司行动或成交证据需要核对，现金及股份权益未核清，跟随暂不成交"
        );
        for o in detail.orders.iter().filter(|o| pending(o)) {
            db.reject_follow_order(o.id, "公司行动待核对，操作单已过期")?;
        }
        return db.save_follow_binding(binding);
    }
    let close = cst()
        .from_local_datetime(&today.and_hms_opt(14, 57, 0).unwrap())
        .single()
        .unwrap()
        .timestamp();
    let buy_until = cst()
        .from_local_datetime(&today.and_hms_opt(9, 35, 0).unwrap())
        .single()
        .unwrap()
        .timestamp();
    let mut symbols: Vec<String> = detail
        .positions
        .iter()
        .map(|p| p.symbol.clone())
        .chain(
            detail
                .orders
                .iter()
                .filter(|o| pending(o))
                .map(|o| o.symbol.clone()),
        )
        .collect();
    symbols.sort();
    symbols.dedup();
    let (mut ticks, mut messages) = held_ticks(manager, &symbols).await?;
    if let Some(symbol) = inconsistent_holding(&detail, &ctx, &ticks, Utc::now()) {
        binding.valuation_blocked = true;
        if binding.valuation_block_reason.is_none() {
            binding.valuation_block_reason = Some("quote_mismatch".into());
        }
        binding.state = "company_action_review".into();
        binding.message =
            format!("{symbol} 昨收与原模型持仓证据不一致，等待公司行动核对；资产估值暂不可用");
        for o in detail.orders.iter().filter(|o| pending(o)) {
            db.reject_follow_order(o.id, "公司行动待核对，操作单已过期")?;
        }
        return db.save_follow_binding(binding);
    }
    if binding.valuation_blocked {
        if let Err(reason) = db.recover_follow_valuation(binding, &ticks, Utc::now()) {
            binding.state = "company_action_review".into();
            binding.message = format!("持仓权益待核对，估值和交易继续暂停：{reason}");
            return db.save_follow_binding(binding);
        }
    }
    for tick in &ticks {
        if crate::simulation_live::validate(tick, Utc::now()).is_err() {
            continue;
        }
        let row = ctx
            .rows
            .iter()
            .find(|r| r.symbol == tick.quote.code)
            .ok_or("标的缺原模型日线")?;
        if row
            .close
            .is_none_or(|v| (tick.quote.prev_close - v).abs() > 0.011)
        {
            messages.push(format!(
                "{} 昨收与冻结计划不一致，需复核公司行动；本轮不成交",
                tick.quote.code
            ));
            continue;
        }
        let d = db.get_sim_detail(binding.account_id)?;
        for o in d
            .orders
            .iter()
            .filter(|o| o.symbol == tick.quote.code && o.status == "pending")
        {
            let m = db.follow_order_meta(o.id)?;
            if Utc::now().timestamp() >= m.valid_until {
                db.reject_follow_order(o.id, "操作单已过期")?;
                continue;
            }
            if should_reprice_sell(o, &m, tick) {
                db.reject_follow_order(o.id, "卖出盘口下移，程序撤单重估可见买一限价；未记作成交")?;
                continue;
            }
            if o.side == "buy" {
                let available = d
                    .account
                    .current_cash
                    .parse::<i64>()
                    .map_err(|_| "现金无效")?
                    - reserved_cash(db, &d, Some(o.id))?;
                if o.quantity * m.limit_price + m.estimated_fee > available {
                    db.reject_follow_order(o.id, "实际现金不足，等待下一次原模型计划")?;
                    continue;
                }
            }
            if let Err(e) = db.match_sim_order_live(o.id, tick) {
                messages.push(e);
            }
        }
    }
    // Sell requests use completed-day conditions, never an invented intraday stop/profit rule.
    let d = db.get_sim_detail(binding.account_id)?;
    for p in &d.positions {
        if d.orders.iter().any(|o| o.symbol == p.symbol && pending(o)) {
            continue;
        }
        let Some(reason) = exit_reason(db, &d, &ctx, &p.symbol, &day)? else {
            continue;
        };
        let Some(tick) = ticks.iter().find(|t| t.quote.code == p.symbol) else {
            continue;
        };
        if crate::simulation_live::validate(tick, Utc::now()).is_err() {
            continue;
        }
        let row = ctx
            .rows
            .iter()
            .find(|r| r.symbol == p.symbol)
            .ok_or("持仓数据缺失")?;
        if row
            .close
            .is_none_or(|v| (tick.quote.prev_close - v).abs() > 0.011)
        {
            continue;
        }
        let quantity = d
            .positions
            .iter()
            .filter(|q| q.symbol == p.symbol && q.acquired_date < day)
            .map(|q| q.quantity)
            .sum();
        if quantity <= 0 {
            messages.push(format!("{} 满足退出条件，但T+1可卖量为零", p.symbol));
            continue;
        }
        let limit = match sell_limit(tick) {
            Ok(v) => v,
            Err(e) => {
                messages.push(e);
                continue;
            }
        };
        stage(
            db,
            binding,
            &ctx,
            tick,
            Side::Sell,
            quantity,
            limit,
            &reason,
            close,
            Utc::now(),
        )?;
    }
    if Utc::now().timestamp() < buy_until {
        let began = std::time::Instant::now();
        for (rank, symbol) in ctx
            .ranked_symbols
            .iter()
            .enumerate()
            .cycle()
            .skip(binding.candidate_cursor % ctx.ranked_symbols.len().max(1))
            .take(ctx.ranked_symbols.len())
        {
            let d = db.get_sim_detail(binding.account_id)?;
            if occupied(&d) >= binding.max_positions as usize {
                break;
            }
            if Utc::now().timestamp() >= buy_until || began.elapsed().as_secs() >= 15 {
                messages.push(
                    "本轮候选检查到达时间上限，下轮继续检查剩余原模型候选并循环重试；不补算旧开盘成交".into(),
                );
                break;
            }
            binding.candidate_cursor = (rank + 1) % ctx.ranked_symbols.len().max(1);
            if d.positions.iter().any(|p| p.symbol == *symbol)
                || d.orders
                    .iter()
                    .any(|o| o.symbol == *symbol && o.side == "buy" && o.signal_date == day)
            {
                continue;
            }
            let row = ctx
                .rows
                .iter()
                .find(|r| r.symbol == *symbol)
                .ok_or("股票缺少执行数据")?;
            if !entry_condition(binding, &ctx, row)? {
                continue;
            }
            let Some(reference) = row.close.filter(|v| *v > 0.) else {
                continue;
            };
            let Some(estimate) = binding.allocation_equity else {
                break;
            };
            let preliminary_cash = d
                .account
                .current_cash
                .parse::<i64>()
                .map_err(|_| "现金无效")?
                - reserved_cash(db, &d, None)?;
            let preliminary_budget = (preliminary_cash - 10 * SCALE)
                .min(entry_budget(db, binding, &d, &ctx, estimate)?)
                .min(money_scaled(row.amount.unwrap_or(0.) * 0.01)?);
            if !row.valid
                || row.is_st != Some(0)
                || row.atr14.is_none_or(|v| v <= 0.)
                || sized_quantity(symbol, preliminary_budget, scaled(reference * 1.001)?)? == 0
            {
                continue;
            }
            let fetched = match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                super::simulation_live::snapshot(manager, symbol),
            )
            .await
            {
                Ok(Ok(t)) => t,
                Ok(Err(e)) => {
                    messages.push(format!("{symbol}：{e}"));
                    continue;
                }
                Err(_) => {
                    messages.push(format!("{symbol}：报价超时，未生成操作单"));
                    continue;
                }
            };
            ticks.retain(|t| t.quote.code != *symbol);
            ticks.push(fetched);
            let tick = ticks.last().unwrap();
            if crate::simulation_live::validate(tick, Utc::now()).is_err() {
                continue;
            }
            if !row.valid
                || row.is_st != Some(0)
                || row.atr14.is_none_or(|v| v <= 0.)
                || row
                    .close
                    .is_none_or(|v| (tick.quote.prev_close - v).abs() > 0.011)
                || crate::market_rules::is_st(&tick.quote.name)
            {
                continue;
            }
            if !tick.quote.open.is_finite()
                || tick.quote.open <= 0.
                || tick.quote.open > row.close.unwrap() * (1. + rules.max_gap_pct / 100.)
            {
                messages.push(format!("{symbol} 高开超过原模型4%限制，跳过"));
                continue;
            }
            let limit =
                ((scaled(tick.quote.open)? as i128 * 10010 + 9999) / 10000 + 99) / 100 * 100;
            let limit = i64::try_from(limit).map_err(|_| "买价超限")?;
            let cash = d
                .account
                .current_cash
                .parse::<i64>()
                .map_err(|_| "现金无效")?
                - reserved_cash(db, &d, None)?;
            let Some(eq) = binding.allocation_equity else {
                messages.push("持仓估值缺失，不增加仓位".into());
                break;
            };
            let Some(amount) = row.amount.filter(|v| *v > 0.) else {
                continue;
            };
            let liquidity = money_scaled(amount * 0.01)?;
            let budget = (cash - 10 * SCALE)
                .min(entry_budget(db, binding, &d, &ctx, eq)?)
                .min(liquidity);
            let quantity = sized_quantity(symbol, budget, limit)?;
            if quantity == 0 {
                continue;
            }
            // Cannot allocate a slot to a buy whose full visible counterparty volume is already unavailable.
            if crate::simulation_live::quote_fill(
                tick,
                Side::Buy,
                quantity,
                limit,
                tick.quote.timestamp - 1,
                0,
                cash,
                crate::simulation::FeeConfig {
                    commission_bps: 3,
                    min_commission: 5 * SCALE,
                    stamp_tax_bps: 5,
                    transfer_fee_bps: 0,
                    slippage_bps: 0,
                },
                crate::market_rules::ensure_simulatable(symbol, &tick.quote.name)?,
                Utc::now(),
            )
            .is_err()
            {
                continue;
            }
            let (per, total) = allocation_targets(binding)?;
            let reason=format!("沿用 {} 完成日原模型信号，排序第{}；{}配置，每票入场目标{}%、总入场预算{}%，最多{}只；不追高开超过4%，程序自动提交并等待之后的新盘口成交",ctx.as_of,rank+1,if binding.allocation_policy=="rank5_equal50"{"排序模型5只/50%预算"}else{"原8%入场"},per,total,binding.max_positions);
            let reason = format!("{reason}；{}", entry_note(&binding.execution_policy));
            stage(
                db,
                binding,
                &ctx,
                tick,
                Side::Buy,
                quantity,
                limit,
                &reason,
                buy_until,
                Utc::now(),
            )?;
        }
    }
    let d = db.get_sim_detail(binding.account_id)?;
    let mark_ticks: Vec<_> = ticks
        .iter()
        .filter(|t| {
            crate::simulation_live::validate(t, Utc::now()).is_ok()
                && ctx
                    .rows
                    .iter()
                    .find(|r| r.symbol == t.quote.code)
                    .and_then(|r| r.close)
                    .is_some_and(|p| (t.quote.prev_close - p).abs() <= 0.011)
        })
        .cloned()
        .collect();
    if !mark_ticks.is_empty() {
        let _ = db.record_live_equity(binding.account_id, &mark_ticks, None);
    }
    binding.state = "listening".into();
    binding.message = if messages.is_empty() {
        format!(
            "按原模型跟踪中：最多{}只，仓位按绑定配置及实际现金/流动性计算；{}条程序委托待撮合；{}",
            binding.max_positions,
            d.orders.iter().filter(|o| pending(o)).count(),
            entry_note(&binding.execution_policy)
        )
    } else {
        messages.into_iter().take(6).collect::<Vec<_>>().join("；")
    };
    db.save_follow_binding(binding)
}
fn timestamp(stamp: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(stamp, 0).map(|d| d.to_rfc3339())
}
fn execution_evidence() -> &'static Value {
    static EVIDENCE: OnceLock<Value> = OnceLock::new();
    EVIDENCE.get_or_init(|| {
        serde_json::from_str(include_str!("../../model-follow-allocation.json"))
            .expect("verified allocation evidence")
    })
}
fn allocation_evidence(model: &str, slots: i64) -> Option<Value> {
    let evidence = execution_evidence();
    let result = evidence["models"][model]["caps"][slots.to_string()].as_object()?;
    Some(
        json!({"start":evidence["start"],"end":evidence["end"],"early_period":evidence["early_period"],"later_period":evidence["later_period"],"decision":evidence["decision"],"selection_note":evidence["selection_note"],"ledger_count":evidence["ledger_count"],"annual_slice_count":evidence["annual_slice_count"],"selected":result["selected"],"selected_policy_id":result["selected_policy_id"],"slot_comparison":evidence["models"][model]["slot_comparison"],"admitted":result["admitted"],"reasons":result["reasons"],"exit_decision":result["exit_decision"],"policies":result["policies"],"recommendation":evidence["models"][model]["recommendation"]}),
    )
}
fn view(db: &Database, b: &FollowBinding) -> Result<Value, String> {
    let ctx = context(b)?;
    let rules = model_execution(&ctx.model_id)?;
    let d = db.get_sim_detail(b.account_id)?;
    let today = Utc::now().with_timezone(&cst()).date_naive().to_string();
    let cash = d
        .account
        .current_cash
        .parse::<i64>()
        .map_err(|_| "现金无效")? as f64
        / SCALE as f64;
    let blocked = b.valuation_blocked || changed_entry_factor(db, &d, &ctx)?.is_some();
    let reserved = reserved_cash(db, &d, None)? as f64 / SCALE as f64;
    let available = (cash - reserved).max(0.);
    let mut e = Some(cash);
    let mut positions = Vec::new();
    for p in &d.positions {
        let row = ctx.rows.iter().find(|r| r.symbol == p.symbol);
        let observed = db
            .follow_mark(b.account_id, &p.symbol)?
            .filter(|(_, stamp)| {
                DateTime::from_timestamp(*stamp, 0).is_some_and(|time| {
                    time.with_timezone(&cst()).date_naive().to_string() > ctx.as_of
                })
            });
        let mark_at = observed
            .and_then(|(_, stamp)| DateTime::from_timestamp(stamp, 0))
            .map(|time| time.to_rfc3339())
            .or_else(|| {
                row.and_then(|r| r.close)
                    .map(|_| format!("{}T15:00:00+08:00", ctx.as_of))
            });
        let mark = if blocked {
            None
        } else {
            observed
                .map(|(v, _)| v as f64 / SCALE as f64)
                .or_else(|| row.and_then(|r| r.close))
        };
        if let (Some(value), Some(price)) = (e, mark) {
            e = Some(value + price * p.quantity as f64);
        } else {
            e = None;
        }
        positions.push(json!({"symbol":p.symbol,"name":p.name,"quantity":p.quantity,"available_quantity":if p.acquired_date<today{p.quantity}else{0},"cost_cny":p.cost_price.parse::<i64>().ok().map(|v|v as f64/SCALE as f64),"mark_cny":mark,"mark_at":mark_at,"entry_date":p.acquired_date,"holding_sessions":held_sessions(&ctx,&p.acquired_date,&today),"exit_reason":exit_reason(db,&d,&ctx,&p.symbol,&today)?}));
    }
    let mut orders = Vec::new();
    let mut delay_sum = 0i64;
    let mut delay_count = 0i64;
    for o in &d.orders {
        let m = db.follow_order_meta(o.id)?;
        if let (Ok(created), Some(confirmed)) = (
            DateTime::parse_from_rfc3339(&o.created_at),
            o.confirmed_at.as_deref(),
        ) {
            if let Ok(c) = DateTime::parse_from_rfc3339(confirmed) {
                delay_sum += (c - created).num_seconds().max(0);
                delay_count += 1;
            }
        }
        let money = |v: &Option<String>| {
            v.as_deref()
                .and_then(|s| s.parse::<i64>().ok())
                .map(|x| x as f64 / SCALE as f64)
        };
        orders.push(json!({"id":o.id,"symbol":o.symbol,"name":o.name,"side":o.side,"quantity":o.quantity,"limit_price_cny":m.limit_price as f64/SCALE as f64,"estimated_fee_cny":m.estimated_fee as f64/SCALE as f64,"status":o.status,"reason":o.decision_reason,"signal_date":o.signal_date,"created_at":o.created_at,"confirmed_at":o.confirmed_at,"automatic_submission":m.automatic_submission,"filled_at":o.filled_at,"filled_price_cny":money(&o.price),"fee_cny":money(&o.fee),"quote_at":timestamp(m.quote_at),"valid_until":timestamp(m.valid_until),"reject_reason":o.reject_reason}));
    }
    if blocked {
        e = None;
    }
    let current_source = source(db, b.source_run_id);
    let source_current = current_source
        .as_ref()
        .is_ok_and(|v| context_is_current(b, v));
    let source_message = if !b.enabled || source_current {
        b.message.clone()
    } else {
        format!(
            "原模型证据或时点程序已更新，等待同步；保留已成交账本。{}",
            current_source.as_ref().err().cloned().unwrap_or_default()
        )
    };
    let watches = current_source
        .ok()
        .filter(|_| source_current)
        .unwrap_or_else(|| json!({}));
    let scores: HashMap<_, _> = watches["signal_watch"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| Some((v["symbol"].as_str()?.to_string(), v["score"].as_f64()?)))
        .collect();
    let (position_pct, total_entry_pct) = allocation_targets(b)?;
    let eq = if blocked { None } else { equity(&d, &ctx) };
    let free = d
        .account
        .current_cash
        .parse::<i64>()
        .map_err(|_| "现金无效")?
        - reserved_cash(db, &d, None)?;
    let candidates: Vec<_> = ctx.ranked_symbols.iter().take(if source_current {30} else {0})
        .enumerate().map(|(i,s)| {
            let row = ctx.rows.iter().find(|r| r.symbol == *s);
            let condition = row.ok_or("缺少本模型入场证据".to_string()).and_then(|r| entry_condition(b,&ctx,r));
            let qualifies = matches!(&condition, Ok(true));
            let qty = if qualifies { match (row.and_then(|r| r.close),eq) {
                (Some(p),Some(v)) => scaled(p*1.001).ok().and_then(|price| sized_quantity(s,
                    (free-10*SCALE).min(entry_budget(db,b,&d,&ctx,v).unwrap_or(0))
                    .min(money_scaled(row.and_then(|r| r.amount).unwrap_or(0.)*0.01).unwrap_or(0)),price).ok()).unwrap_or(0),
                _ => 0
            }} else {0};
            let reason = match condition {
                Ok(true) => format!("{}；按收盘估算，盘中还需通过现金、名额、高开、限价和盘口检查。",entry_note(&b.execution_policy)),
                Ok(false) => "原模型信号合格，本模型条件入场尚未满足；暂不生成买单。".into(),
                Err(reason) => reason,
            };
            json!({"symbol":s,"rank":i+1,"score":scores.get(s),"reference_quantity":qty,"entry_condition_met":qualifies,"reason":reason})
        }).collect();
    let execution = json!({"signal_basis":"completed_daily","signal_label":"完成日日线选股","entry_schedule":"next_session_open","execution_basis":"live_depth","schedule_note":"前一交易日完整日线确定候选，下一交易日09:30–09:35用实时盘口买入；盘中实时检查委托与卖出。","position_pct":position_pct,"total_entry_pct":total_entry_pct,"allocation_note":if b.allocation_policy=="rank5_equal50"{"按已核对的排序模型5只组合，单票入场目标10%、总预算50%；保留20日/3ATR退出，实际数量受现金、费用和流动性限制；仅作模拟，历史回撤与收益一并查看。"}else{"原8%入场目标；实际数量受净值、现金、费用及流动性限制，保持原20日/3ATR退出。"},"entry_policy":b.execution_policy,"entry_note":entry_note(&b.execution_policy),"exit_policy":"original","exit_note":"最多持有20个交易日；可因3ATR或ST/状态风险提前退出；到期遇停牌、跌停无可见买盘或零碎股待核对时持续提示，不伪造清仓。","holding_days":rules.holding_sessions,"max_gap_pct":rules.max_gap_pct,"buy_window":"下一交易日09:30–09:35，北京时间；程序自动提交后等待新盘口，错过不追单","sell_window":"交易日连续竞价时段，14:57前；午休不成交，T+1及无可见买盘时等待","intraday_note":"日线模型：完成日确定入场资格及3ATR退出，盘中自动检查现金、名额、高开、限价、T+1和盘口并撮合；没有新增未回测的盘中价格止损或午后入场。","fee_note":"佣金万3/最低5元、卖出印花税万5；过户费0、滑点0。原研究收益保留原成本口径"});
    Ok(
        json!({"account_id":b.account_id,"source_run_id":b.source_run_id,"model_id":b.model_id,"model_name":b.model_name,"as_of":b.as_of,"max_positions":b.max_positions,"enabled":b.enabled,"effective_enabled":b.enabled && automatic_enabled(db)?,"automatic_execution":b.auto_execute,"allocation_policy":b.allocation_policy,"initial_cash_cny":b.initial_cash_cny,"cash_cny":cash,"reserved_cash_cny":reserved,"available_cash_cny":available,"equity_cny":e,"net_return_pct":e.map(|v|(v/b.initial_cash_cny-1.)*100.),"valuation_note":"净值为按持仓报价估算，缺当日标记时用原模型完成日收盘；各持仓展示估值数据时间，公司行动待核对时不计净值。","state":if !b.enabled{"paused"}else if source_current{b.state.as_str()}else{"waiting_model_data"},"message":source_message,"source_sha256":b.source_sha256,"positions":positions,"orders":orders,"candidates":candidates,"allocation_research":allocation_evidence(&b.model_id,b.max_positions),"timing_research":timing_research(b),"execution":execution,"comparison_note":"原研究账户最多10只；跟随入口按模型独立选择默认数量，可调1至10只。仓位方案按已核对模型与数量绑定；实际资金使用和自动提交/盘口差异独立记录，原研究收益不代表本账户收益。","performance":{"filled_buys":d.orders.iter().filter(|o|o.status=="filled"&&o.side=="buy").count(),"filled_sells":d.orders.iter().filter(|o|o.status=="filled"&&o.side=="sell").count(),"cancelled":d.orders.iter().filter(|o|o.status=="rejected"&&o.reject_reason.as_deref().is_some_and(|r|r.contains("撤销"))).count(),"expired":d.orders.iter().filter(|o|o.status=="rejected"&&o.reject_reason.as_deref().is_some_and(|r|r.contains("过期"))).count(),"confirmation_delay_seconds":if delay_count>0{Some(delay_sum/delay_count)}else{None}}}),
    )
}

const AUTOMATIC_DELAY_SECONDS: i64 = 600;
const AUTOMATIC_RETRY_SECONDS: i64 = 300;
const AUTOMATIC_RESEARCH_BUSY: &str = "原模型研究正在运行，等待下一轮自动检查";

fn automatic_enabled(db: &Database) -> Result<bool, String> {
    Ok(db.get_setting(AUTOMATIC_ENABLED_KEY).map_err(|e| e.to_string())?.as_deref() != Some("0")
        && db.get_setting("ai_enabled").map_err(|e| e.to_string())?.as_deref() != Some("0"))
}

fn automatic_revision() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!("../../../research/research-center-runner/model_runner.py"));
    hash.update(include_bytes!("../../../research/research-center-runner/follow_context.py"));
    hex::encode(hash.finalize())
}

struct AutomaticInput {
    enabled: bool,
    effective_enabled: bool,
    state: &'static str,
    message: String,
    next_check_at: Option<String>,
    day: String,
    as_of: String,
    update_at: String,
}

fn automatic_input(db: &Database, now: DateTime<Utc>) -> Result<AutomaticInput, String> {
    let enabled = db.get_setting(AUTOMATIC_ENABLED_KEY).map_err(|e| e.to_string())?.as_deref() != Some("0");
    let mut input = AutomaticInput { enabled, effective_enabled: automatic_enabled(db)?, state: "paused",
        message: "自动模型交易已暂停，账户和持仓保留".into(), next_check_at: None,
        day: crate::stockdb_schedule::day(now), as_of: String::new(), update_at: String::new() };
    if !input.effective_enabled { return Ok(input); }
    let local = now.with_timezone(&cst());
    match crate::datasource::trading_calendar::is_trading_day_at(now, local.date_naive()) {
        Ok(true) => {},
        Ok(false) => { input.state = "closed"; input.message = "A股休市，自动更新和新模型检查等到交易日".into(); return Ok(input); },
        Err(error) => { input.state = "waiting_calendar"; input.message = error; return Ok(input); },
    }
    let cfg = crate::stockdb_schedule::config(db)?;
    let target = crate::stockdb_schedule::validate_time(&cfg.time)?;
    use chrono::Timelike;
    if local.hour() * 60 + local.minute() < target {
        input.state = "waiting_time"; input.message = format!("等待交易日{}自动更新数据", cfg.time); return Ok(input);
    }
    let update = db.stockdb_update_record()?;
    if !cfg.stockdb_enabled || update.running_owner.is_some_and(crate::stockdb_schedule::owner_alive)
        || update.last_success_day.as_deref() != Some(input.day.as_str()) || update.last_error.is_some() {
        input.state = "waiting_update"; input.message = "等待今天的数据更新成功；程序按原重试规则处理，近期行情兜底保留".into(); return Ok(input);
    }
    input.as_of = match super::research::model_completed_day(now) {
        Ok(day) => day.to_string(),
        Err(error) => { input.state = "waiting_calendar"; input.message = error; return Ok(input); },
    };
    if update.data_as_of.as_deref().is_none_or(|day| day < input.as_of.as_str()) {
        input.state = "waiting_data"; input.message = format!("等待核验{}已完成日线，不使用今天尚未完成的日线", input.as_of); return Ok(input);
    }
    let Some(updated) = update.last_success_at.as_ref().and_then(|value| DateTime::parse_from_rfc3339(value).ok()) else {
        input.state = "waiting_update"; input.message = "数据更新完成时间尚未核验，等待更新记录".into(); return Ok(input);
    };
    input.update_at = updated.to_rfc3339();
    let ready_at = updated.with_timezone(&Utc) + chrono::Duration::seconds(AUTOMATIC_DELAY_SECONDS);
    if now < ready_at {
        input.state = "waiting_delay"; input.message = "数据已更新，10分钟后自动检查已有冻结模型".into();
        input.next_check_at = Some(ready_at.to_rfc3339()); return Ok(input);
    }
    input.state = "due"; input.message = "即将检查冻结模型；合格候选出现时独立建账并自动交易".into();
    Ok(input)
}

fn automatic_record_state(record: &AutomaticRecord, input: &AutomaticInput, now: DateTime<Utc>) -> &'static str {
    if record.running_owner.is_some_and(crate::stockdb_schedule::owner_alive) { return "running"; }
    if record.day == input.day && record.as_of == input.as_of && record.runner_sha256 == automatic_revision() {
        if record.last_completed_day.as_deref() == Some(input.day.as_str()) { return "complete"; }
        if record.failures >= crate::stockdb_schedule::MAX_FAILURES { return "failed"; }
        if record.retry_at.as_ref().and_then(|value| DateTime::parse_from_rfc3339(value).ok()).is_some_and(|retry| now < retry) { return "retry_wait"; }
    }
    "due"
}

fn automatic_status_at(db: &Database, now: DateTime<Utc>) -> Result<Value, String> {
    let mut input = automatic_input(db, now)?;
    let record = db.follow_automatic_record()?;
    if input.state == "due" {
        input.state = automatic_record_state(&record, &input, now);
        input.message = match input.state {
            "running" => "正在后台检查模型，已有账户继续盘中处理".into(),
            "complete" => "今天的模型检查已完成；账户按原模型规则自动处理，无需每天操作".into(),
            "failed" => "今天模型检查已连续5次失败，保留成功模型与账户，下一交易日再检查".into(),
            "retry_wait" => "部分模型尚未完成，5分钟后只重试未完成部分".into(),
            _ => input.message,
        };
        if input.state == "retry_wait" { input.next_check_at = record.retry_at.clone(); }
    }
    Ok(json!({"enabled":input.enabled,"effective_enabled":input.effective_enabled,"state":input.state,
        "message":input.message,"next_check_at":input.next_check_at,"as_of":input.as_of,
        "last_completed_day":record.last_completed_day,"last_completed_at":record.last_completed_at,
        "last_error":record.last_error,"failures":record.failures,"models":record.models}))
}

#[tauri::command]
pub async fn research_auto_status(db: State<'_, Arc<Database>>, enabled: Option<bool>) -> Result<Value, String> {
    if let Some(enabled) = enabled {
        db.set_follow_automatic_enabled(enabled)?;
    }
    automatic_status_at(&db, Utc::now())
}

fn claim_automatic(db: &Database, input: &AutomaticInput, now: DateTime<Utc>) -> Result<bool, String> {
    if input.state != "due" { return Ok(false); }
    db.change_follow_automatic_record(|record| {
        if automatic_record_state(record, input, now) != "due" { return Ok(false); }
        if record.day != input.day || record.as_of != input.as_of || record.runner_sha256 != automatic_revision() {
            record.day = input.day.clone(); record.as_of = input.as_of.clone(); record.runner_sha256 = automatic_revision();
            record.models.clear(); record.failures = 0; record.retry_at = None; record.last_error = None;
            // A new source day/revision must complete independently of the previous watermark.
            record.last_completed_day = None;
        }
        record.update_at = input.update_at.clone(); record.started_at = Some(now.to_rfc3339());
        record.running_owner = Some(std::process::id());
        Ok(true)
    })
}

fn automatic_model_terminal(row: &Value) -> bool {
    matches!(row["state"].as_str(), Some("following" | "created" | "no_candidates" | "paused" | "excluded"))
}

fn model_binding(db: &Database, model: &str) -> Result<Option<FollowBinding>, String> {
    Ok(db.follow_bindings()?.into_iter().filter(|binding| binding.model_id == model).max_by_key(|binding| binding.account_id))
}

fn automatic_candidates(binding: &FollowBinding) -> Result<bool, String> {
    let ctx = context(binding)?;
    let cash = crate::db::simulation::capital_cny_scaled(binding.initial_cash_cny)?;
    let (per, _) = allocation_targets(binding)?;
    for symbol in &ctx.ranked_symbols {
        let Some(row) = ctx.rows.iter().find(|row| row.symbol == *symbol) else { continue; };
        if !row.valid || row.is_st != Some(0) || row.atr14.is_none_or(|atr| atr <= 0.) || !entry_condition(binding, &ctx, row)? { continue; }
        let Some(price) = row.close.filter(|price| *price > 0.) else { continue; };
        let budget = (cash - 10 * SCALE).min(cash / 100 * per).min(money_scaled(row.amount.unwrap_or(0.) * 0.01)?);
        if sized_quantity(symbol, budget, scaled(price * 1.001)?)? > 0 { return Ok(true); }
    }
    Ok(false)
}

async fn automatic_model(db: &Database, model: &str, expected: &AutomaticInput) -> Result<Value, String> {
    if db.automatic_model_excluded(model)? { return Ok(json!({"model_id":model,"state":"excluded","message":"用户暂停或删除的模型，后台不重新开启"})); }
    let existing = model_binding(db, model)?;
    if let Some(binding) = &existing {
        if !binding.enabled { return Ok(json!({"model_id":model,"account_id":binding.account_id,"state":"paused","message":"保留用户暂停设置"})); }
    }
    let source_id = {
        let _research = super::research::research_gate().try_acquire().map_err(|_| AUTOMATIC_RESEARCH_BUSY)?;
        let continuation = if let Some(binding) = &existing { Some(binding.source_run_id) } else {
            db.model_runs()?.as_array().ok_or("模型来源列表无效")?.iter().find_map(|run| {
                (run["model_id"] == model && run["comparison"] == "baseline" && run["mode"] == "forward" && run["holding_days"] == 20).then(|| run["id"].as_i64()).flatten()
            })
        };
        if let Some(id) = continuation.filter(|id| source(db, *id).is_ok_and(|run| run["as_of"] == expected.as_of)) { id }
        else { super::research::run_model(db, model.into(), 20, "baseline".into(), "forward".into(), continuation).await? }
    };
    let run = source(db, source_id)?;
    if run["as_of"] != expected.as_of { return Err("模型只接受本次已完成日线；行情日期已改变，等待下一轮核验".into()); }
    if existing.is_none() && run["signal_watch"].as_array().is_some_and(Vec::is_empty) {
        return Ok(json!({"model_id":model,"state":"no_candidates","as_of":expected.as_of,"source_run_id":source_id,"source_sha256":run["content_sha256"],"message":"当前没有合格冻结模型信号，不新建空账户"}));
    }
    let current = automatic_input(db, Utc::now())?;
    if current.state != "due" || current.day != expected.day || current.as_of != expected.as_of { return Err("自动检查已暂停或数据更新中，保留现有账户等待核验".into()); }
    if let Some(binding) = &existing {
        if context_is_current(binding, &run) {
            return Ok(json!({"model_id":model,"state":"following","account_id":binding.account_id,"as_of":expected.as_of,"source_run_id":source_id,"source_sha256":run["content_sha256"],"message":"复用独立账户与用户配置，继续原模型自动执行"}));
        }
    }
    // Research/context work deliberately runs outside the follow gate: existing ledgers keep trading.
    let cash = db.simulation_auto_preset()?.initial_cash_cny;
    let prepared = prepare_follow_binding(db, FollowStart { source_run_id: source_id, initial_cash_cny: cash,
        max_positions: existing.as_ref().map(|binding| binding.max_positions) }).await?;
    let _follow = gate().try_acquire().map_err(|_| AUTOMATIC_RESEARCH_BUSY)?;
    let current = automatic_input(db, Utc::now())?;
    if current.state != "due" || current.day != expected.day || current.as_of != expected.as_of { return Err("自动检查已暂停或数据更新中，未提交账户变更".into()); }
    if db.automatic_model_excluded(model)? { return Ok(json!({"model_id":model,"state":"excluded","message":"用户暂停或删除的模型，后台不重新开启"})); }
    if let Some(mut binding) = model_binding(db, model)? {
        if !binding.enabled { return Ok(json!({"model_id":model,"account_id":binding.account_id,"state":"paused","message":"保留用户暂停设置"})); }
        if binding.source_run_id != source_id { return Err("模型已有其他来源的独立账户，保留原绑定等待刷新".into()); }
        apply_follow_context(db, &mut binding, context(&prepared)?)?;
        db.enable_model_observation(source_id, true)?;
        return Ok(json!({"model_id":model,"state":"following","account_id":binding.account_id,"as_of":expected.as_of,"source_run_id":source_id,"source_sha256":prepared.source_sha256,"message":"已更新原账户的完成日计划，账户本金、持仓和数量设置保留"}));
    }
    if existing.is_some() { return Ok(json!({"model_id":model,"state":"excluded","message":"准备期间账户已删除，后台不补建"})); }
    if !automatic_candidates(&prepared)? {
        return Ok(json!({"model_id":model,"state":"no_candidates","as_of":expected.as_of,"source_run_id":source_id,"source_sha256":prepared.source_sha256,"message":"当前没有同时满足入场条件和资金整手数量的候选，继续等待"}));
    }
    let mut binding = prepared;
    // Re-read the prospective preset at commit time; no existing account receives this change.
    binding.initial_cash_cny = db.simulation_auto_preset()?.initial_cash_cny;
    if !automatic_candidates(&binding)? { return Ok(json!({"model_id":model,"state":"no_candidates","as_of":expected.as_of,"source_run_id":source_id,"source_sha256":binding.source_sha256,"message":"新资金预设不足以按模型买入一手，等待合格候选"})); }
    db.create_automatic_follow_account(&mut binding)?;
    log::info!(target: "automation::model", "模型 {model} 已建立专用账户 #{}，按原模型入场时段等待新盘口",binding.account_id);
    db.enable_model_observation(source_id, true)?;
    Ok(json!({"model_id":model,"state":"created","account_id":binding.account_id,"as_of":binding.as_of,
        "source_run_id":source_id,"source_sha256":binding.source_sha256,"message":"已按本模型独立推荐配置新建账户，等待原入场时段和新盘口"}))
}

async fn automatic_worker(db: &Database, input: &AutomaticInput, started: DateTime<Utc>) -> Result<bool, String> {
    let mut deferred = false;
    let mut errors = Vec::new();
    for model in super::research_jobs::DAILY_MODEL_IDS {
        if !automatic_enabled(db)? { return Ok(false); }
        let record = db.follow_automatic_record()?;
        if let Some(done) = record.models.iter().find(|row| row["model_id"] == model && automatic_model_terminal(row)) {
            let fingerprint_matches = done["source_run_id"].as_i64().is_none_or(|id| source(db, id).is_ok_and(|run| run["content_sha256"] == done["source_sha256"]));
            if fingerprint_matches { continue; }
        }
        let row = match automatic_model(db, model, input).await {
            Ok(row) => row,
            Err(error) if error == AUTOMATIC_RESEARCH_BUSY => { deferred = true; json!({"model_id":model,"state":"waiting_research","message":error}) },
            Err(error) => { errors.push(format!("{model}：{error}")); json!({"model_id":model,"state":"error","account_id":model_binding(db, model)?.map(|binding| binding.account_id),"message":error}) },
        };
        db.change_follow_automatic_record(|record| {
            if record.running_owner != Some(std::process::id()) || record.started_at.as_deref() != Some(started.to_rfc3339().as_str()) { return Err("自动检查任务所有者已改变，不提交旧任务结果".into()); }
            record.models.retain(|previous| previous["model_id"] != model);
            record.models.push(row.clone()); Ok(())
        })?;
        if row["state"]=="error" {
            log::warn!(target: "automation::model", "模型 {model}：{}",row["message"].as_str().unwrap_or("检查失败"));
        } else { log::info!(target: "automation::model", "模型 {model}：{}",row["message"].as_str().unwrap_or("检查完成")); }
        if deferred { break; }
    }
    if errors.is_empty() { Ok(!deferred) } else { Err(errors.join("；")) }
}


fn finish_automatic(db: &Database, input: &AutomaticInput, started: DateTime<Utc>, finished: DateTime<Utc>, result: Result<bool, String>) -> Result<(), String> {
    let stopped = automatic_enabled(db).is_ok_and(|enabled| !enabled);
    db.change_follow_automatic_record(|record| {
        if record.running_owner != Some(std::process::id()) || record.started_at.as_deref() != Some(started.to_rfc3339().as_str()) { return Err("自动检查任务所有者已改变".into()); }
        let complete = super::research_jobs::DAILY_MODEL_IDS.iter().all(|model| record.models.iter().any(|row| row["model_id"] == *model && automatic_model_terminal(row)));
        record.running_owner = None;
        match result {
            Ok(true) if !stopped && complete => { record.last_completed_day = Some(input.day.clone()); record.last_completed_at = Some(finished.to_rfc3339()); record.last_error = None; record.retry_at = None; record.failures = 0; },
            Ok(_) => { record.retry_at = (!stopped).then(|| (finished + chrono::Duration::seconds(AUTOMATIC_RETRY_SECONDS)).to_rfc3339()); },
            Err(error) => { record.last_error = Some(error); if !stopped { record.failures = record.failures.saturating_add(1); record.retry_at = Some((finished + chrono::Duration::seconds(AUTOMATIC_RETRY_SECONDS)).to_rfc3339()); } },
        }
        Ok(())
    })
}

/// A completed watermark prevents another full run. Only cheap source checks can invalidate it.
fn revalidate_automatic_completion(db: &Database, input: &AutomaticInput, now: DateTime<Utc>) -> Result<(), String> {
    let current = db.follow_automatic_record()?;
    if automatic_record_state(&current, input, now) != "complete" { return Ok(()); }
    let invalid: HashSet<String> = current.models.iter().filter_map(|row| {
        let id = row["source_run_id"].as_i64()?;
        (!source(db, id).is_ok_and(|run| run["as_of"] == input.as_of && run["content_sha256"] == row["source_sha256"]))
            .then(|| row["model_id"].as_str().unwrap_or("").to_string())
    }).collect();
    if invalid.is_empty() { return Ok(()); }
    db.change_follow_automatic_record(|record| {
        if record.running_owner.is_some() || record.day != current.day || record.as_of != current.as_of || record.last_completed_at != current.last_completed_at { return Ok(()); }
        record.last_completed_day = None;
        for row in &mut record.models {
            if invalid.contains(row["model_id"].as_str().unwrap_or("")) {
                row["state"] = json!("waiting_source"); row["message"] = json!("来源指纹已变化，保留原账户等待可信刷新，不新建第二账户");
            }
        }
        Ok(())
    })
}

/// Called from a separate 60-second clock; claiming/spawning never waits for model research.
pub async fn automatic_tick(db: Arc<Database>) {
    let now = Utc::now();
    let input = match automatic_input(&db, now) { Ok(input) if input.state == "due" => input, _ => return };
    if let Err(error) = revalidate_automatic_completion(&db, &input, now) { log::warn!(target: "automation::model", "自动模型检查失败：{error}"); return; }
    let Ok(permit) = SETUP_GATE.get_or_init(|| tokio::sync::Semaphore::new(1)).try_acquire() else { return; };
    match claim_automatic(&db, &input, now) {
        Ok(true) => {}, Ok(false) => return,
        Err(error) => { log::warn!(target: "automation::model", "自动模型检查失败：{error}"); return; },
    }
    log::info!(target: "automation::model", "开始自动检查原模型，行情截止日 {}",input.as_of);
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let result = automatic_worker(&db, &input, now).await;
        match &result {
            Ok(true)=>log::info!(target: "automation::model", "本轮模型检查完成；复用原账户，仅为合格候选建立新账户"),
            Ok(false)=>log::info!(target: "automation::model", "本轮模型检查暂停或等待其他研究任务，已保留进度"),
            Err(error)=>log::warn!(target: "automation::model", "本轮模型检查失败，保留账户并等待后台重试：{error}"),
        }
        let persisted = finish_automatic(&db, &input, now, Utc::now(), result);
        if let Err(error) = persisted { log::error!(target: "automation::model", "无法保存任务完成水位：{error}"); }
    });
}

#[tauri::command]
pub fn research_follow_accounts(db: State<'_, Arc<Database>>) -> Result<Vec<Value>, String> {
    db.follow_bindings()?.iter().map(|b| view(&db, b)).collect()
}
/// One initial setup click; all subsequent selection and trading stays in the existing schedulers.
/// The command continues in the backend if the research panel is closed.
#[tauri::command]
pub async fn research_follow_setup(
    db: State<'_, Arc<Database>>,
    input: FollowSetup,
) -> Result<Value, String> {
    let maximum = validate_setup(&input)?;
    let _setup = SETUP_GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "正在准备自动交易账户，请等待完成；已有账户仍按原规则运行")?;
    {
        let _follow = manual_permit(gate()).await?;
        if let Some(view) = reuse_model_account(&db, &input.model_id)? {
            return Ok(view);
        }
        if db.follow_bindings()?.len() >= 20 {
            return Err("最多保留20个跟随账户，请复用现有账户".into());
        }
    }
    // Do not hold the follow lock during history preparation: existing accounts keep trading.
    let source_id = {
        let _research = super::research::research_gate()
            .try_acquire()
            .map_err(|_| "另一项模型研究正在运行，请等待完成后再启动自动交易")?;
        let day = super::research::model_completed_day(Utc::now())?.to_string();
        let rows = db.model_runs()?;
        let ready = rows
            .as_array()
            .ok_or("模型来源列表无效")?
            .iter()
            .find_map(|run| {
                let id = run["id"].as_i64()?;
                (run["model_id"] == input.model_id && source(&db, id).is_ok()).then_some(id)
            });
        let id = if let Some(id) = ready {
            if source(&db, id)?["as_of"] == day {
                id
            } else {
                super::research::run_model(
                    &db,
                    input.model_id.clone(),
                    20,
                    "baseline".into(),
                    "forward".into(),
                    Some(id),
                )
                .await?
            }
        } else {
            super::research::run_model(
                &db,
                input.model_id.clone(),
                20,
                "baseline".into(),
                "forward".into(),
                None,
            )
            .await?
        };
        if source(&db, id)?["as_of"] != day {
            return Err("历史数据尚未到最新完成日，请在数据齐备后重试；没有补造旧信号成交".into());
        }
        db.enable_model_observation(id, true)?;
        id
    };
    let _follow = manual_permit(gate()).await?;
    // A user may have created this model account while the source was preparing.
    if let Some(view) = reuse_model_account(&db, &input.model_id)? {
        return Ok(view);
    }
    let mut view = start_locked(
        &db,
        FollowStart {
            source_run_id: source_id,
            initial_cash_cny: input.initial_cash_cny,
            max_positions: Some(maximum),
        },
    )
    .await?;
    view["setup_reused"] = json!(false);
    Ok(view)
}

fn reuse_model_account(db: &Database, model_id: &str) -> Result<Option<Value>, String> {
    let bindings = db.follow_bindings()?;
    let Some(binding) = bindings
        .iter()
        .filter(|b| b.model_id == model_id)
        .max_by_key(|b| b.account_id)
    else {
        return Ok(None);
    };
    let run = source(db, binding.source_run_id)?;
    if run["model_id"] != model_id {
        return Err("现有账户与原模型来源身份不同，请核对来源".into());
    }
    db.enable_model_observation(binding.source_run_id, true)?;
    let mut view = if binding.enabled {
        let mut current = binding.clone();
        db.enable_follow_automation(&mut current)?;
        view(db, &current)?
    } else {
        update_locked(db, binding.account_id, None, Some(true))?
    };
    view["setup_reused"] = json!(true);
    Ok(Some(view))
}

#[tauri::command]
pub async fn research_follow_start(
    db: State<'_, Arc<Database>>,
    input: FollowStart,
) -> Result<Value, String> {
    let _permit = manual_permit(gate()).await?;
    start_locked(&db, input).await
}

async fn start_locked(db: &Database, input: FollowStart) -> Result<Value, String> {
    let mut binding = prepare_follow_binding(db, input).await?;
    db.create_follow_account(&mut binding)?;
    db.enable_model_observation(binding.source_run_id, true)?;
    view(db, &binding)
}

async fn prepare_follow_binding(db: &Database, input: FollowStart) -> Result<FollowBinding, String> {
    let initial_run = source(db, input.source_run_id)?;
    let max_positions = input
        .max_positions
        .unwrap_or_else(|| recommended_slots(initial_run["model_id"].as_str().unwrap()));
    validate_follow_values(input.initial_cash_cny, max_positions)?;
    if db.follow_bindings()?.len() >= 20 {
        return Err("最多保留20个跟随账户，请复用现有账户".into());
    }
    let run = source(&db, input.source_run_id)?;
    let _research = super::research::research_gate()
        .try_acquire()
        .map_err(|_| AUTOMATIC_RESEARCH_BUSY)?;
    let config = super::research::model_config(&db)?;
    let bundle = db.model_run_bundle(input.source_run_id)?;
    let ctx = tokio::task::spawn_blocking(move || export_context(config, bundle, run))
        .await
        .map_err(|e| e.to_string())??;
    let run = source(&db, input.source_run_id)?;
    if run["content_sha256"] != ctx.source_run_sha256 {
        return Err("创建期间原模型更新，请重试".into());
    }
    let model_name = run["model_name"]
        .as_str()
        .ok_or("原模型名缺失")?
        .to_string();
    let b = FollowBinding {
        account_id: 0,
        source_run_id: input.source_run_id,
        model_id: ctx.model_id.clone(),
        model_name,
        max_positions: max_positions,
        allocation_policy: recommended_allocation(&ctx.model_id, max_positions),
        execution_policy: recommended_execution(
            &ctx.model_id,
            max_positions,
            &recommended_allocation(&ctx.model_id, max_positions),
        ),
        enabled: true,
        auto_execute: true,
        initial_cash_cny: input.initial_cash_cny,
        as_of: ctx.as_of.clone(),
        source_sha256: ctx.source_run_sha256.clone(),
        context_json: serde_json::to_string(&ctx).map_err(|e| e.to_string())?,
        allocation_date: None,
        allocation_equity: None,
        valuation_blocked: false,
        valuation_block_reason: None,
        candidate_cursor: 0,
        state: "waiting_session".into(),
        message: "从空仓开始，等待下一交易日原模型操作；未补买过去的信号".into(),
    };
    Ok(b)
}
fn set_max_positions(db: &Database, b: &mut FollowBinding, n: i64) -> Result<(), String> {
    if !(1..=10).contains(&n) || occupied(&db.get_sim_detail(b.account_id)?) > n as usize {
        return Err("最多持股数应为1至10，且不能小于当前持仓及待买名额".into());
    }
    let policy = recommended_allocation(&b.model_id, n);
    let execution = recommended_execution(&b.model_id, n, &policy);
    if policy != b.allocation_policy || execution != b.execution_policy {
        for o in db
            .get_sim_detail(b.account_id)?
            .orders
            .iter()
            .filter(|o| pending(o))
        {
            db.reject_follow_order(
                o.id,
                "持股数量/仓位/时点配置已改变，未成交操作单撤销，后续新计划按新配置计算",
            )?;
        }
    }
    b.max_positions = n;
    b.allocation_policy = policy;
    b.execution_policy = execution;
    Ok(())
}
#[tauri::command]
pub async fn research_follow_update(
    db: State<'_, Arc<Database>>,
    account_id: i64,
    max_positions: Option<i64>,
    enabled: Option<bool>,
) -> Result<Value, String> {
    let _permit = manual_permit(gate()).await?;
    update_locked(&db, account_id, max_positions, enabled)
}

fn update_locked(
    db: &Database,
    account_id: i64,
    max_positions: Option<i64>,
    enabled: Option<bool>,
) -> Result<Value, String> {
    let mut b = db.follow_binding(account_id)?;
    if let Some(n) = max_positions {
        set_max_positions(&db, &mut b, n)?;
    }
    if let Some(on) = enabled {
        b.enabled = on;
        if !on {
            b.state = "paused".into();
            b.message = "跟随已暂停；持仓仍保留，请留意已有持仓风险".into();
        } else {
            b.state = "waiting_session".into();
            b.message = "已恢复，等待原模型及新行情".into();
        }
    }
    db.save_follow_binding_for_user(&b)?;
    db.enable_follow_automation(&mut b)?;
    view(&db, &b)
}
#[tauri::command]
pub async fn research_follow_delete(
    db: State<'_, Arc<Database>>,
    account_id: i64,
) -> Result<(), String> {
    delete_account(&db, account_id).await
}

async fn delete_account(db: &Database, account_id: i64) -> Result<(), String> {
    let _permit = manual_permit(gate()).await?;
    db.delete_follow_account(account_id)
}

#[tauri::command]
pub async fn research_follow_refresh(
    db: State<'_, Arc<Database>>,
    manager: State<'_, Arc<DataSourceManager>>,
    account_id: i64,
) -> Result<Value, String> {
    refresh_account(&db, &manager, account_id).await
}

async fn refresh_account(
    db: &Database,
    manager: &DataSourceManager,
    account_id: i64,
) -> Result<Value, String> {
    let _permit = manual_permit(gate()).await?;
    let mut b = db.follow_binding(account_id)?;
    if let Err(e) = drive(db, manager, &mut b).await {
        b.state = "data_unavailable".into();
        b.message = e;
        db.save_follow_binding(&b)?;
    }
    view(&db, &b)
}
#[tauri::command]
pub async fn research_follow_order(
    db: State<'_, Arc<Database>>,
    order_id: i64,
    action: String,
) -> Result<Value, String> {
    let _permit = manual_permit(gate()).await?;
    if action != "cancel" {
        return Err("自动模型专用账户由程序自动提交，不接受手动确认或下单".into());
    }
    db.follow_order_account(order_id)?;
    Err("自动模型专用账户不接受手动撤单；请暂停自动买卖，由程序统一撤销未成交委托并保留持仓".into())
}
pub async fn tick_all(db: &Database, manager: &DataSourceManager, app: &tauri::AppHandle) {
    let Ok(bindings) = db.follow_bindings() else {
        return;
    };
    for listed in bindings {
        let Ok(_permit) = gate().try_acquire() else {
            break;
        };
        // Re-read after acquiring: a queued user may have changed or paused this account.
        let Ok(mut binding) = db.follow_binding(listed.account_id) else {
            continue;
        };
        if let Err(error) = drive(db, manager, &mut binding).await {
            binding.state = "data_unavailable".into();
            binding.message = error;
            let _ = db.save_follow_binding(&binding);
        }
        if binding.state != listed.state || binding.message != listed.message {
            if binding.state=="data_unavailable" {
                log::warn!(target: "automation::trading", "账户 #{}：{}",binding.account_id,binding.message);
            } else { log::info!(target: "automation::trading", "账户 #{}：{}",binding.account_id,binding.message); }
        }
        // Publish before processing another model so the first plan is immediately followable.
        if let Ok(notices) = db.follow_notices() {
            for notice in notices {
                crate::notifications::publish(app, notice);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn automatic_input_test(day: &str, as_of: &str) -> AutomaticInput {
        AutomaticInput { enabled: true, effective_enabled: true, state: "due", message: "test".into(),
            next_check_at: None, day: day.into(), as_of: as_of.into(), update_at: "verified-update".into() }
    }


    #[test]
    fn automatic_pipeline_waits_ten_minutes_after_verified_update_and_stops_on_failure() {
        let (db, root, _, _) = automatic_test_account();
        db.set_setting("local_history_enabled", "1").unwrap();
        let updated = Utc.with_ymd_and_hms(2026, 9, 21, 1, 0, 0).unwrap();
        db.change_stockdb_update_record(|record| {
            record.last_success_day = Some("2026-09-21".into());
            record.last_success_at = Some(updated.to_rfc3339()); record.data_as_of = Some("2026-09-18".into()); Ok(())
        }).unwrap();
        let waiting = automatic_input(&db, updated + chrono::Duration::seconds(599)).unwrap();
        assert_eq!(waiting.state, "waiting_delay"); assert!(!claim_automatic(&db, &waiting, updated).unwrap());
        let due = automatic_input(&db, updated + chrono::Duration::seconds(600)).unwrap();
        assert_eq!(due.state, "due"); assert_eq!(due.as_of, "2026-09-18");
        db.change_stockdb_update_record(|record| { record.last_error = Some("更新失败".into()); Ok(()) }).unwrap();
        let failed = automatic_input(&db, updated + chrono::Duration::seconds(601)).unwrap();
        assert_eq!(failed.state, "waiting_update"); assert!(!claim_automatic(&db, &failed, updated).unwrap());
        assert!(db.follow_automatic_record().unwrap().last_completed_day.is_none());
        drop(db); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_claim_restart_and_completion_are_durable_and_failure_does_not_complete() {
        let (db, root, binding, _) = automatic_test_account();
        let other = Database::open(root.clone()).unwrap();
        let began = Utc.with_ymd_and_hms(2026, 9, 21, 1, 10, 0).unwrap();
        let input = automatic_input_test("2026-09-21", "2026-09-18");
        assert!(claim_automatic(&db, &input, began).unwrap());
        assert!(!claim_automatic(&other, &input, began).unwrap());
        finish_automatic(&db, &input, began, began, Err("StockDB及近期行情均未取得合格数据".into())).unwrap();
        assert!(other.follow_automatic_record().unwrap().last_completed_day.is_none());
        assert!(!claim_automatic(&other, &input, began + chrono::Duration::seconds(299)).unwrap());
        let retried = began + chrono::Duration::seconds(300);
        assert!(claim_automatic(&other, &input, retried).unwrap());
        // Even a caller's premature success cannot mark a partial list complete.
        finish_automatic(&other, &input, retried, retried, Ok(true)).unwrap();
        assert!(db.follow_automatic_record().unwrap().last_completed_day.is_none());
        let third = retried + chrono::Duration::seconds(300);
        assert!(claim_automatic(&db, &input, third).unwrap());
        db.change_follow_automatic_record(|record| {
            record.models = super::super::research_jobs::DAILY_MODEL_IDS.iter().map(|model| json!({"model_id":model,"state":"excluded","message":"user choice"})).collect(); Ok(())
        }).unwrap();
        finish_automatic(&db, &input, third, third, Ok(true)).unwrap();
        assert_eq!(other.follow_automatic_record().unwrap().last_completed_day.as_deref(), Some("2026-09-21"));
        assert!(!claim_automatic(&other, &input, third + chrono::Duration::hours(1)).unwrap());
        assert_eq!(other.follow_bindings().unwrap().len(), 1);
        assert_eq!(other.follow_bindings().unwrap()[0].account_id, binding.account_id);
        drop(other); drop(db);
        let reopened = Database::open(root.clone()).unwrap();
        assert!(!claim_automatic(&reopened, &input, third + chrono::Duration::hours(2)).unwrap());
        assert_eq!(reopened.follow_bindings().unwrap()[0].account_id, binding.account_id);
        drop(reopened); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_failures_have_a_daily_bound_and_program_change_refreshes_same_account() {
        let (db, root, binding, _) = automatic_test_account();
        let input = automatic_input_test("2026-09-21", "2026-09-18");
        let mut now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 10, 0).unwrap();
        for attempt in 1..=5 {
            assert!(claim_automatic(&db, &input, now).unwrap());
            finish_automatic(&db, &input, now, now, Err("数据源延迟".into())).unwrap();
            assert_eq!(db.follow_automatic_record().unwrap().failures, attempt);
            now += chrono::Duration::seconds(AUTOMATIC_RETRY_SECONDS);
        }
        assert!(!claim_automatic(&db, &input, now).unwrap());
        assert_eq!(automatic_record_state(&db.follow_automatic_record().unwrap(), &input, now), "failed");
        // A trusted program revision invalidates task evidence, not the model's ledger identity.
        db.change_follow_automatic_record(|record| { record.runner_sha256 = "old-trusted-version".into(); Ok(()) }).unwrap();
        assert!(claim_automatic(&db, &input, now).unwrap());
        assert_eq!(db.follow_bindings().unwrap()[0].account_id, binding.account_id);
        assert_eq!(db.follow_bindings().unwrap().len(), 1);
        finish_automatic(&db, &input, now, now, Ok(false)).unwrap();
        drop(db); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_setup_race_pause_delete_and_stale_background_cannot_duplicate_accounts() {
        let (db, root, mut template, _) = automatic_test_account();
        db.delete_follow_account(template.account_id).unwrap();
        template.account_id = 0;
        let first = Database::open(root.clone()).unwrap();
        let second = Database::open(root.clone()).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let original = template.clone();
        let outcomes = std::thread::scope(|scope| {
            let barrier_a = barrier.clone(); let mut a = original.clone();
            let barrier_b = barrier.clone(); let mut b = original;
            let one = scope.spawn(move || { barrier_a.wait(); first.create_follow_account(&mut a).map(|account| account.id) });
            let two = scope.spawn(move || { barrier_b.wait(); second.create_follow_account(&mut b).map(|account| account.id) });
            (one.join().unwrap(), two.join().unwrap())
        });
        assert_eq!(usize::from(outcomes.0.is_ok()) + usize::from(outcomes.1.is_ok()), 1);
        let current = db.follow_bindings().unwrap()[0].clone();
        assert_eq!(db.follow_bindings().unwrap().len(), 1);
        let mut paused = current.clone(); paused.enabled = false;
        db.save_follow_binding_for_user(&paused).unwrap();
        let other = Database::open(root.clone()).unwrap();
        assert!(other.automatic_model_excluded(&current.model_id).unwrap());
        assert!(other.save_follow_binding(&current).is_err());
        let mut old = current.clone(); assert!(other.enable_follow_automation(&mut old).is_err());
        let mut fresh = current.clone(); fresh.account_id = 0;
        assert!(other.create_automatic_follow_account(&mut fresh).is_err());
        db.delete_follow_account(current.account_id).unwrap();
        assert!(other.create_automatic_follow_account(&mut fresh).is_err());
        assert!(other.save_follow_binding(&current).is_err());
        assert!(other.follow_bindings().unwrap().is_empty());
        drop(other); drop(db);
        let reopened = Database::open(root.clone()).unwrap();
        assert!(reopened.automatic_model_excluded(&current.model_id).unwrap());
        assert!(reopened.create_automatic_follow_account(&mut fresh).is_err());
        drop(reopened); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_pause_rejects_pending_once_and_global_pause_blocks_stale_stage() {
        let (db, root, binding, ctx) = automatic_test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        let tick = sample_tick(now);
        stage(&db, &binding, &ctx, &tick, Side::Buy, 100, 100000, "automatic", now.timestamp()+300, now).unwrap();
        // A repeated caller and source recheck still use the existing deterministic order key.
        stage(&db, &binding, &ctx, &tick, Side::Buy, 100, 100000, "repeated clock", now.timestamp()+300, now).unwrap();
        assert_eq!(db.get_sim_detail(binding.account_id).unwrap().orders.len(), 1);
        let other = Database::open(root.clone()).unwrap();
        let mut paused = binding.clone(); paused.enabled = false;
        other.save_follow_binding_for_user(&paused).unwrap();
        assert_eq!(db.get_sim_detail(binding.account_id).unwrap().orders[0].status, "rejected");
        assert!(db.save_follow_binding(&binding).is_err());
        // Use a new day key with the same valid symbol; idempotence cannot hide pause validation.
        let later = now + chrono::Duration::days(1);
        let new_tick = sample_tick(later);
        assert!(stage(&db, &binding, &ctx, &new_tick, Side::Buy, 100, 100000, "stale after pause", later.timestamp()+300, later).is_err());
        other.save_follow_binding_for_user(&binding).unwrap();
        stage(&db, &binding, &ctx, &new_tick, Side::Buy, 100, 100000, "resumed automatic", later.timestamp()+300, later).unwrap();
        assert!(db.get_sim_detail(binding.account_id).unwrap().orders.iter().any(|order| order.status == "pending"));
        db.set_follow_automatic_enabled(false).unwrap();
        assert!(!automatic_enabled(&other).unwrap());
        assert!(other.get_sim_detail(binding.account_id).unwrap().orders.iter().all(|order| order.status == "rejected"));
        let third = later + chrono::Duration::days(1); let third_tick = sample_tick(third);
        assert!(stage(&db, &binding, &ctx, &third_tick, Side::Buy, 100, 100000, "global pause", third.timestamp()+300, third).is_err());
        db.set_follow_automatic_enabled(false).unwrap(); // Repeat pause has no ledger side effects.
        db.set_follow_automatic_enabled(true).unwrap(); db.set_setting("ai_enabled", "0").unwrap();
        assert!(!automatic_enabled(&other).unwrap());
        assert!(stage(&db, &binding, &ctx, &third_tick, Side::Buy, 100, 100000, "AI total pause", third.timestamp()+300, third).is_err());
        assert_eq!(db.get_sim_detail(binding.account_id).unwrap().orders.len(), 2);
        drop(other); drop(db); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_capital_preset_validates_and_never_changes_existing_ledger() {
        let (db, root, binding, _) = automatic_test_account();
        let before = db.get_sim_detail(binding.account_id).unwrap();
        assert_eq!(db.simulation_auto_preset().unwrap().initial_cash_cny, 100000.);
        let preset = crate::db::simulation::AutomaticAccountPreset { initial_cash_cny: 123456.78 };
        assert_eq!(db.save_simulation_auto_preset(&preset).unwrap().initial_cash_cny, 123456.78);
        for cash in [f64::NAN, f64::INFINITY, -1., 999., 100000001., 1000.001] {
            assert!(db.save_simulation_auto_preset(&crate::db::simulation::AutomaticAccountPreset { initial_cash_cny: cash }).is_err());
        }
        let after = db.get_sim_detail(binding.account_id).unwrap();
        assert_eq!(after.account.initial_cash, before.account.initial_cash);
        assert_eq!(after.account.current_cash, before.account.current_cash);
        assert_eq!(db.follow_binding(binding.account_id).unwrap().initial_cash_cny, binding.initial_cash_cny);
        drop(db);
        let reopened = Database::open(root.clone()).unwrap();
        assert_eq!(reopened.simulation_auto_preset().unwrap().initial_cash_cny, 123456.78);
        drop(reopened); std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn automatic_source_refresh_failure_preserves_existing_account_and_old_hash() {
        let (db, root, mut binding, _) = automatic_test_account();
        binding.source_sha256 = "older-program-source-evidence".into();
        db.save_follow_binding(&binding).unwrap();
        // Invalid configured input fails before starting Python/network work.
        db.set_setting("model_runner_config", "invalid-json").unwrap();
        let input = automatic_input_test("2026-09-21", "2099-01-01");
        let result = automatic_model(&db, &binding.model_id, &input).await;
        assert!(result.is_err());
        assert_eq!(db.follow_bindings().unwrap().len(), 1);
        let after = db.follow_binding(binding.account_id).unwrap();
        assert_eq!(after.account_id, binding.account_id);
        assert_eq!(after.source_run_id, binding.source_run_id);
        assert_eq!(after.source_sha256, "older-program-source-evidence");
        let mut duplicate = binding.clone(); duplicate.account_id = 0;
        assert!(db.create_automatic_follow_account(&mut duplicate).is_err());
        drop(db); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recommended_setup_only_uses_registered_models_and_validates_before_side_effects() {
        for (model, maximum) in [
            ("breadth22_h20", 2),
            ("index26_h20", 3),
            ("breadth22_excess_csi20", 3),
            ("breadth22_rank20", 5),
            ("breadth22_open_downside20", 2),
        ] {
            let mut input = FollowSetup {
                model_id: model.into(),
                initial_cash_cny: 100000.,
                max_positions: None,
            };
            assert_eq!(validate_setup(&input).unwrap(), maximum);
            input.max_positions = Some(10);
            assert_eq!(validate_setup(&input).unwrap(), 10);
            input.max_positions = Some(11);
            assert!(validate_setup(&input).is_err());
        }
        assert!(validate_setup(&FollowSetup {
            model_id: "fundamental37_h20".into(),
            initial_cash_cny: 100000.,
            max_positions: None
        })
        .is_err());
        for invalid in [f64::NAN, 0., 999., 100000001., 1000.001] {
            assert!(validate_setup(&FollowSetup {
                model_id: "breadth22_rank20".into(),
                initial_cash_cny: invalid,
                max_positions: None
            })
            .is_err());
        }
    }

    #[test]
    fn pause_and_resume_cancel_pending_without_changing_filled_assets() {
        let (db, root, mut binding, mut ctx) = test_account();
        db.enable_follow_automation(&mut binding).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &binding,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "已成交仓位",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let filled_id = db.get_sim_detail(binding.account_id).unwrap().orders[0].id;
        let later = now + chrono::Duration::seconds(2);
        assert_eq!(
            db.match_sim_order_live_at(filled_id, &sample_tick(later), later)
                .unwrap()
                .status,
            "filled"
        );
        let next = later + chrono::Duration::seconds(2);
        // Same stock/day buys are deliberately idempotent; stage a different eligible symbol.
        let mut second_row = ctx.rows[0].clone();
        second_row.symbol = "sh600001".into();
        ctx.rows.push(second_row);
        ctx.ranked_symbols.push("sh600001".into());
        binding.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&binding).unwrap();
        let mut pending_tick = sample_tick(next);
        pending_tick.quote.code = "sh600001".into();
        pending_tick.depth.code = "sh600001".into();
        stage(
            &db,
            &binding,
            &ctx,
            &pending_tick,
            Side::Buy,
            100,
            100100,
            "待买单",
            next.timestamp() + 300,
            next,
        )
        .unwrap();
        let before = db.get_sim_detail(binding.account_id).unwrap();
        assert!(before.orders.iter().any(|o| o.status == "pending"));
        let paused = update_locked(&db, binding.account_id, None, Some(false)).unwrap();
        assert_eq!(paused["enabled"], false);
        assert_eq!(paused["state"], "paused");
        let after = db.get_sim_detail(binding.account_id).unwrap();
        assert_eq!(after.account.current_cash, before.account.current_cash);
        assert_eq!(after.positions[0].quantity, before.positions[0].quantity);
        assert!(!after.account.auto_enabled);
        assert!(after
            .orders
            .iter()
            .filter(|o| o.id != filled_id)
            .all(|o| o.status == "rejected"));
        let pending_id = after.orders.iter().find(|o| o.id != filled_id).unwrap().id;
        pending_tick.quote.timestamp += 2;
        pending_tick.depth.timestamp += 2;
        pending_tick.received_at += 2;
        let attempted = db.match_sim_order_live_at(
            pending_id,
            &pending_tick,
            next + chrono::Duration::seconds(2),
        );
        assert!(attempted.is_err() || attempted.unwrap().status != "filled");
        let resumed = update_locked(&db, binding.account_id, None, Some(true)).unwrap();
        assert_eq!(resumed["enabled"], true);
        let restored = db.get_sim_detail(binding.account_id).unwrap();
        assert!(restored.account.auto_enabled);
        assert_eq!(restored.account.current_cash, before.account.current_cash);
        assert_eq!(restored.positions[0].quantity, before.positions[0].quantity);
        assert_eq!(
            restored
                .orders
                .iter()
                .filter(|o| o.status == "filled")
                .count(),
            1
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_click_reuses_and_resumes_existing_model_account_without_resetting_configuration() {
        let (db, root, binding, _) = test_account();
        update_locked(&db, binding.account_id, None, Some(false)).unwrap();
        let before = db.get_sim_detail(binding.account_id).unwrap();
        assert!(reuse_model_account(&db, "index26_h20").unwrap().is_none());
        for _ in 0..3 {
            let reused = reuse_model_account(&db, "breadth22_rank20")
                .unwrap()
                .unwrap();
            assert_eq!(reused["account_id"], binding.account_id);
            assert_eq!(reused["setup_reused"], true);
            assert_eq!(reused["max_positions"], binding.max_positions);
            assert_eq!(reused["enabled"], true);
            assert_eq!(db.follow_bindings().unwrap().len(), 1);
        }
        let after = db.get_sim_detail(binding.account_id).unwrap();
        assert_eq!(after.account.current_cash, before.account.current_cash);
        assert_eq!(after.orders.len(), before.orders.len());
        assert!(after.positions.is_empty());
        assert!(db.model_run(binding.source_run_id).unwrap()["enabled"] == true);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn queued_manual_action_is_not_overtaken_by_background_polling() {
        let semaphore = tokio::sync::Semaphore::new(1);
        let background = semaphore.acquire().await.unwrap();
        let mut user = Box::pin(manual_permit(&semaphore));
        tokio::select! {
            biased;
            _ = &mut user => panic!("manual action bypassed active account lock"),
            _ = tokio::task::yield_now() => {}
        }
        drop(background);
        assert!(
            semaphore.try_acquire().is_err(),
            "background stole the queued user permit"
        );
        let user_permit = user.await.unwrap();
        assert!(semaphore.try_acquire().is_err());
        drop(user_permit);
        assert!(semaphore.try_acquire().is_ok());
    }
    fn index_test_account() -> (Database, std::path::PathBuf, FollowBinding, FollowContext) {
        // Initialize the correct frozen model rather than rebinding a different model account.
        let (db, root, mut b, mut ctx) = test_account_for_source(include_str!("../../../research/follow-timing-2026-10-04/checks/index-forward.json"));
        b.model_id = "index26_h20".into();
        b.model_name = "指数模型条件入场测试".into();
        b.execution_policy = recommended_execution(&b.model_id, 3, &b.allocation_policy);
        b.source_sha256 = source(&db, b.source_run_id).unwrap()["content_sha256"]
            .as_str()
            .unwrap()
            .into();
        b.as_of = "2026-09-30".into();
        ctx.model_id = b.model_id.clone();
        ctx.as_of = b.as_of.clone();
        ctx.source_run_sha256 = b.source_sha256.clone();
        ctx.market = Some(MarketContext {
            as_of: ctx.as_of.clone(),
            csi300_return5: Some(0.01),
            csi300_ma60_deviation: Some(0.),
        });
        ctx.session_dates = vec![ctx.as_of.clone()];
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        (db, root, b, ctx)
    }
    #[test]
    fn timing_policy_is_admitted_only_for_the_audited_model_quantity_and_budget() {
        for model in [
            "breadth22_h20",
            "index26_h20",
            "breadth22_excess_csi20",
            "breadth22_rank20",
            "breadth22_open_downside20",
        ] {
            for cap in 1..=10 {
                let allocation = recommended_allocation(model, cap);
                assert_eq!(
                    recommended_execution(model, cap, &allocation),
                    if model == "index26_h20" && cap == 3 {
                        "index_support"
                    } else {
                        "baseline"
                    }
                );
            }
        }
        assert_eq!(
            recommended_execution("index26_h20", 3, "rank5_equal50"),
            "baseline"
        );
        let evidence = timing_evidence();
        assert_eq!(evidence["ledger_count"], 63);
        assert_eq!(evidence["annual_slice_count"], 441);
        assert_eq!(
            evidence["models"]
                .as_object()
                .unwrap()
                .values()
                .filter(|v| v["admitted"] == true)
                .count(),
            1
        );
        assert!(evidence["models"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| v["policies"].as_array().unwrap().len() == 9));
    }
    #[test]
    fn index_entry_uses_completed_adjusted_facts_and_rejects_missing_or_mismatched_history() {
        let (db, root, b, mut ctx) = index_test_account();
        assert!(entry_condition(&b, &ctx, &ctx.rows[0]).unwrap()); // Equality to both MAs qualifies.
        ctx.rows[0].adjusted_close = Some(20.);
        ctx.rows[0].ma10 = Some(19.);
        assert!(entry_condition(&b, &ctx, &ctx.rows[0]).unwrap()); // Raw close is 10; use adjusted domain.
        ctx.rows[0].ma10 = Some(21.);
        assert!(!entry_condition(&b, &ctx, &ctx.rows[0]).unwrap());
        ctx.rows[0].ma10 = Some(19.);
        ctx.market.as_mut().unwrap().csi300_return5 = Some(0.);
        assert!(!entry_condition(&b, &ctx, &ctx.rows[0]).unwrap());
        ctx.market.as_mut().unwrap().csi300_return5 = Some(0.01);
        ctx.market.as_mut().unwrap().csi300_ma60_deviation = Some(-0.0001);
        assert!(!entry_condition(&b, &ctx, &ctx.rows[0]).unwrap());
        ctx.market.as_mut().unwrap().csi300_ma60_deviation = None;
        assert!(entry_condition(&b, &ctx, &ctx.rows[0]).is_err());
        ctx.market.as_mut().unwrap().csi300_ma60_deviation = Some(0.);
        ctx.market.as_mut().unwrap().as_of = "2026-09-29".into();
        assert!(entry_condition(&b, &ctx, &ctx.rows[0]).is_err());
        ctx.market.as_mut().unwrap().as_of = b.as_of.clone();
        ctx.rows[0].ma10 = None;
        assert!(entry_condition(&b, &ctx, &ctx.rows[0]).is_err());
        ctx.rows[0].ma10 = Some(19.);
        ctx.market = None;
        assert!(entry_condition(&b, &ctx, &ctx.rows[0]).is_err());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn entry_conditions_are_rechecked_at_stage_confirmation_and_actual_fill() {
        let (db, root, mut b, mut ctx) = index_test_account();
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 1, 30, 0).unwrap();
        ctx.market.as_mut().unwrap().csi300_return5 = Some(0.);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        assert!(stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "未满足条件",
            now.timestamp() + 300,
            now
        )
        .unwrap_err()
        .contains("尚未满足"));
        assert!(db.get_sim_detail(b.account_id).unwrap().orders.is_empty());
        let candidate = &view(&db, &b).unwrap()["candidates"][0];
        assert_eq!(candidate["reference_quantity"], 0);
        assert_eq!(candidate["entry_condition_met"], false);
        ctx.market.as_mut().unwrap().csi300_return5 = Some(0.01);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "合格条件",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        ctx.rows[0].ma10 = None;
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        assert!(db
            .confirm_follow_order(id, now.timestamp())
            .unwrap_err()
            .contains("10日均线"));
        assert_eq!(
            db.get_sim_detail(b.account_id).unwrap().orders[0].status,
            "awaiting_confirmation"
        );
        ctx.rows[0].ma10 = Some(10.);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        db.confirm_follow_order(id, now.timestamp()).unwrap();
        ctx.market.as_mut().unwrap().csi300_return5 = Some(-0.01);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        let later = now + chrono::Duration::seconds(2);
        assert!(db
            .match_sim_order_live_at(id, &sample_tick(later), later)
            .unwrap_err()
            .contains("尚未满足"));
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.account.current_cash, "1000000000");
        assert!(d.positions.is_empty());
        assert!(d.orders[0].price.is_none());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_timing_order_cannot_confirm_or_fill_even_when_model_source_is_unchanged() {
        let (db, root, b, ctx) = index_test_account();
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "时点版本测试",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        let mut meta = db.follow_order_meta(id).unwrap();
        meta.execution_policy = "baseline".into();
        let test_connection = rusqlite::Connection::open(root.join("bull-arrives.db")).unwrap();
        test_connection
            .execute(
                "UPDATE model_follow_orders SET meta_json=?1 WHERE order_id=?2",
                rusqlite::params![serde_json::to_string(&meta).unwrap(), id],
            )
            .unwrap();
        assert!(db
            .confirm_follow_order(id, now.timestamp())
            .unwrap_err()
            .contains("时点配置"));
        // Simulate an old pending record loaded after a restart, bypassing the normal confirm path.
        test_connection
            .execute(
                "UPDATE sim_orders SET status='pending',confirmed_at=?1 WHERE id=?2",
                rusqlite::params![now.to_rfc3339(), id],
            )
            .unwrap();
        let later = now + chrono::Duration::seconds(2);
        assert!(db
            .match_sim_order_live_at(id, &sample_tick(later), later)
            .unwrap_err()
            .contains("时点配置"));
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.account.current_cash, "1000000000");
        assert!(d.positions.is_empty());
        drop(test_connection);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn quantity_change_retires_index_timing_orders_and_only_audited_quantity_reenables_policy() {
        let (db, root, mut b, ctx) = index_test_account();
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "数量变更",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        set_max_positions(&db, &mut b, 2).unwrap();
        db.save_follow_binding(&b).unwrap();
        assert_eq!(b.execution_policy, "baseline");
        assert_eq!(
            db.get_sim_detail(b.account_id).unwrap().orders[0].status,
            "rejected"
        );
        assert!(db.confirm_follow_order(id, now.timestamp()).is_err());
        let research = timing_research(&b).unwrap();
        assert_eq!(research["current_configuration_matches"], false);
        assert_eq!(research["active_policy"], "original");
        set_max_positions(&db, &mut b, 3).unwrap();
        assert_eq!(b.execution_policy, "index_support");
        assert_eq!(
            timing_research(&b).unwrap()["current_configuration_matches"],
            true
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn old_records_require_context_program_and_timing_migration_before_trading() {
        let (db, root, b, _) = index_test_account();
        let run = source(&db, b.source_run_id).unwrap();
        assert!(context_is_current(&b, &run));
        let mut value = serde_json::to_value(&b).unwrap();
        value.as_object_mut().unwrap().remove("execution_policy");
        let mut legacy = crate::db::model_follow::decode_binding(&value.to_string()).unwrap();
        assert_eq!(legacy.execution_policy, "baseline");
        assert!(!context_is_current(&legacy, &run));
        assert!(ensure_execution_policy(&legacy).is_err());
        assert_eq!(
            timing_research(&legacy).unwrap()["current_configuration_matches"],
            false
        );
        legacy.execution_policy = b.execution_policy.clone();
        let mut ctx = context(&legacy).unwrap();
        ctx.context_runner_sha256 = "old exporter".into();
        legacy.context_json = serde_json::to_string(&ctx).unwrap();
        assert!(!context_is_current(&legacy, &run));
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    fn automatic_test_account() -> (Database, std::path::PathBuf, FollowBinding, FollowContext) {
        let (db, root, mut b, ctx) = test_account();
        db.enable_follow_automation(&mut b).unwrap();
        (db, root, b, ctx)
    }
    #[test]
    fn automatic_buy_and_sell_need_no_confirmation_and_only_fills_produce_trade_notices() {
        let (db, root, b, ctx) = automatic_test_account();
        db.set_setting("alerts_enabled", "0").unwrap(); // Notification preference never disables the simulation.
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100200,
            "程序自动买入",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let d = db.get_sim_detail(b.account_id).unwrap();
        let buy = d.orders[0].id;
        assert_eq!(d.account.mode, "auto");
        assert!(d.account.auto_enabled);
        assert_eq!(d.orders[0].status, "pending");
        assert_eq!(
            d.orders[0].confirmed_at.as_deref(),
            Some(now.to_rfc3339().as_str())
        );
        assert!(db.follow_order_meta(buy).unwrap().automatic_submission);
        assert!(db.follow_notices().unwrap().is_empty());
        assert!(d.positions.is_empty());
        assert_eq!(d.account.current_cash, "1000000000");
        assert_eq!(
            db.match_sim_order_live_at(buy, &sample_tick(now), now)
                .unwrap()
                .status,
            "pending"
        );
        let later = now + chrono::Duration::seconds(2);
        assert_eq!(
            db.match_sim_order_live_at(buy, &sample_tick(later), later)
                .unwrap()
                .status,
            "filled"
        );
        let notice = db.follow_notices().unwrap();
        assert_eq!(notice.len(), 1);
        assert_eq!(notice[0]["model_snapshot"]["automatic"], true);
        let detail = db.get_sim_detail(b.account_id).unwrap();
        let filled_buy = detail.orders.iter().find(|o| o.id == buy).unwrap();
        let actual_price =
            filled_buy.price.as_deref().unwrap().parse::<i64>().unwrap() as f64 / SCALE as f64;
        assert_eq!(notice[0]["model_snapshot"]["quantity"], 100);
        assert_eq!(notice[0]["model_snapshot"]["side"], "buy");
        assert_eq!(
            notice[0]["model_snapshot"]["filled_price_cny"],
            actual_price
        );
        assert_ne!(
            actual_price,
            db.follow_order_meta(buy).unwrap().limit_price as f64 / SCALE as f64
        );
        assert_eq!(
            notice[0]["model_snapshot"]["filled_at"].as_str(),
            filled_buy.filled_at.as_deref()
        );
        assert!(notice[0]["body"].as_str().unwrap().contains("成交时间"));
        assert!(!notice[0]["body"].as_str().unwrap().contains("有效至"));
        assert!(notice[0]["title"].as_str().unwrap().contains("模拟已成交"));
        assert!(db.follow_notices().unwrap().is_empty());
        let same_day = now + chrono::Duration::minutes(20);
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(same_day),
            Side::Sell,
            100,
            99900,
            "自动卖出T+1检查",
            same_day.timestamp() + 18000,
            same_day,
        )
        .unwrap();
        let same_sell = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        let after = same_day + chrono::Duration::seconds(2);
        assert_eq!(
            db.match_sim_order_live_at(same_sell, &sample_tick(after), after)
                .unwrap()
                .status,
            "pending"
        );
        assert_eq!(
            db.get_sim_detail(b.account_id).unwrap().positions[0].quantity,
            100
        );
        db.reject_follow_order(same_sell, "T+1等待下一日").unwrap();
        assert!(db.follow_notices().unwrap().is_empty());
        let next = Utc.with_ymd_and_hms(2026, 9, 22, 5, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(next),
            Side::Sell,
            100,
            99900,
            "午后自动卖出",
            next.timestamp() + 5000,
            next,
        )
        .unwrap();
        let sell = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        let fresh = next + chrono::Duration::seconds(2);
        assert_eq!(
            db.match_sim_order_live_at(sell, &sample_tick(fresh), fresh)
                .unwrap()
                .status,
            "filled"
        );
        let notices = db.follow_notices().unwrap();
        assert_eq!(notices.len(), 1);
        assert!(notices[0]["title"].as_str().unwrap().contains("卖出100股"));
        assert!(db
            .get_sim_detail(b.account_id)
            .unwrap()
            .positions
            .is_empty());
        let cash = db
            .get_sim_detail(b.account_id)
            .unwrap()
            .account
            .current_cash;
        drop(db);
        let db = Database::open(root.clone()).unwrap();
        assert!(db.follow_binding(b.account_id).unwrap().auto_execute);
        assert_eq!(
            db.get_sim_detail(b.account_id)
                .unwrap()
                .account
                .current_cash,
            cash
        );
        assert!(db.follow_notices().unwrap().is_empty());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn automation_upgrade_retires_old_unfilled_orders_and_preserves_filled_assets() {
        let (db, root, mut b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "旧已成交",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(id, now.timestamp()).unwrap();
        let later = now + chrono::Duration::seconds(2);
        db.match_sim_order_live_at(id, &sample_tick(later), later)
            .unwrap();
        let mut row = ctx.rows[0].clone();
        row.symbol = "sh600001".into();
        ctx.ranked_symbols.push(row.symbol.clone());
        ctx.rows.push(row);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        let mut tick = sample_tick(now);
        tick.quote.code = "sh600001".into();
        tick.depth.code = tick.quote.code.clone();
        stage(
            &db,
            &b,
            &ctx,
            &tick,
            Side::Buy,
            100,
            100100,
            "旧待确认",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(later),
            Side::Sell,
            100,
            99900,
            "旧待撮合",
            now.timestamp() + 18000,
            later,
        )
        .unwrap();
        let before = db.get_sim_detail(b.account_id).unwrap();
        db.enable_follow_automation(&mut b).unwrap();
        let after = db.get_sim_detail(b.account_id).unwrap();
        assert!(b.auto_execute);
        assert_eq!(after.account.mode, "auto");
        assert_eq!(after.account.current_cash, before.account.current_cash);
        assert_eq!(after.positions[0].quantity, 100);
        assert_eq!(
            after.orders.iter().filter(|o| o.status == "filled").count(),
            1
        );
        assert_eq!(
            after
                .orders
                .iter()
                .filter(|o| o.status == "rejected")
                .count(),
            2
        );
        db.enable_follow_automation(&mut b).unwrap();
        assert_eq!(db.get_sim_detail(b.account_id).unwrap().orders.len(), 3);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn paused_automation_blocks_pending_fills_and_unfilled_orders_expire_without_spending() {
        let (db, root, mut b, ctx) = automatic_test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "自动暂停检查",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        b.enabled = false;
        db.save_follow_binding_for_user(&b).unwrap();
        db.enable_follow_automation(&mut b).unwrap();
        let later = now + chrono::Duration::seconds(2);
        let cancelled=db.get_sim_detail(b.account_id).unwrap().orders[0].clone();
        assert_eq!(cancelled.status,"rejected","User pause synchronously cancels pending orders before the next quote");
        assert!(cancelled.reject_reason.as_deref().unwrap().contains("暂停"));
        assert!(db.match_sim_order_live_at(id,&sample_tick(later),later).unwrap_err().contains("不可成交"));
        assert!(
            !db.get_sim_detail(b.account_id)
                .unwrap()
                .account
                .auto_enabled
        );
        b.enabled = true;
        db.save_follow_binding_for_user(&b).unwrap();
        db.enable_follow_automation(&mut b).unwrap();
        expire(&db, b.account_id, now + chrono::Duration::seconds(301)).unwrap();
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.orders[0].status, "rejected");
        assert_eq!(d.account.current_cash, "1000000000");
        assert!(d.positions.is_empty());
        assert!(db.follow_notices().unwrap().is_empty());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn automatic_sell_reprices_only_on_a_new_lower_visible_bid_and_never_chases_buys() {
        let (db, root, b, ctx) = automatic_test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 5, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Sell,
            100,
            99900,
            "盘口重估",
            now.timestamp() + 18000,
            now,
        )
        .unwrap();
        let mut order = db.get_sim_detail(b.account_id).unwrap().orders[0].clone();
        let meta = db.follow_order_meta(order.id).unwrap();
        let mut tick = sample_tick(now);
        tick.depth.bids[0].price = 9.5;
        assert!(!should_reprice_sell(&order, &meta, &tick));
        tick = sample_tick(now + chrono::Duration::seconds(2));
        tick.depth.bids[0].price = 9.5;
        assert!(should_reprice_sell(&order, &meta, &tick));
        order.side = "buy".into();
        assert!(!should_reprice_sell(&order, &meta, &tick));
        order.side = "sell".into();
        tick.depth.bids[0].price = 10.;
        assert!(!should_reprice_sell(&order, &meta, &tick));
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn automatic_orders_wait_through_lunch_and_missing_depth_and_never_fill_after_expiry() {
        let (db, root, b, ctx) = automatic_test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "缺盘口检查",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        let later = now + chrono::Duration::seconds(2);
        let mut no_depth = sample_tick(later);
        no_depth.depth.asks.clear();
        assert_eq!(
            db.match_sim_order_live_at(id, &no_depth, later)
                .unwrap()
                .status,
            "pending"
        );
        assert!(db.follow_notices().unwrap().is_empty());
        let expired = now + chrono::Duration::seconds(301);
        assert!(db
            .match_sim_order_live_at(id, &sample_tick(expired), expired)
            .unwrap_err()
            .contains("过期"));
        expire(&db, b.account_id, expired).unwrap();
        let noon = Utc.with_ymd_and_hms(2026, 9, 21, 4, 0, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(noon),
            Side::Sell,
            100,
            99900,
            "午休检查",
            noon.timestamp() + 12000,
            noon,
        )
        .unwrap();
        let sell = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        let lunch = noon + chrono::Duration::seconds(2);
        assert_eq!(
            db.match_sim_order_live_at(sell, &sample_tick(lunch), lunch)
                .unwrap()
                .status,
            "pending"
        );
        assert_eq!(
            db.get_sim_detail(b.account_id)
                .unwrap()
                .account
                .current_cash,
            "1000000000"
        );
        assert!(db
            .get_sim_detail(b.account_id)
            .unwrap()
            .positions
            .is_empty());
        assert!(db.follow_notices().unwrap().is_empty());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cap_does_not_increase_single_name_allocation() {
        assert_eq!(recommended_slots("breadth22_h20"), 2);
        assert_eq!(recommended_slots("breadth22_rank20"), 5);
        let eq = 100000 * SCALE;
        for cap in [2, 3, 10] {
            let q = sized_quantity("sh600000", eq * 8 / 100, 10 * SCALE).unwrap();
            assert_eq!(q, 700);
            assert!(q * 10 * SCALE * cap <= eq * 8 * cap / 100);
        }
    }
    #[test]
    fn independent_defaults_and_policy_admission_match_each_model() {
        let (db, root, mut b, _) = test_account();
        for (model, cap, per, total) in [
            ("breadth22_h20", 2, 8, 16),
            ("index26_h20", 3, 8, 24),
            ("breadth22_excess_csi20", 3, 8, 24),
            ("breadth22_rank20", 5, 10, 50),
            ("breadth22_open_downside20", 2, 8, 16),
        ] {
            assert_eq!(recommended_slots(model), cap);
            b.model_id = model.into();
            b.max_positions = cap;
            b.allocation_policy = recommended_allocation(model, cap);
            assert_eq!(allocation_targets(&b).unwrap(), (per, total));
            let rules = model_execution(model).unwrap();
            assert_eq!(rules.holding_sessions, 20);
            assert_eq!(rules.entry_atr_multiple, 3.);
            assert_eq!(rules.max_gap_pct, 4.);
        }
        b.allocation_policy = "rank5_equal50".into();
        assert!(allocation_targets(&b).is_err()); // Cannot attach rank policy to another model.
        b.model_id = "breadth22_rank20".into();
        b.max_positions = 3;
        assert!(allocation_targets(&b).is_err()); // Cannot attach five-slot policy to three slots.
        b.max_positions = 10;
        b.allocation_policy = recommended_allocation(&b.model_id, 10);
        assert_eq!(allocation_targets(&b).unwrap(), (8, 80));
        b.max_positions = 11;
        assert!(allocation_targets(&b).is_err());
        assert!(model_execution("unknown_model").is_err());
        assert!(model_execution("intraday_signal_model").err().unwrap().contains("盘中信号模型须单独验证"));
        assert_eq!(super::super::research_jobs::DAILY_MODEL_IDS.len(), 5);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rank_budget_counts_held_value_and_unfilled_orders_together() {
        let (db, root, mut b, ctx) = test_account();
        b.max_positions = 5;
        b.allocation_policy = recommended_allocation(&b.model_id, 5);
        db.save_follow_binding(&b).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "资金预留测试",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let mut d = db.get_sim_detail(b.account_id).unwrap();
        let eq = 100000 * SCALE;
        assert_eq!(entry_budget(&db, &b, &d, &ctx, eq).unwrap(), 10000 * SCALE);
        // Independent portfolio snapshot: 45% held, plus the real DB pending buy.
        d.account.current_cash = (55000 * SCALE).to_string();
        d.positions.push(crate::db::simulation::Position {
            lot_id: 100,
            account_id: b.account_id,
            symbol: "sh600000".into(),
            name: "预算样例持仓".into(),
            quantity: 4500,
            available_quantity: 4500,
            cost_price: (10 * SCALE).to_string(),
            acquired_date: "20260918".into(),
            stop_bps: 0,
            take_bps: 0,
            max_hold_days: 20,
            limit_bps: 1000,
        });
        let reserved = reserved_cash(&db, &d, None).unwrap();
        let budget = entry_budget(&db, &b, &d, &ctx, eq).unwrap();
        assert_eq!(budget, 5000 * SCALE - reserved);
        assert!(budget + reserved + 45000 * SCALE <= eq / 2);
        d.positions[0].quantity = 4900;
        assert_eq!(entry_budget(&db, &b, &d, &ctx, eq).unwrap(), 0);
        db.reject_follow_order(d.orders[0].id, "撤销预留测试")
            .unwrap();
        d.orders = db.get_sim_detail(b.account_id).unwrap().orders;
        assert_eq!(entry_budget(&db, &b, &d, &ctx, eq).unwrap(), 1000 * SCALE);
        let mut missing = ctx;
        missing.rows[0].close = None;
        assert!(entry_budget(&db, &b, &d, &missing, eq).is_err());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_policy_blocks_old_confirmations_and_fills_without_spending() {
        let (db, root, mut b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "旧仓位已确认单",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let confirmed = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(confirmed, now.timestamp()).unwrap();
        let mut row = ctx.rows[0].clone();
        row.symbol = "sh600001".into();
        ctx.ranked_symbols.push(row.symbol.clone());
        ctx.rows.push(row);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        let mut tick = sample_tick(now);
        tick.quote.code = "sh600001".into();
        tick.depth.code = tick.quote.code.clone();
        stage(
            &db,
            &b,
            &ctx,
            &tick,
            Side::Buy,
            100,
            100100,
            "旧仓位未确认单",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let awaiting = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        b.max_positions = 5;
        b.allocation_policy = recommended_allocation(&b.model_id, 5);
        db.save_follow_binding(&b).unwrap();
        assert!(db
            .confirm_follow_order(awaiting, now.timestamp() + 1)
            .unwrap_err()
            .contains("仓位配置"));
        let later = now + chrono::Duration::seconds(2);
        assert!(db
            .match_sim_order_live_at(confirmed, &sample_tick(later), later)
            .unwrap_err()
            .contains("仓位配置"));
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert!(d.positions.is_empty());
        assert_eq!(d.account.current_cash, "1000000000");
        assert!(d
            .orders
            .iter()
            .all(|o| o.price.is_none() && o.filled_at.is_none()));
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn quantity_policy_change_retires_buys_and_sells_but_keeps_actual_holdings() {
        let (db, root, mut b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "形成持仓",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(id, now.timestamp()).unwrap();
        let fill_time = now + chrono::Duration::seconds(1);
        db.match_sim_order_live_at(id, &sample_tick(fill_time), fill_time)
            .unwrap();
        let day2 = now + chrono::Duration::days(1);
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(day2),
            Side::Sell,
            100,
            99900,
            "原退出待确认",
            day2.timestamp() + 300,
            day2,
        )
        .unwrap();
        let mut row = ctx.rows[0].clone();
        row.symbol = "sh600001".into();
        ctx.ranked_symbols.push(row.symbol.clone());
        ctx.rows.push(row);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        let mut tick = sample_tick(day2);
        tick.quote.code = "sh600001".into();
        tick.depth.code = tick.quote.code.clone();
        stage(
            &db,
            &b,
            &ctx,
            &tick,
            Side::Buy,
            100,
            100100,
            "另一股票待买",
            day2.timestamp() + 300,
            day2,
        )
        .unwrap();
        let before = db.get_sim_detail(b.account_id).unwrap();
        set_max_positions(&db, &mut b, 5).unwrap();
        db.save_follow_binding(&b).unwrap();
        let after = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(b.allocation_policy, "rank5_equal50");
        assert!(after.orders.iter().all(|o| !pending(o)));
        assert_eq!(after.account.current_cash, before.account.current_cash);
        assert_eq!(after.positions.len(), before.positions.len());
        assert_eq!(after.positions[0].quantity, before.positions[0].quantity);
        assert_eq!(reserved_cash(&db, &after, None).unwrap(), 0);
        assert!(set_max_positions(&db, &mut b, 0).is_err());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cash_fees_and_star_lots_are_reserved() {
        assert_eq!(
            sized_quantity("sh600000", 1000 * SCALE, 10 * SCALE).unwrap(),
            0
        );
        assert_eq!(
            sized_quantity("sh688001", 2100 * SCALE, 10 * SCALE).unwrap(),
            209
        );
        assert_eq!(
            sized_quantity("sh688001", 1999 * SCALE, 10 * SCALE).unwrap(),
            0
        );
        assert_eq!(follow_fee(10000 * SCALE, Side::Buy).unwrap(), 50000);
        assert_eq!(follow_fee(10000 * SCALE, Side::Sell).unwrap(), 100000);
    }
    #[test]
    fn follow_sessions_use_market_evidence_without_guessing() {
        // Calendar evidence is process-wide; keep this fixture out of parallel calendar tests.
        if std::env::var_os("BULL_FOLLOW_SESSION_TEST").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["commands::model_follow::tests::follow_sessions_use_market_evidence_without_guessing", "--exact", "--nocapture"])
                .env("BULL_FOLLOW_SESSION_TEST", "1")
                .output().unwrap();
            assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        assert_eq!(next_session(date("2026-09-30").unwrap()).unwrap(), date("2026-10-08").unwrap());
        assert!(next_session(date("2026-12-31").unwrap()).is_err());
        let (_db, binding) = crate::db::model_follow::delete_tests::fixture();
        let mut ctx = context(&binding).unwrap();
        ctx.as_of = "2027-03-01".into();
        ctx.session_dates = vec![ctx.as_of.clone()];
        assert!(next_session(date(&ctx.as_of).unwrap()).is_err());
        assert_eq!(held_sessions(&ctx, "2027-03-01", "2027-03-02"), 1);
        crate::datasource::trading_calendar::install(
            [date("2027-03-01").unwrap(), date("2027-03-02").unwrap()],
            "offline confirmed-session fixture", 2, Utc::now(),
        );
        assert_eq!(next_session(date(&ctx.as_of).unwrap()).unwrap(), date("2027-03-02").unwrap());
        assert_eq!(held_sessions(&ctx, "2027-03-01", "2027-03-02"), 2);
        assert!(next_session(date("2027-03-02").unwrap()).is_err());
    }
    fn sample_tick(now: DateTime<Utc>) -> LiveTick {
        LiveTick {
            quote: crate::domain::Quote {
                code: "sh600000".into(),
                market: "CN".into(),
                name: "测试A股".into(),
                price: 10.,
                change: 0.,
                change_pct: 0.,
                prev_close: 10.,
                open: 10.,
                high: 10.,
                low: 10.,
                volume: 100000,
                turnover: 1000000.,
                turnover_rate: None,
                timestamp: now.timestamp(),
            },
            depth: crate::domain::Depth {
                code: "sh600000".into(),
                bids: vec![crate::domain::Level {
                    price: 10.,
                    volume: 1000,
                }],
                asks: vec![crate::domain::Level {
                    price: 10.,
                    volume: 1000,
                }],
                timestamp: now.timestamp(),
            },
            source: "synthetic-follow-ledger-test".into(),
            received_at: now.timestamp(),
        }
    }
    #[tokio::test]
    async fn follow_delete_gate_serializes_both_refresh_orders_and_stale_ticks_cannot_revive() {
        use crate::db::model_follow::delete_tests::{assert_deleted, fixture, seed_ledger};
        for delete_first in [true, false] {
            let (db, mut binding) = fixture();
            let orders = seed_ledger(&db, binding.account_id);
            // A paused refresh runs the real drive/save/view path without requesting data.
            binding.enabled = false;
            db.save_follow_binding_for_user(&binding).unwrap();
            db.enable_follow_automation(&mut binding).unwrap();
            assert!(delete_account(&db, i64::MAX).await.is_err());
            let listed = db.follow_bindings().unwrap();
            let manager = DataSourceManager::new();
            let blocker = manual_permit(gate()).await.unwrap();
            let mut deleting = Box::pin(delete_account(&db, binding.account_id));
            let mut refreshing = Box::pin(refresh_account(&db, &manager, binding.account_id));
            if delete_first {
                tokio::select! {
                    biased;
                    _ = &mut deleting => panic!("delete bypassed active tick"),
                    _ = tokio::task::yield_now() => {}
                }
                tokio::select! {
                    biased;
                    _ = &mut refreshing => panic!("refresh bypassed active tick"),
                    _ = tokio::task::yield_now() => {}
                }
            } else {
                tokio::select! {
                    biased;
                    _ = &mut refreshing => panic!("refresh bypassed active tick"),
                    _ = tokio::task::yield_now() => {}
                }
                tokio::select! {
                    biased;
                    _ = &mut deleting => panic!("delete bypassed active tick"),
                    _ = tokio::task::yield_now() => {}
                }
            }
            assert!(gate().try_acquire().is_err());
            assert_eq!(
                db.get_sim_detail(binding.account_id).unwrap().orders.len(),
                3
            );
            drop(blocker);
            if delete_first {
                tokio::time::timeout(std::time::Duration::from_secs(5), &mut deleting)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    tokio::time::timeout(std::time::Duration::from_secs(5), &mut refreshing)
                        .await
                        .unwrap()
                        .unwrap_err()
                        .contains("跟随账户不存在")
                );
            } else {
                let refreshed =
                    tokio::time::timeout(std::time::Duration::from_secs(5), &mut refreshing)
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(refreshed["account_id"], binding.account_id);
                assert_eq!(refreshed["enabled"], false);
                tokio::time::timeout(std::time::Duration::from_secs(5), &mut deleting)
                    .await
                    .unwrap()
                    .unwrap();
            }
            assert_deleted(&db, binding.account_id, &orders);
            // tick_all may have listed this account before deletion. Its under-gate
            // re-read must fail, and neither a stale save nor an old fill can recreate it.
            let _permit = manual_permit(gate()).await.unwrap();
            assert!(db.follow_binding(listed[0].account_id).is_err());
            assert!(db.save_follow_binding(&listed[0]).is_err());
            let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 1).unwrap();
            assert!(db
                .match_sim_order_live_at(orders[1], &sample_tick(now), now)
                .is_err());
            let ctx = context(&binding).unwrap();
            assert!(stage(
                &db,
                &binding,
                &ctx,
                &sample_tick(now),
                Side::Buy,
                100,
                100000,
                "stale tick",
                now.timestamp() + 300,
                now
            )
            .is_err());
            assert!(db.follow_notices().unwrap().is_empty());
            assert_deleted(&db, binding.account_id, &orders);
        }
    }

    #[test]
    fn new_automatic_account_is_created_atomically_and_invalid_source_leaves_no_account() {
        let (db, root, mut binding, _) = test_account();
        db.delete_follow_account(binding.account_id).unwrap();
        let count = db.list_sim_accounts().unwrap().len();
        binding.account_id = 0; binding.auto_execute = true; binding.initial_cash_cny = 110000.01;
        let mut invalid = binding.clone(); invalid.source_sha256 = "client-spoof".into();
        assert!(db.create_follow_account(&mut invalid).is_err());
        assert_eq!(db.list_sim_accounts().unwrap().len(), count); assert_eq!(invalid.account_id, 0);
        let account = db.create_follow_account(&mut binding).unwrap();
        assert_eq!(account.managed_by, "model_follow"); assert_eq!(account.initial_cash, "1100000100");
        assert!(db.live_account(account.id).unwrap()); assert!(db.is_follow_account(account.id).unwrap());
        assert_eq!(db.follow_binding(account.id).unwrap().initial_cash_cny, 110000.01);
        assert!(!db.list_auto_sim_account_ids().unwrap().contains(&account.id));
        assert!(db.get_sim_detail(account.id).unwrap().positions.is_empty()); assert!(db.get_sim_detail(account.id).unwrap().orders.is_empty());
        assert!(db.create_follow_account(&mut binding).is_err());
        drop(db); std::fs::remove_dir_all(root).unwrap();
    }

    fn test_account() -> (Database, std::path::PathBuf, FollowBinding, FollowContext) {
        test_account_for_source(include_str!("../../../research/research-center-runner/checks/verified-rank-forward-b6cce0d8-167d-4c84-ad84-32f5b6800312.json"))
    }
    // Test legacy confirmation and new automatic execution against the dedicated model
    // ledger. A realtime engine marker is not a manual strategy or shared account.
    fn test_account_for_source(raw: &str) -> (Database, std::path::PathBuf, FollowBinding, FollowContext) {
        let root = std::env::temp_dir().join(format!("bull-follow-test-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let source_id=db.save_model_run(raw,None).unwrap();
        let a = db
            .save_sim_account(&AccountInput {
                id: None,
                name: "测试跟随".into(),
                initial_cash: "1000000000".into(),
                mode: "confirm".into(),
                auto_enabled: false,
                manual_source_enabled: false,
                rule_source_enabled: true,
                ai_source_enabled: false,
                commission_bps: 3,
                min_commission: "50000".into(),
                stamp_tax_bps: 5,
                transfer_fee_bps: 0,
                slippage_bps: 0,
                targets: vec![],
            })
            .unwrap();
        db.enable_live_account(a.id, &[]).unwrap();
        let ctx = FollowContext {
            schema: "frozen-model-execution-v1".into(),
            as_of: "2026-09-18".into(),
            model_id: source(&db, source_id).unwrap()["model_id"].as_str().unwrap().into(),
            model_sha256: "test".into(),
            source_run_sha256: source(&db, source_id).unwrap()["content_sha256"]
                .as_str()
                .unwrap()
                .into(),
            source_runner_sha256: "test".into(),
            context_runner_sha256: format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../../research/research-center-runner/follow_context.py"
                ))
            ),
            data_sha256: json!({}),
            session_dates: vec!["2026-09-18".into()],
            market: None,
            ranked_symbols: vec!["sh600000".into()],
            rows: vec![ContextRow {
                symbol: "sh600000".into(),
                close: Some(10.),
                adjusted_close: Some(10.),
                atr14: Some(0.2),
                ma10: Some(10.),
                factor: 1.,
                amount: Some(1000000.),
                valid: true,
                is_st: Some(0),
            }],
        };
        let b = FollowBinding {
            account_id: a.id,
            source_run_id: source_id,
            model_id: ctx.model_id.clone(),
            model_name: "测试模型".into(),
            max_positions: 3,
            allocation_policy: "baseline8".into(),
            execution_policy: "baseline".into(),
            enabled: true,
            auto_execute: false,
            initial_cash_cny: 100000.,
            as_of: ctx.as_of.clone(),
            source_sha256: ctx.source_run_sha256.clone(),
            context_json: serde_json::to_string(&ctx).unwrap(),
            state: "waiting_session".into(),
            message: "合成账户测试，不是市场表现".into(),
            allocation_date: None,
            allocation_equity: None,
            valuation_blocked: false,
            valuation_block_reason: None,
            candidate_cursor: 0,
        };
        db.save_follow_binding(&b).unwrap();
        (db, root, b, ctx)
    }
    #[test]
    fn confirmation_requires_new_quote_and_changes_only_actual_ledger() {
        let (db, root, b, ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        let until = now.timestamp() + 300;
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "原模型买单",
            until,
            now,
        )
        .unwrap();
        let before = db.get_sim_detail(b.account_id).unwrap();
        let id = before.orders[0].id;
        assert_eq!(before.orders[0].status, "awaiting_confirmation");
        assert_eq!(before.account.current_cash, "1000000000");
        assert!(before.positions.is_empty());
        assert_eq!(occupied(&before), 1);
        assert!(reserved_cash(&db, &before, None).unwrap() > 1000 * SCALE);
        assert!(db
            .match_sim_order_live_at(id, &sample_tick(now), now)
            .is_err());
        db.confirm_follow_order(id, now.timestamp()).unwrap();
        assert!(db.confirm_follow_order(id, now.timestamp()).is_err());
        assert_eq!(
            db.match_sim_order_live_at(id, &sample_tick(now), now)
                .unwrap()
                .status,
            "pending"
        );
        let later = now + chrono::Duration::seconds(1);
        let fill = db
            .match_sim_order_live_at(id, &sample_tick(later), later)
            .unwrap();
        assert_eq!(fill.status, "filled");
        assert_eq!(fill.fee.as_deref(), Some("50000"));
        let after = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(after.positions[0].quantity, 100);
        assert_eq!(
            after.account.current_cash.parse::<i64>().unwrap(),
            1000000000 - 10000000 - 50000
        );
        assert_eq!(db.follow_notices().unwrap().len(), 1);
        assert!(db.follow_notices().unwrap().is_empty());
        let cash = after.account.current_cash.clone();
        assert_eq!(
            db.match_sim_order_live_at(id, &sample_tick(later), later)
                .unwrap()
                .status,
            "filled"
        );
        assert_eq!(
            db.get_sim_detail(b.account_id)
                .unwrap()
                .account
                .current_cash,
            cash
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unanswered_orders_expire_without_spending_cash_or_creating_positions() {
        let (db, root, b, ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "原模型",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        expire(&db, b.account_id, now + chrono::Duration::seconds(300)).unwrap();
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.orders[0].status, "rejected");
        assert!(d.orders[0]
            .reject_reason
            .as_deref()
            .unwrap()
            .contains("过期"));
        assert!(d.positions.is_empty());
        assert_eq!(d.account.current_cash, "1000000000");
        assert_eq!(occupied(&d), 0);
        assert_eq!(reserved_cash(&db, &d, None).unwrap(), 0);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn exits_use_completed_day_original_rule_and_t_plus_one() {
        let (db, root, b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "原模型",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(id, now.timestamp()).unwrap();
        let later = now + chrono::Duration::seconds(1);
        db.match_sim_order_live_at(id, &sample_tick(later), later)
            .unwrap();
        ctx.session_dates.push("2026-09-21".into());
        ctx.as_of = "2026-09-21".into();
        ctx.rows[0].adjusted_close = Some(9.5);
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert!(exit_reason(&db, &d, &ctx, "sh600000", "2026-09-22")
            .unwrap()
            .is_none());
        ctx.rows[0].adjusted_close = Some(9.3);
        assert!(exit_reason(&db, &d, &ctx, "sh600000", "2026-09-22")
            .unwrap()
            .unwrap()
            .contains("3倍"));
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(later),
            Side::Sell,
            100,
            99900,
            "原模型退出",
            now.timestamp() + 18000,
            later,
        )
        .unwrap();
        let sell = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(sell, later.timestamp()).unwrap();
        let current = now + chrono::Duration::seconds(2);
        let result = db
            .match_sim_order_live_at(sell, &sample_tick(current), current)
            .unwrap();
        assert_eq!(result.status, "pending");
        assert!(result.reject_reason.unwrap().contains("T+1"));
        assert_eq!(
            db.get_sim_detail(b.account_id).unwrap().positions[0].quantity,
            100
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_frozen_context_export_round_trips_through_rust_binding() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let python = dirs::home_dir()
            .unwrap()
            .join(".cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe");
        if !python.is_file()
            || !repo
                .join("research/ashare-open-2026-10-01/stockdb-live/exports/matrices.npz")
                .is_file()
        {
            eprintln!("local real context smoke requires frozen research files and bundled Python");
            return;
        }
        let raw=include_str!("../../../research/research-center-runner/checks/verified-rank-forward-b6cce0d8-167d-4c84-ad84-32f5b6800312.json").to_string();
        let run = super::super::research::check_bound_run(&raw).unwrap();
        let cfg = super::super::research::ModelRunnerConfig {
            research_root: repo.to_string_lossy().into_owned(),
            python: python.to_string_lossy().into_owned(),
            snapshot: repo
                .join("research/ashare-open-2026-10-01/stockdb-live/exports")
                .to_string_lossy()
                .into_owned(),
            index: repo
                .join("research/ashare-open-2026-10-01/online/index-sh-000300.ndjson")
                .to_string_lossy()
                .into_owned(),
        };
        let ctx = export_context(cfg.clone(), raw, run.clone()).unwrap();
        assert_eq!(ctx.as_of, "2026-09-30");
        assert_eq!(ctx.rows.len(), 5427);
        assert_eq!(ctx.session_dates.len(), 2123);
        assert_eq!(ctx.source_run_sha256, run["content_sha256"]);
        assert!(ctx.ranked_symbols.is_empty());
        let body = serde_json::to_string(&ctx).unwrap();
        let envelope = json!({"schema":"model-follow-context-bundle-v1","content":body,"content_sha256":hex::encode(Sha256::digest(body.as_bytes()))});
        assert!(validate_context(&envelope.to_string(), &run).is_ok());
        let mut changed = run;
        changed["model_id"] = "breadth22_h20".into();
        assert!(validate_context(&envelope.to_string(), &changed).is_err());
        println!("Real frozen context → Rust: 5427 stocks/2123 sessions/source/model/program hashes verified; zero candidates kept empty");
        let index_raw =
            include_str!("../../../research/follow-timing-2026-10-04/checks/index-forward.json")
                .to_string();
        let index_run = super::super::research::check_bound_run(&index_raw).unwrap();
        let index_ctx = export_context(cfg, index_raw, index_run).unwrap();
        assert_eq!(index_ctx.model_id, "index26_h20");
        assert_eq!(index_ctx.rows.len(), 5427);
        let market = index_ctx.market.unwrap();
        assert_eq!(market.as_of, index_ctx.as_of);
        assert!(market.csi300_return5.is_some_and(f64::is_finite));
        assert!(market.csi300_ma60_deviation.is_some_and(f64::is_finite));
        assert!(index_ctx
            .rows
            .iter()
            .any(|r| r.ma10.is_some_and(|v| v.is_finite() && v > 0.)));
        println!("Real frozen index context: CSI300 conditions and adjusted MA10 share completed-day axis; no forced candidates");
    }
    #[test]
    fn pending_buys_reserve_slots_atomically_and_survive_restart() {
        let (db, root, mut b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        for i in 0..4 {
            let mut row = ctx.rows[0].clone();
            row.symbol = format!("sh60000{i}");
            if i > 0 {
                ctx.ranked_symbols.push(row.symbol.clone());
                ctx.rows.push(row);
                b.context_json = serde_json::to_string(&ctx).unwrap();
                db.save_follow_binding(&b).unwrap();
            }
            let mut tick = sample_tick(now);
            tick.quote.code = format!("sh60000{i}");
            tick.depth.code = tick.quote.code.clone();
            let staged = stage(
                &db,
                &b,
                &ctx,
                &tick,
                Side::Buy,
                100,
                100100,
                "原模型",
                now.timestamp() + 300,
                now,
            );
            assert_eq!(staged.is_ok(), i < 3);
        }
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.orders.len(), 3);
        assert_eq!(occupied(&d), 3);
        assert!(reserved_cash(&db, &d, None).unwrap() > 3000 * SCALE);
        assert!(d.positions.is_empty());
        for o in &d.orders {
            assert!(db.follow_order_meta(o.id).is_ok());
        }
        drop(db);
        let db = Database::open(root.clone()).unwrap();
        assert_eq!(db.get_sim_detail(b.account_id).unwrap().orders.len(), 3);
        assert_eq!(db.follow_binding(b.account_id).unwrap().max_positions, 3);
        assert!(
            db.follow_notices().unwrap().is_empty(),
            "unfilled orders do not notify as trades"
        );
        assert!(db.follow_notices().unwrap().is_empty());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cash_cannot_be_reserved_by_two_unfilled_orders() {
        let (db, root, mut b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            9000,
            100100,
            "资金预留测试",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let mut row = ctx.rows[0].clone();
        row.symbol = "sh600001".into();
        ctx.ranked_symbols.push(row.symbol.clone());
        ctx.rows.push(row);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        let mut tick = sample_tick(now);
        tick.quote.code = "sh600001".into();
        tick.depth.code = tick.quote.code.clone();
        assert!(stage(
            &db,
            &b,
            &ctx,
            &tick,
            Side::Buy,
            9000,
            100100,
            "另一买单",
            now.timestamp() + 300,
            now
        )
        .unwrap_err()
        .contains("现金"));
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.orders.len(), 1);
        assert!(d.positions.is_empty());
        assert_eq!(d.account.current_cash, "1000000000");
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cash_and_liquidity_use_money_scale_not_price_scale() {
        assert_eq!(money_scaled(2_000_000.).unwrap(), 20_000_000_000);
        assert_eq!(money_scaled(200_000_000. * 0.01).unwrap(), 20_000_000_000);
        assert!(scaled(2_000_000.).is_err());
        assert!(money_scaled(f64::NAN).is_err());
        assert!(money_scaled(-1.).is_err());
        assert!(money_scaled(1e15).is_err());
    }
    #[test]
    fn expired_confirmation_is_transactionally_rejected() {
        let (db, root, b, ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 34, 59).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "有效期测试",
            now.timestamp() + 1,
            now,
        )
        .unwrap();
        let id = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        assert!(db
            .confirm_follow_order(id, now.timestamp() + 2)
            .unwrap_err()
            .contains("过期"));
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.orders[0].status, "awaiting_confirmation");
        assert!(d.orders[0].confirmed_at.is_none());
        assert!(d.positions.is_empty());
        assert_eq!(d.account.current_cash, "1000000000");
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn updated_source_blocks_old_confirmations_and_fills_without_account_changes() {
        let (db, root, mut b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "原版本",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let confirmed = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(confirmed, now.timestamp()).unwrap();
        let mut row = ctx.rows[0].clone();
        row.symbol = "sh600001".into();
        ctx.ranked_symbols.push(row.symbol.clone());
        ctx.rows.push(row);
        b.context_json = serde_json::to_string(&ctx).unwrap();
        db.save_follow_binding(&b).unwrap();
        let mut tick = sample_tick(now);
        tick.quote.code = "sh600001".into();
        tick.depth.code = tick.quote.code.clone();
        stage(
            &db,
            &b,
            &ctx,
            &tick,
            Side::Buy,
            100,
            100100,
            "待确认版本",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let awaiting = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        let raw = db.model_run_bundle(b.source_run_id).unwrap();
        let mut envelope: Value = serde_json::from_str(&raw).unwrap();
        let mut body: Value = serde_json::from_str(envelope["content"].as_str().unwrap()).unwrap();
        body["test_source_revision"] = json!(2);
        let text = body.to_string();
        envelope["content_sha256"] = hex::encode(Sha256::digest(text.as_bytes())).into();
        envelope["content"] = text.into();
        db.save_model_run(&envelope.to_string(), Some(b.source_run_id))
            .unwrap();
        assert!(db
            .confirm_follow_order(awaiting, now.timestamp() + 1)
            .unwrap_err()
            .contains("更新"));
        let next = now + chrono::Duration::seconds(2);
        assert!(db
            .match_sim_order_live_at(confirmed, &sample_tick(next), next)
            .unwrap_err()
            .contains("更新"));
        let shown = view(&db, &b).unwrap();
        assert_eq!(shown["state"], "waiting_model_data");
        assert!(shown["candidates"].as_array().unwrap().is_empty());
        assert_eq!(shown["orders"].as_array().unwrap().len(), 2);
        let d = db.get_sim_detail(b.account_id).unwrap();
        assert_eq!(d.orders.len(), 2);
        assert!(d.positions.is_empty());
        assert_eq!(d.account.current_cash, "1000000000");
        assert_eq!(d.orders.iter().filter(|o| o.status == "pending").count(), 1);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sell_uses_visible_bid_and_can_be_repriced_after_cancel() {
        let (db, root, b, ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "买入",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let buy = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(buy, now.timestamp()).unwrap();
        let next = now + chrono::Duration::seconds(1);
        assert_eq!(
            db.match_sim_order_live_at(buy, &sample_tick(next), next)
                .unwrap()
                .status,
            "filled"
        );
        let later = Utc.with_ymd_and_hms(2026, 9, 22, 1, 30, 0).unwrap();
        let mut quote = sample_tick(later);
        quote.depth.bids[0].price = 9.99;
        let limit = sell_limit(&quote).unwrap();
        assert_eq!(limit, 99800);
        stage(
            &db,
            &b,
            &ctx,
            &quote,
            Side::Sell,
            100,
            limit,
            "退出",
            later.timestamp() + 18000,
            later,
        )
        .unwrap();
        let first = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.reject_follow_order(first, "用户撤销操作").unwrap();
        let newer = later + chrono::Duration::seconds(1);
        let mut quote = sample_tick(newer);
        quote.depth.bids[0].price = 9.98;
        let limit = sell_limit(&quote).unwrap();
        assert_eq!(limit, 99700);
        stage(
            &db,
            &b,
            &ctx,
            &quote,
            Side::Sell,
            100,
            limit,
            "重新核对盘口退出",
            later.timestamp() + 18000,
            newer,
        )
        .unwrap();
        let sell = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        assert_ne!(first, sell);
        db.confirm_follow_order(sell, newer.timestamp()).unwrap();
        let fresh = newer + chrono::Duration::seconds(1);
        quote.quote.timestamp = fresh.timestamp();
        quote.depth.timestamp = fresh.timestamp();
        quote.received_at = fresh.timestamp();
        let result = db.match_sim_order_live_at(sell, &quote, fresh).unwrap();
        assert_eq!(result.status, "filled");
        assert_eq!(result.price.as_deref(), Some("99800"));
        assert!(db
            .get_sim_detail(b.account_id)
            .unwrap()
            .positions
            .is_empty());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn inconsistent_held_quote_blocks_valuation_and_matching() {
        let (db, root, mut b, ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "买入",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let buy = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(buy, now.timestamp()).unwrap();
        let next = now + chrono::Duration::seconds(1);
        db.match_sim_order_live_at(buy, &sample_tick(next), next)
            .unwrap();
        let later = Utc.with_ymd_and_hms(2026, 9, 22, 1, 30, 0).unwrap();
        let mut bad = sample_tick(later);
        bad.quote.prev_close = 9.5;
        assert_eq!(
            inconsistent_holding(
                &db.get_sim_detail(b.account_id).unwrap(),
                &ctx,
                &[bad.clone()],
                later
            )
            .as_deref(),
            Some("sh600000")
        );
        b.state = "company_action_review".into();
        b.valuation_blocked = true;
        db.save_follow_binding(&b).unwrap();
        let result = view(&db, &b).unwrap();
        assert!(result["equity_cny"].is_null());
        assert!(result["positions"][0]["mark_cny"].is_null());
        assert!((result["cash_cny"].as_f64().unwrap() - 98995.0).abs() < 1e-6);
        assert!(stage(
            &db,
            &b,
            &ctx,
            &bad,
            Side::Sell,
            100,
            99900,
            "异常不成交",
            later.timestamp() + 18000,
            later
        )
        .is_err());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn quote_valuation_recovery_requires_all_fresh_marks_and_preserves_factor_blocks() {
        let (db, root, mut b, mut ctx) = test_account();
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap();
        stage(
            &db,
            &b,
            &ctx,
            &sample_tick(now),
            Side::Buy,
            100,
            100100,
            "买入",
            now.timestamp() + 300,
            now,
        )
        .unwrap();
        let buy = db.get_sim_detail(b.account_id).unwrap().orders[0].id;
        db.confirm_follow_order(buy, now.timestamp()).unwrap();
        let next = now + chrono::Duration::seconds(1);
        db.match_sim_order_live_at(buy, &sample_tick(next), next)
            .unwrap();
        ctx.as_of = "2026-09-30".into();
        ctx.session_dates = vec!["2026-09-18".into(), "2026-09-30".into()];
        b.as_of = ctx.as_of.clone();
        b.context_json = serde_json::to_string(&ctx).unwrap();
        b.valuation_blocked = true;
        b.valuation_block_reason = Some("quote_mismatch".into());
        b.state = "paused".into();
        db.save_follow_binding(&b).unwrap();
        assert!(view(&db, &b).unwrap()["equity_cny"].is_null());
        b.state = "waiting_session".into();
        db.save_follow_binding(&b).unwrap();
        assert!(view(&db, &b).unwrap()["equity_cny"].is_null());
        let today = Utc.with_ymd_and_hms(2026, 10, 8, 1, 30, 0).unwrap();
        assert!(db.recover_follow_valuation(&mut b, &[], today).is_err());
        assert!(b.valuation_blocked);
        let old = sample_tick(today - chrono::Duration::seconds(20));
        assert!(db.recover_follow_valuation(&mut b, &[old], today).is_err());
        assert!(b.valuation_blocked);
        db.recover_follow_valuation(&mut b, &[sample_tick(today)], today)
            .unwrap();
        assert!(!b.valuation_blocked);
        assert_eq!(
            db.follow_mark(b.account_id, "sh600000").unwrap(),
            Some((100000, today.timestamp()))
        );
        assert!(view(&db, &b).unwrap()["equity_cny"].is_number());
        b.valuation_blocked = true;
        b.valuation_block_reason = Some("factor_or_evidence_changed".into());
        db.save_follow_binding(&b).unwrap();
        assert!(db
            .recover_follow_valuation(&mut b, &[sample_tick(today)], today)
            .is_err());
        drop(db);
        let db = Database::open(root.clone()).unwrap();
        let persisted = db.follow_binding(b.account_id).unwrap();
        assert!(persisted.valuation_blocked);
        assert_eq!(
            persisted.valuation_block_reason.as_deref(),
            Some("factor_or_evidence_changed")
        );
        assert!(view(&db, &persisted).unwrap()["equity_cny"].is_null());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn old_follow_record_keeps_unknown_equity_when_review_field_was_missing() {
        let (db, root, mut b, _) = test_account();
        b.state = "company_action_review".into();
        let mut v = serde_json::to_value(&b).unwrap();
        v.as_object_mut().unwrap().remove("valuation_blocked");
        v.as_object_mut().unwrap().remove("valuation_block_reason");
        let legacy = crate::db::model_follow::decode_binding(&v.to_string()).unwrap();
        assert!(legacy.valuation_blocked);
        assert_eq!(
            legacy.valuation_block_reason.as_deref(),
            Some("legacy_review")
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod capital_gate_tests {
    use super::*;
    #[tokio::test]
    async fn capital_change_waits_for_active_model_tick_and_keeps_auto_state() {
        let root = std::env::temp_dir().join(format!("bull-capital-gate-{}", uuid::Uuid::new_v4()));
        let db = Arc::new(Database::open(root.clone()).unwrap());
        let account = db.save_sim_account(&AccountInput { id: None, name: "isolated capital gate".into(), initial_cash: "1000000000".into(), mode: "auto".into(), auto_enabled: true, manual_source_enabled: true, rule_source_enabled: true, ai_source_enabled: false, commission_bps: 3, min_commission: "50000".into(), stamp_tax_bps: 5, transfer_fee_bps: 0, slippage_bps: 0, targets: vec![] }).unwrap();
        let permit = manual_permit(gate()).await.unwrap();
        let task = { let db=db.clone(); tokio::spawn(async move { adjust_account_capital(&db, &crate::db::simulation::CapitalAdjustmentInput { account_id: account.id, initial_cash: 120000.01 }).await }) };
        tokio::task::yield_now().await;
        assert!(!task.is_finished()); assert_eq!(db.get_sim_detail(account.id).unwrap().account.current_cash, "1000000000");
        drop(permit);
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
        assert_eq!(result.current_cash, "1200000100"); assert!(result.auto_enabled);
        drop(db); assert_eq!(root.parent(), Some(std::env::temp_dir().as_path())); std::fs::remove_dir_all(root).unwrap();
    }
}
