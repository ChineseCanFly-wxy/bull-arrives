use crate::datasource::history::{self, LocalHistoryConfig, LocalHistoryResult};
use crate::db::{
    simulation::{AccountInput, OrderInput, SimAccount, SimDetail, SimOrder, Target},
    Database,
};
use crate::domain::KLineData;
use crate::quant::playbook::{self, TradeRule};
use crate::simulation::{RawBar, SCALE};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
static ACCOUNT_RUNS: OnceLock<Mutex<std::collections::HashSet<i64>>> = OnceLock::new();
struct AccountRun(i64);
impl Drop for AccountRun {
    fn drop(&mut self) {
        ACCOUNT_RUNS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}
use tauri::State;

#[tauri::command]
pub fn simulation_list_accounts(db: State<'_, Arc<Database>>) -> Result<Vec<SimAccount>, String> {
    db.list_sim_accounts().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn simulation_save_account(
    db: State<'_, Arc<Database>>,
    input: AccountInput,
    execution_mode: Option<String>,
) -> Result<SimAccount, String> {
    if let Some(id) = input.id {
        if db.experiment_account(id)?.is_some() {
            return Err(
                "研究账户的规则、股票池与费用已经冻结，请在研究中心管理；更改请建立新实验".into(),
            );
        }
        if db.live_account(id)? {
            let old = db.get_sim_targets(id)?;
            if old
                .iter()
                .map(|t| (&t.symbol, &t.rule, t.limit_bps))
                .collect::<Vec<_>>()
                != input
                    .targets
                    .iter()
                    .map(|t| (&t.symbol, &t.rule, t.limit_bps))
                    .collect::<Vec<_>>()
            {
                return Err(
                    "实时计划已建立，变更股票池或执行规则请新建账户，避免悄悄沿用旧买卖价".into(),
                );
            }
        }
    }
    if input.targets.len() > 10 {
        return Err("每个模拟账户最多可配置 10 个标的".into());
    }
    let is_new = input.id.is_none();
    let live = if let Some(id) = input.id {
        db.live_account(id)?
    } else {
        match execution_mode.as_deref().unwrap_or("realtime") {
            "realtime" => true,
            "daily" => false,
            _ => return Err("账户执行方式只能为 realtime / daily".into()),
        }
    };
    if input
        .targets
        .iter()
        .any(|t| !crate::simulation_live::is_a_share(&t.symbol))
    {
        return Err("模拟只支持沪深北 A 股股票".into());
    }
    let plans = if is_new && live && input.rule_source_enabled {
        super::simulation_live::prepare_plans(&db, &input.targets).await?
    } else {
        Vec::new()
    };
    let account = db.save_sim_account(&input)?;
    if is_new && live {
        db.enable_live_account(account.id, &plans)?;
    }
    Ok(account)
}

#[tauri::command]
pub fn simulation_delete_account(
    db: State<'_, Arc<Database>>,
    account_id: i64,
) -> Result<(), String> {
    if db.experiment_account(account_id)?.is_some() {
        return Err("研究账户必须保留验证历史，请在研究中心暂停或淘汰".into());
    }
    db.delete_sim_account(account_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn simulation_get_detail(
    db: State<'_, Arc<Database>>,
    account_id: i64,
) -> Result<SimDetail, String> {
    db.get_sim_detail(account_id)
}

#[tauri::command]
pub fn simulation_submit_order(
    db: State<'_, Arc<Database>>,
    mut input: OrderInput,
    limit_price: Option<String>,
    stop_price: Option<String>,
    take_price: Option<String>,
) -> Result<SimOrder, String> {
    if db.experiment_account(input.account_id)?.is_some() {
        return Err("不能向冻结研究账户插入手动指令".into());
    }
    input.source = "manual".into();
    input.ai_generated = false;
    if !db.live_account(input.account_id)? {
        if !crate::simulation_live::is_a_share(&input.symbol) {
            return Err("只支持沪深北 A 股代码".into());
        }
        input.signal_date = chrono::Utc::now()
            .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        return db.submit_sim_order(&input);
    }
    if !crate::simulation_live::is_a_share(&input.symbol) {
        return Err("只允许完整沪深北 A 股代码，例如 sh600000".into());
    }
    let limit = crate::simulation::parse_scaled(
        limit_price.as_deref().ok_or("请填写实时委托限价")?,
        "限价",
    )?;
    if limit <= 0 {
        return Err("限价必须大于零".into());
    }
    let now = chrono::Utc::now();
    input.signal_date = now
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .format("%Y-%m-%d")
        .to_string();
    if input.side == "buy" {
        let stop = crate::simulation::parse_scaled(
            stop_price.as_deref().ok_or("买入时须设置止损价")?,
            "止损价",
        )?;
        let take = crate::simulation::parse_scaled(
            take_price.as_deref().ok_or("买入时须设置止盈价")?,
            "止盈价",
        )?;
        db.set_live_manual_plan(
            input.account_id,
            &crate::db::simulation_live::LivePlan {
                symbol: input.symbol.clone(),
                buy_low: limit,
                buy_high: limit,
                stop,
                take,
                limit_bps: crate::market_rules::ensure_simulatable(&input.symbol, &input.name)?,
                basis_date: input.signal_date.clone(),
                reference_close: 0,
                position_pct: 0.0,
            },
        )?;
    }
    let order = db.submit_sim_order(&input)?;
    db.live_order(order.id, limit, now.timestamp())?;
    Ok(order)
}

#[tauri::command]
pub fn simulation_confirm_order(
    db: State<'_, Arc<Database>>,
    order_id: i64,
) -> Result<SimOrder, String> {
    let result = db.confirm_sim_order(order_id)?;
    if db.live_account(result.account_id)? {
        db.rearm_live_order(order_id)?;
    }
    Ok(result)
}

#[tauri::command]
pub async fn simulation_run(
    db: State<'_, Arc<Database>>,
    manager: State<'_, Arc<crate::datasource::DataSourceManager>>,
    account_id: i64,
) -> Result<SimDetail, String> {
    if db.live_account(account_id)? {
        super::simulation_live::run(&db, &manager, account_id).await
    } else {
        run_account(&db, account_id).await
    }
}

/// Run one account from locally stored history. QFQ bars decide at T close;
/// only the following raw bar may mutate the cash/lot ledger.
pub async fn run_account(db: &Database, account_id: i64) -> Result<SimDetail, String> {
    if db.live_account(account_id)? {
        return Err("实时账户禁止使用日线回放撮合".into());
    }
    if !ACCOUNT_RUNS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(account_id)
    {
        return Err("该账户正在运行，请勿重复推进".into());
    }
    let _run = AccountRun(account_id);
    if let Some(state) = db.experiment_account(account_id)? {
        if !matches!(state.as_str(), "observing" | "extended" | "adopted") {
            return Err("研究账户当前已暂停、待采纳或淘汰，请到研究中心查看".into());
        }
        let now = chrono::Utc::now();
        let local = now.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
        let schedule = db.get_setting("quote_schedule").ok().flatten();
        let policy =
            crate::datasource::market_policy::MarketRequestPolicy::from_quote_schedule_json(
                schedule.as_deref(),
            )?;
        use chrono::Timelike;
        if policy.is_trading_day_at(now) && local.hour() < 16 {
            return db.get_sim_detail(account_id);
        }
    }
    let mut detail = db.get_sim_detail(account_id)?;
    if detail.targets.len() > 10 {
        return Err("每个模拟账户最多可配置 10 个标的".into());
    }
    let mut execution: Vec<(String, String, Option<Target>)> = detail
        .targets
        .iter()
        .map(|target| {
            (
                target.symbol.clone(),
                target.name.clone(),
                Some(target.clone()),
            )
        })
        .collect();
    for order in detail
        .orders
        .iter()
        .filter(|order| order.status == "pending")
    {
        if !execution.iter().any(|item| item.0 == order.symbol) {
            execution.push((order.symbol.clone(), order.name.clone(), None));
        }
    }
    for position in &detail.positions {
        if !execution.iter().any(|item| item.0 == position.symbol) {
            execution.push((position.symbol.clone(), position.name.clone(), None));
        }
    }
    if execution.is_empty() {
        return Ok(detail);
    }
    let enabled = db
        .get_setting("local_history_enabled")
        .map_err(|error| error.to_string())?
        .is_some_and(|value| value == "1");
    if !enabled {
        return Err("模拟成交需要本地未复权历史数据，请先启用本地历史引擎".into());
    }
    let url = db
        .get_setting("local_history_url")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "http://127.0.0.1:7899".into());
    let config = LocalHistoryConfig::new(url);

    let mut histories = Vec::with_capacity(execution.len());
    for (symbol, name, target) in execution {
        let data = history::fetch_daily(&config, &symbol, None, None).await?;
        histories.push((symbol, name, target, data));
    }
    let marks: HashMap<String, (String, i64)> = histories
        .iter()
        .filter_map(|(symbol, _, _, data)| {
            data.raw_klines.last().and_then(|bar| {
                scaled(bar.close)
                    .ok()
                    .map(|price| (symbol.clone(), (bar.date.clone(), price)))
            })
        })
        .collect();
    let trade_date = marks
        .values()
        .map(|(date, _)| date.as_str())
        .max()
        .unwrap_or_default()
        .to_string();
    if db.experiment_account(account_id)?.is_some() {
        let today = chrono::Utc::now()
            .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        if trade_date > today {
            return Err("本地行情包含未来日期，拒绝模拟".into());
        }
        if marks.values().any(|(date, _)| date != &trade_date) {
            return Err("研究账户各标的估值日期不一致，请更新本地数据后重试".into());
        }
        let curve = db.sim_equity_curve(account_id)?;
        if let Some(last) = curve.last() {
            if trade_date < last.date {
                return Err("行情早于已记录净值，拒绝倒退重算".into());
            }
            if histories
                .iter()
                .any(|(_, _, _, h)| h.raw_klines.iter().filter(|b| b.date > last.date).count() > 1)
            {
                return Err("本地日线跨过多个未记录交易日，当前引擎无法完整补记中间净值；请暂停并新建实验，不能把缺口算成验证通过".into());
            }
        }
    }
    let benchmark_close = match history::fetch_daily(&config, "000300", None, None).await {
        Ok(data) => data
            .raw_klines
            .iter()
            .find(|bar| bar.date == trade_date)
            .and_then(|bar| scaled(bar.close).ok()),
        Err(error) => {
            log::warn!("模拟账户基准沪深300读取失败：{}", error);
            None
        }
    };
    let pending_key = detail
        .orders
        .iter()
        .filter(|order| order.status == "pending")
        .map(|order| format!("{}:{}", order.id, order.status))
        .collect::<Vec<_>>()
        .join(",");
    let run_key = format!(
        "{}|pending={}",
        histories
            .iter()
            .map(|(symbol, _, target, data)| format!(
                "{}={}@{}:{}",
                symbol,
                data.end_date.as_deref().unwrap_or("-"),
                target
                    .as_ref()
                    .map(|value| value.rule.as_str())
                    .unwrap_or("manual"),
                target
                    .as_ref()
                    .map(|value| value.limit_bps)
                    .unwrap_or_default()
            ))
            .collect::<Vec<_>>()
            .join(";"),
        pending_key
    );
    let Some(run) = db.begin_sim_run(account_id, &run_key)? else {
        return db.get_sim_detail(account_id);
    };

    let result = (|| -> Result<SimDetail, String> {
        if let Some(state) = db.experiment_account(account_id)? {
            if !matches!(state.as_str(), "observing" | "extended" | "adopted") {
                return Err("研究账户已暂停，未继续处理指令".into());
            }
        }
        let mut pending = Vec::new();
        for order in detail
            .orders
            .iter()
            .filter(|order| order.status == "pending")
        {
            let Some(data) = histories
                .iter()
                .find(|item| item.0 == order.symbol)
                .map(|item| &item.3)
            else {
                continue;
            };
            let Some(index) = data
                .raw_klines
                .iter()
                .position(|bar| bar.date > order.signal_date)
            else {
                continue;
            };
            if index == 0 {
                continue;
            }
            let bar = raw_bar(&data.raw_klines, index)?;
            pending.push((
                bar.date.clone(),
                order.id,
                order.symbol.clone(),
                order.side.clone(),
                bar,
            ));
        }
        sort_pending(&mut pending);
        for (date, order_id, symbol, side, bar) in pending {
            detail = db.get_sim_detail(account_id)?;
            if side == "sell" {
                let data = histories
                    .iter()
                    .find(|item| item.0 == symbol)
                    .map(|item| &item.3)
                    .ok_or_else(|| format!("{} 缺少未复权成交数据", symbol))?;
                ensure_no_company_action_until(&detail, &symbol, data, &date)?;
            }
            db.match_sim_order_raw(order_id, &bar)?;
        }
        detail = db.get_sim_detail(account_id)?;
        let target_count = histories.len();
        for (target_index, (symbol, name, target, data)) in histories.into_iter().enumerate() {
            db.update_sim_run_progress(run.id, (target_index * 100 / target_count.max(1)) as i64)?;
            ensure_no_company_action_until(
                &detail,
                &symbol,
                &data,
                data.raw_klines
                    .last()
                    .map(|bar| bar.date.as_str())
                    .unwrap_or_default(),
            )?;

            let (signal_index, signal_bar) = latest_common_bar(&data)?;
            // A strategy selected using today's market cannot be backdated to an earlier signal.
            if db.experiment_account(account_id)?.is_some() {
                let selected_day = detail.account.created_at.get(..10).unwrap_or("");
                if signal_bar.date.as_str() < selected_day {
                    continue;
                }
            }
            let qfq = &data.klines[..=signal_index];
            let raw = &data.raw_klines[signal_index];

            for position in detail
                .positions
                .iter()
                .filter(|position| position.symbol == symbol)
            {
                if position.available_quantity <= 0 {
                    continue;
                }
                let cost = position
                    .cost_price
                    .parse::<i64>()
                    .map_err(|_| "持仓成本数据无效".to_string())?;
                let close = scaled(raw.close)?;
                let held_days = data
                    .raw_klines
                    .iter()
                    .filter(|bar| {
                        bar.date.as_str() > position.acquired_date.as_str()
                            && bar.date.as_str() <= signal_bar.date.as_str()
                    })
                    .count() as i64;
                let stop = position.stop_bps > 0
                    && close.saturating_mul(10_000)
                        <= cost.saturating_mul(10_000 - position.stop_bps);
                let take = position.take_bps > 0
                    && close.saturating_mul(10_000)
                        >= cost.saturating_mul(10_000 + position.take_bps);
                let expired = position.max_hold_days > 0 && held_days >= position.max_hold_days;
                if stop || take || expired {
                    let reason = if stop {
                        "stop"
                    } else if take {
                        "take"
                    } else {
                        "time"
                    };
                    db.submit_sim_order(&OrderInput {
                        account_id,
                        idempotency_key: format!(
                            "rule:{}:{}:sell:{}:{reason}",
                            symbol, signal_bar.date, position.lot_id
                        ),
                        symbol: symbol.clone(),
                        name: name.clone(),
                        side: "sell".into(),
                        quantity: position.available_quantity,
                        signal_date: signal_bar.date.clone(),
                        source: "risk".into(),
                        rule: target.as_ref().map(|value| value.rule.clone()),
                        stop_bps: 0,
                        take_bps: 0,
                        limit_bps: position.limit_bps,
                        max_hold_days: 0,
                        ai_generated: false,
                    })?;
                }
            }

            let Some(target) = target else { continue };
            if !detail.account.rule_source_enabled {
                continue;
            }
            if detail
                .positions
                .iter()
                .any(|position| position.symbol == symbol && position.quantity > 0)
            {
                continue;
            }
            let rule = target_rule(&target.rule, qfq)?;
            let audit = crate::quant::causal::audit(qfq, rule);
            if !audit.passed {
                return Err(audit.message);
            }
            let Some(plan) = playbook::plan(qfq, rule).filter(|plan| plan.ready) else {
                continue;
            };
            let cash = detail
                .account
                .current_cash
                .parse::<i64>()
                .map_err(|_| "账户现金数据无效".to_string())?;
            let allocation_pct = plan.position_pct.min(100.0 / detail.targets.len() as f64);
            let quantity =
                ((cash as f64 * allocation_pct / 100.0 / scaled(raw.close)? as f64) as i64 / 100)
                    * 100;
            if quantity == 0 {
                continue;
            }
            let stop_bps = ((plan.reference_price - plan.stop_loss) / plan.reference_price
                * 10_000.0)
                .round() as i64;
            let take_bps = ((plan.take_profit - plan.reference_price) / plan.reference_price
                * 10_000.0)
                .round() as i64;
            db.submit_sim_order(&OrderInput {
                account_id,
                idempotency_key: format!("rule:{}:{}:buy", target.symbol, signal_bar.date),
                symbol: target.symbol.clone(),
                name: target.name.clone(),
                side: "buy".into(),
                quantity,
                signal_date: signal_bar.date.clone(),
                source: "rule".into(),
                rule: Some(rule.id().into()),
                stop_bps,
                take_bps,
                limit_bps: target.limit_bps,
                max_hold_days: rule.max_hold_days() as i64,
                ai_generated: false,
            })?;
            detail = db.get_sim_detail(account_id)?;
        }
        db.get_sim_detail(account_id)
    })();
    let result = result.and_then(|detail| {
        let market_value = detail.positions.iter().try_fold(0i64, |sum, position| {
            let price = marks
                .get(&position.symbol)
                .map(|item| item.1)
                .ok_or_else(|| format!("{} 缺少盯市价格", position.symbol))?;
            sum.checked_add(
                price
                    .checked_mul(position.quantity)
                    .ok_or_else(|| "市值超出安全范围".to_string())?,
            )
            .ok_or_else(|| "市值超出安全范围".to_string())
        })?;
        if !trade_date.is_empty() {
            db.mark_sim_equity(account_id, &trade_date, market_value, benchmark_close)?;
        }
        Ok(detail)
    });
    match &result {
        Ok(_) => {
            db.finish_sim_run(run.id, "completed", None)?;
        }
        Err(message) => {
            db.finish_sim_run(run.id, "failed", Some(message))?;
        }
    }
    result
        .map(|_| ())
        .and_then(|_| db.get_sim_detail(account_id))
}

fn target_rule(value: &str, bars: &[KLineData]) -> Result<TradeRule, String> {
    if value.trim().is_empty() || value == "auto" {
        Ok(playbook::match_rule(bars)
            .map(|matched| matched.recommended)
            .unwrap_or(TradeRule::TrendFollow))
    } else {
        TradeRule::try_from_id(value)
    }
}

fn ensure_no_company_action_until(
    detail: &SimDetail,
    symbol: &str,
    data: &LocalHistoryResult,
    through_date: &str,
) -> Result<(), String> {
    let Some((_, latest_qfq, latest_raw)) =
        data.klines
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, qfq)| {
                data.raw_klines
                    .get(index)
                    .filter(|raw| raw.date == qfq.date && raw.date.as_str() <= through_date)
                    .map(|raw| (index, qfq, raw))
            })
    else {
        return Err("前复权与未复权日 K 日期不对齐".into());
    };
    for position in detail
        .positions
        .iter()
        .filter(|position| position.symbol == symbol)
    {
        let Some((entry_qfq, entry_raw)) =
            data.klines.iter().zip(&data.raw_klines).find(|(qfq, raw)| {
                qfq.date == position.acquired_date && raw.date == position.acquired_date
            })
        else {
            return Err(format!("{} 缺少买入日复权对照，已暂停自动成交", symbol));
        };
        if factor_changed(
            entry_qfq.close,
            entry_raw.close,
            latest_qfq.close,
            latest_raw.close,
        ) {
            return Err(format!(
                "{} 持仓期检测到除权除息；公司行动处理尚未实现，已暂停自动成交与估值",
                symbol
            ));
        }
    }
    Ok(())
}

fn factor_changed(entry_qfq: f64, entry_raw: f64, latest_qfq: f64, latest_raw: f64) -> bool {
    let entry_ratio = entry_raw / entry_qfq;
    let latest_ratio = latest_raw / latest_qfq;
    !entry_ratio.is_finite()
        || !latest_ratio.is_finite()
        || (entry_ratio / latest_ratio - 1.0).abs() > 0.0001
}

fn sort_pending(items: &mut [(String, i64, String, String, RawBar)]) {
    items.sort_by(|left, right| (left.0.as_str(), left.1).cmp(&(right.0.as_str(), right.1)));
}

fn latest_common_bar(data: &LocalHistoryResult) -> Result<(usize, &KLineData), String> {
    data.klines
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, qfq)| {
            data.raw_klines
                .iter()
                .find(|raw| raw.date == qfq.date)
                .map(|raw| (index, raw))
        })
        .ok_or_else(|| "前复权与未复权日 K 日期不对齐".to_string())
}

fn raw_bar(bars: &[KLineData], index: usize) -> Result<RawBar, String> {
    let bar = bars
        .get(index)
        .ok_or_else(|| "未复权日 K 不存在".to_string())?;
    let previous = index
        .checked_sub(1)
        .and_then(|i| bars.get(i))
        .ok_or_else(|| "未复权日 K 缺少前收盘价".to_string())?;
    Ok(RawBar {
        date: bar.date.clone(),
        open: scaled(bar.open)?.to_string(),
        high: scaled(bar.high)?.to_string(),
        low: scaled(bar.low)?.to_string(),
        close: scaled(bar.close)?.to_string(),
        prev_close: scaled(previous.close)?.to_string(),
        volume: bar.volume,
    })
}

fn scaled(value: f64) -> Result<i64, String> {
    if !value.is_finite() || value <= 0.0 {
        return Err("未复权价格无效".into());
    }
    let value = (value * SCALE as f64).round();
    if value > crate::simulation::MAX_MONEY as f64 {
        return Err("未复权价格超出安全范围".into());
    }
    Ok(value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_execution_uses_previous_unadjusted_close() {
        let bars = vec![
            KLineData {
                date: "2026-01-02".into(),
                open: 10.0,
                high: 10.5,
                low: 9.8,
                close: 10.2,
                volume: 1,
                turnover: 0.0,
            },
            KLineData {
                date: "2026-01-05".into(),
                open: 10.3,
                high: 10.6,
                low: 10.1,
                close: 10.4,
                volume: 2,
                turnover: 0.0,
            },
        ];
        let bar = raw_bar(&bars, 1).unwrap();
        assert_eq!(bar.open, "103000");
        assert_eq!(bar.prev_close, "102000");
        assert!(!factor_changed(10.0, 10.0, 11.0, 11.0));
        assert!(factor_changed(5.0, 10.0, 11.0, 11.0));
        let mut pending = vec![
            (
                "2026-01-06".into(),
                1,
                "sz000001".into(),
                "buy".into(),
                raw_bar(&bars, 1).unwrap(),
            ),
            (
                "2026-01-05".into(),
                9,
                "sh600000".into(),
                "buy".into(),
                raw_bar(&bars, 1).unwrap(),
            ),
            (
                "2026-01-05".into(),
                2,
                "sh600519".into(),
                "buy".into(),
                raw_bar(&bars, 1).unwrap(),
            ),
        ];
        sort_pending(&mut pending);
        assert_eq!(
            pending.iter().map(|item| item.1).collect::<Vec<_>>(),
            vec![2, 9, 1]
        );
    }
}
