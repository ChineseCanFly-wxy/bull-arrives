use crate::{
    datasource::DataSourceManager,
    db::{
        simulation::{OrderInput, SimDetail},
        simulation_live::LiveStatus,
        Database,
    },
    simulation_live::{scaled, LiveTick},
};
use std::sync::{Arc, OnceLock};
use tauri::State;
static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

pub async fn prepare_plans(
    db: &Database,
    targets: &[crate::db::simulation::Target],
) -> Result<Vec<crate::db::simulation_live::LivePlan>, String> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    if db
        .get_setting("local_history_enabled")
        .ok()
        .flatten()
        .as_deref()
        != Some("1")
    {
        return Err(
            "规则买卖计划需要已完成的本地日线；请启用本地历史，或先留空股票池手动下限价单".into(),
        );
    }
    let url = db
        .get_setting("local_history_url")
        .ok()
        .flatten()
        .unwrap_or("http://127.0.0.1:7899".into());
    let config = crate::datasource::history::LocalHistoryConfig::new(url);
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .format("%Y-%m-%d")
        .to_string();
    let mut plans = Vec::new();
    for target in targets {
        if !crate::simulation_live::is_a_share(&target.symbol) {
            return Err("规则仅支持沪深北 A 股完整代码".into());
        }
        let data =
            crate::datasource::history::fetch_daily(&config, &target.symbol, None, None).await?;
        let end = data
            .klines
            .iter()
            .rposition(|b| b.date < today)
            .ok_or("缺少已完成日线")?;
        if end < 59 {
            return Err("至少需要 60 根已完成日线生成规则计划".into());
        }
        let rule = crate::quant::playbook::TradeRule::try_from_id(&target.rule)?;
        let plan =
            crate::quant::playbook::plan(&data.klines[..=end], rule).ok_or("无法生成买卖区间")?;
        let factor = data.raw_klines[end].close / data.klines[end].close;
        let price = |v: f64| scaled(v * factor);
        plans.push(crate::db::simulation_live::LivePlan {
            symbol: target.symbol.clone(),
            buy_low: price(plan.buy_low)?,
            buy_high: price(plan.buy_high)?,
            stop: price(plan.stop_loss)?,
            take: price(plan.take_profit)?,
            limit_bps: crate::market_rules::ensure_simulatable(&target.symbol, &target.name)?,
            basis_date: data.klines[end].date.clone(),
            reference_close: price(data.klines[end].close)?,
            position_pct: plan.position_pct,
        });
    }
    Ok(plans)
}

#[tauri::command]
pub fn simulation_live_status(
    db: State<'_, Arc<Database>>,
    account_id: i64,
) -> Result<LiveStatus, String> {
    db.live_status(account_id)
}
pub async fn snapshot(manager: &DataSourceManager, symbol: &str) -> Result<LiveTick, String> {
    manager.ensure_request_allowed()?;
    let active = manager.active_name();
    let mut sources = manager.all_sources();
    sources.sort_by_key(|(name, _)| *name != active);
    let mut last = "无实时行情通道".to_string();
    for (name, source) in sources {
        let codes = [symbol.to_string()];
        let (quotes, depth) = tokio::join!(
            source.fetch_realtime(&codes, "CN"),
            source.fetch_depth(symbol, "CN")
        );
        let Ok(quotes) = quotes else {
            last = format!("{name} 报价失败");
            continue;
        };
        let Some(quote) = quotes.into_iter().find(|q| q.code == symbol) else {
            last = format!("{name} 未返回 {symbol}");
            continue;
        };
        let Ok(depth) = depth else {
            last = format!("{name} 盘口不可用");
            continue;
        };
        let tick = LiveTick {
            quote,
            depth,
            source: format!("{name} quote / Tencent depth"),
            received_at: chrono::Utc::now().timestamp(),
        };
        match crate::simulation_live::validate(&tick, chrono::Utc::now()) {
            Ok(()) => return Ok(tick),
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub async fn run(
    db: &Database,
    manager: &DataSourceManager,
    account: i64,
) -> Result<SimDetail, String> {
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "实时模拟正在运行")?;
    if !db.live_account(account)? {
        return Err("旧日线账户已保留为历史记录，请新建实时账户".into());
    }
    if let Some(state) = db.experiment_account(account)? {
        if !matches!(state.as_str(), "observing" | "extended" | "adopted") {
            return Err("实验已暂停或结束".into());
        }
    }
    let now = chrono::Utc::now();
    let schedule = db.get_setting("quote_schedule").ok().flatten();
    let policy = crate::datasource::market_policy::MarketRequestPolicy::from_quote_schedule_json(
        schedule.as_deref(),
    )?;
    let gate = crate::datasource::a_share_calendar::continuous(now).and_then(|_| {
        if policy.is_trading_day_at(now) {
            Ok(())
        } else {
            Err("A 股休市或用户额外休市日".into())
        }
    });
    if let Err(reason) = gate {
        db.live_message(account, &reason)?;
        return db.get_sim_detail(account);
    }
    let initial = db.get_sim_detail(account)?;
    let plans = db.live_status(account)?.plans;
    let mut symbols: Vec<_> = initial
        .targets
        .iter()
        .map(|t| t.symbol.clone())
        .chain(initial.positions.iter().map(|p| p.symbol.clone()))
        .chain(
            initial
                .orders
                .iter()
                .filter(|o| o.status == "pending")
                .map(|o| o.symbol.clone()),
        )
        .collect();
    symbols.sort();
    symbols.dedup();
    if symbols.len() > 20 {
        return Err("实时模拟每账户最多 20 个关联标的".into());
    }
    let mut ticks = Vec::new();
    let mut messages = Vec::new();
    for symbol in symbols {
        match snapshot(manager, &symbol).await {
            Ok(tick) => ticks.push(tick),
            Err(e) => messages.push(format!("{symbol}: {e}")),
        }
    }
    let mut benchmark = None;
    for (_, source) in manager.all_sources() {
        if let Ok(rows) = source.fetch_realtime(&["sh000300".into()], "CN").await {
            if let Some(q) = rows.iter().find(|q| {
                q.code == "sh000300"
                    && q.timestamp > 0
                    && chrono::Utc::now().timestamp() - q.timestamp <= 10
                    && q.timestamp <= chrono::Utc::now().timestamp() + 1
            }) {
                benchmark = scaled(q.price).ok();
                break;
            }
        }
    }
    // Establish benchmark before any fill; a missing initial benchmark stays unavailable.
    for tick in &ticks {
        if let Err(reason) = db.live_reference_check(account, tick) {
            db.live_message(account, &reason)?;
            return Err(reason);
        }
    }
    if db.get_sim_detail(account)?.positions.is_empty() {
        let _ = db.record_live_equity(account, &ticks, benchmark);
    }
    let date = now
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .format("%Y-%m-%d")
        .to_string();
    // Never carry an executable limit from an earlier trading day into a gap opening.
    db.expire_live_orders(account, &date)?;
    for tick in &ticks {
        if let Err(reason) = crate::simulation_live::validate(tick, chrono::Utc::now()) {
            messages.push(reason);
            continue;
        }
        let mut detail = db.get_sim_detail(account)?;
        let pending: Vec<_> = detail
            .orders
            .iter()
            .filter(|o| o.symbol == tick.quote.code && o.status == "pending")
            .map(|o| o.id)
            .collect();
        let had_pending = !pending.is_empty();
        for id in pending.into_iter().take(1) {
            let order = match db.match_sim_order_live(id, tick) {
                Ok(o) => o,
                Err(e) => {
                    messages.push(e);
                    continue;
                }
            };
            if order.status == "pending" {
                messages.push(format!(
                    "{} {}",
                    order.symbol,
                    order.reject_reason.unwrap_or("等待限价".into())
                ));
            }
        }
        if had_pending {
            continue;
        }
        detail = db.get_sim_detail(account)?;
        let Some(plan) = plans.iter().find(|p| p.symbol == tick.quote.code) else {
            continue;
        };
        if plan.reference_close > 0
            && ((scaled(tick.quote.prev_close)? as f64 / plan.reference_close as f64) - 1.0).abs()
                > 0.5
        {
            messages.push(format!(
                "{} 较计划参考价变化过大，等待复核公司行为",
                plan.symbol
            ));
            continue;
        }
        let price = scaled(tick.quote.price)?;
        let positions: Vec<_> = detail
            .positions
            .iter()
            .filter(|p| p.symbol == plan.symbol)
            .collect();
        if !positions.is_empty() {
            if price >= plan.take || price <= plan.stop {
                let available: i64 = positions
                    .iter()
                    .filter(|p| p.acquired_date < date)
                    .map(|p| p.quantity)
                    .sum();
                if available <= 0 {
                    messages.push(format!("{} 已触发卖点，但 T+1 今日买入不可卖", plan.symbol));
                    continue;
                }
                let limit = if price >= plan.take {
                    plan.take
                } else {
                    ((price as i128 * (10000 - detail.account.slippage_bps) as i128 / 10000) / 100
                        * 100) as i64
                };
                let order = db.submit_sim_order(&OrderInput {
                    account_id: account,
                    idempotency_key: format!("live:{}:{}:sell", plan.symbol, date),
                    symbol: plan.symbol.clone(),
                    name: tick.quote.name.clone(),
                    side: "sell".into(),
                    quantity: available,
                    signal_date: date.clone(),
                    source: "risk".into(),
                    rule: None,
                    stop_bps: 0,
                    take_bps: 0,
                    limit_bps: plan.limit_bps,
                    max_hold_days: 0,
                    ai_generated: false,
                })?;
                db.live_order(order.id, limit, chrono::Utc::now().timestamp())?;
                messages.push(format!("{} 卖出触发，等待新盘口验证限价", plan.symbol));
            }
        } else if detail.account.rule_source_enabled
            && price >= plan.buy_low
            && price <= plan.buy_high
        {
            let cash = detail
                .account
                .current_cash
                .parse::<i64>()
                .map_err(|_| "账户现金无效")?;
            let allocation = plan.position_pct.min(100.0 / plans.len().max(1) as f64);
            let quantity =
                ((cash as f64 * allocation / 100.0 / plan.buy_high as f64) as i64 / 100) * 100;
            if quantity < 100 {
                continue;
            }
            let order = db.submit_sim_order(&OrderInput {
                account_id: account,
                idempotency_key: format!("live:{}:{}:buy", plan.symbol, date),
                symbol: plan.symbol.clone(),
                name: tick.quote.name.clone(),
                side: "buy".into(),
                quantity,
                signal_date: date.clone(),
                source: "rule".into(),
                rule: detail
                    .targets
                    .iter()
                    .find(|t| t.symbol == plan.symbol)
                    .map(|t| t.rule.clone()),
                stop_bps: 0,
                take_bps: 0,
                limit_bps: plan.limit_bps,
                max_hold_days: 0,
                ai_generated: false,
            })?;
            db.live_order(order.id, plan.buy_high, chrono::Utc::now().timestamp())?;
            messages.push(format!("{} 进入买入区间，等待新盘口验证限价", plan.symbol));
        }
    }
    if let Err(e) = db.record_live_equity(account, &ticks, benchmark) {
        messages.push(e);
    }
    if messages.is_empty() {
        messages.push("实时监听中：尚无新的买卖触发；未补算历史成交".into());
    }
    db.live_message(account, &messages.join("；"))?;
    if let Some(e) = db
        .research_experiments()?
        .into_iter()
        .find(|e| e.account_id == Some(account))
    {
        super::research::evaluate(db, e.id)?;
    }
    db.get_sim_detail(account)
}

pub async fn tick_all(db: &Database, manager: &DataSourceManager) {
    let Ok(ids) = db.list_auto_sim_account_ids() else {
        return;
    };
    for id in ids {
        if db.live_account(id).unwrap_or(false) {
            if let Err(e) = run(db, manager, id).await {
                let _ = db.live_message(id, &e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::datasource::DataSource;
    #[tokio::test]
    #[ignore = "manual network smoke; no account writes or fills"]
    async fn quote_depth_timestamps_are_from_provider() {
        let source = crate::datasource::tencent::TencentAdapter::new();
        let symbol = "sh600000";
        let codes = vec![symbol.into()];
        let (quote, depth) = tokio::join!(
            source.fetch_realtime(&codes, "CN"),
            source.fetch_depth(symbol, "CN")
        );
        let quote = quote.unwrap();
        let depth = depth.unwrap();
        assert_eq!(quote[0].code, symbol);
        assert!(quote[0].timestamp > 0);
        assert!(depth.timestamp > 0);
        assert_eq!(depth.code, symbol);
        assert_eq!(quote[0].timestamp, depth.timestamp);
        println!("Provider timestamps parsed; no freshness or fill claim outside trading hours");
    }
}
