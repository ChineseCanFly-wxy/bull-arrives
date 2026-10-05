//! Read-only model-data state. Missing data never schedules a popup or changes assets.
use crate::db::Database;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
fn source_rows(db: &Database, expected: NaiveDate) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    for binding in db.follow_bindings()?.into_iter().filter(|b| b.enabled) {
        let run = db.model_run(binding.source_run_id);
        let as_of = run
            .as_ref()
            .ok()
            .and_then(|v| v["as_of"].as_str())
            .unwrap_or("");
        let ready = as_of == expected.to_string();
        let recorded = db
            .get_setting(&format!(
                "model_source_data_error_{}",
                binding.source_run_id
            ))
            .map_err(|e| e.to_string())?
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        let error = if ready {
            None
        } else {
            recorded
                .and_then(|v| v["error"].as_str().map(str::to_owned))
                .or_else(|| run.as_ref().err().cloned())
                .or_else(|| db.get_setting("model_research_last_error").ok().flatten())
                .filter(|v| !v.is_empty())
        };
        rows.push(json!({"account_id":binding.account_id,"source_run_id":binding.source_run_id,"model_id":binding.model_id,"model_name":binding.model_name,"as_of":as_of,"expected_as_of":expected.to_string(),"ready":ready,"error":error,"state":binding.state,"message":binding.message}));
    }
    Ok(rows)
}
pub fn status(db: &Database, now: DateTime<Utc>) -> Result<Value, String> {
    let expected = crate::commands::research::model_completed_day(now)?;
    let accounts = source_rows(db, expected)?;
    let missing = accounts.iter().filter(|row| row["ready"] != true).count();
    let last = db
        .get_setting("model_market_data_status")
        .map_err(|e| e.to_string())?
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({"mode":"saved_snapshot"}));
    Ok(
        json!({"schema":"model-market-data-health-v1","checked_at":now.to_rfc3339(),"expected_as_of":expected.to_string(),"enabled_accounts":accounts.len(),"missing_accounts":missing,"state":if accounts.is_empty(){"no_active_accounts"}else if missing==0{"ready"}else{"waiting_data"},"last_refresh":last,"accounts":accounts,"policy":"StockDB优先；近期完成日缺口从新浪/腾讯补齐。盘中报价与盘口继续走现有行情源。","limitation":"备用只补与已有快照相邻的完成交易日；跨多日缺口、覆盖不足或复权未核对时后台等待数据恢复，未认证股票不可用"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasource::trading_calendar;
    use chrono::TimeZone;
    fn at(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour - 8, minute, 0)
            .unwrap()
    }
    #[test]
    fn data_status_is_readonly_and_paused_sources_are_excluded() {
        use crate::db::{model_follow::FollowBinding, simulation::AccountInput};
        let root = std::env::temp_dir().join(format!("bull-data-health-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let source=db.save_model_run(include_str!("../../research/research-center-runner/checks/verified-rank-forward-b6cce0d8-167d-4c84-ad84-32f5b6800312.json"),None).unwrap();
        let account = db
            .save_sim_account(&AccountInput {
                id: None,
                name: "数据提醒测试".into(),
                initial_cash: "1000000000".into(),
                mode: "auto".into(),
                auto_enabled: true,
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
        let mut binding = FollowBinding {
            account_id: account.id,
            source_run_id: source,
            model_id: "breadth22_rank20".into(),
            model_name: "隔离模型".into(),
            max_positions: 3,
            allocation_policy: "baseline8".into(),
            execution_policy: "baseline".into(),
            enabled: true,
            auto_execute: true,
            initial_cash_cny: 100000.,
            as_of: "2026-09-18".into(),
            source_sha256: "fixture".into(),
            context_json: "{}".into(),
            state: "waiting_model_update".into(),
            message: "不涉及实际交易".into(),
            allocation_date: None,
            allocation_equity: None,
            valuation_blocked: false,
            valuation_block_reason: None,
            candidate_cursor: 0,
        };
        db.save_follow_binding(&binding).unwrap();
        assert!(
            db.enabled_model_runs()
                .unwrap()
                .iter()
                .any(|run| run["id"] == source),
            "自动账户必须持续取得来源，手动观察状态不得影响它"
        );
        assert!(
            db.enable_model_observation(source, false).is_err(),
            "手动来源开关不能停用自动账户所需数据"
        );

        db.set_setting("research_notifications_enabled", "0")
            .unwrap();
        db.set_setting(
            &format!("model_source_data_error_{source}"),
            &json!({"error":"StockDB未更新；备用日期不符"}).to_string(),
        )
        .unwrap();
        let balances = db.get_sim_detail(account.id).unwrap().account;
        for time in [at(28, 9, 20), at(28, 9, 35), at(28, 16, 30)] {
            let state = status(&db, time).unwrap();
            assert_eq!(state["missing_accounts"], 1);
            assert!(state["accounts"][0]["error"]
                .as_str()
                .unwrap()
                .contains("StockDB未更新"));
            assert!(state.get("alerts_enabled").is_none());
        }
        assert!(db
            .get_setting("model_data_alerted_2026-09-28_pre_open_1")
            .unwrap()
            .is_none());
        binding.enabled = false;
        db.save_follow_binding_for_user(&binding).unwrap();
        assert_eq!(
            status(&db, at(28, 16, 30)).unwrap()["state"],
            "no_active_accounts"
        );
        binding.enabled = true;
        db.save_follow_binding_for_user(&binding).unwrap();
        let source_day = NaiveDate::parse_from_str(
            db.model_run(source).unwrap()["as_of"].as_str().unwrap(),
            "%Y-%m-%d",
        )
        .unwrap();
        let mut ready_day = source_day + chrono::Duration::days(1);
        while !trading_calendar::is_trading_day_at(Utc::now(), ready_day).unwrap() {
            ready_day += chrono::Duration::days(1);
        }
        assert_eq!(
            status(&db, ready_day.and_hms_opt(1, 20, 0).unwrap().and_utc()).unwrap()["state"],
            "ready"
        );
        assert_eq!(
            db.get_sim_detail(account.id).unwrap().account.current_cash,
            balances.current_cash
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
