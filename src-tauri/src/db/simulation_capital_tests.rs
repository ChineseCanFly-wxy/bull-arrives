//! Ledger/capital and account-ownership regressions. Every database is isolated in memory or a new temp directory.
use super::*;
use crate::db::model_follow::{FollowBinding, FollowOrderMeta};
use rusqlite::Connection;

fn database() -> Database {
    let db = Database { conn: std::sync::Mutex::new(Connection::open_in_memory().unwrap()) };
    db.migrate_simulation().unwrap();
    db.migrate_simulation_live().unwrap();
    db.migrate_strategies().unwrap();
    db.migrate_research_loop().unwrap();
    db.migrate_model_research().unwrap();
    db.migrate_model_follow().unwrap();
    db
}
fn account_input() -> AccountInput {
    AccountInput { id: None, name: "capital regression".into(), initial_cash: "1000000000".into(), mode: "auto".into(), auto_enabled: true, manual_source_enabled: true, rule_source_enabled: true, ai_source_enabled: false, commission_bps: 3, min_commission: "50000".into(), stamp_tax_bps: 5, transfer_fee_bps: 0, slippage_bps: 0, targets: vec![] }
}
fn order_input(account: i64, key: &str) -> OrderInput {
    OrderInput { account_id: account, idempotency_key: key.into(), symbol: "sh600000".into(), name: "浦发银行".into(), side: "buy".into(), quantity: 100, signal_date: "2026-09-18".into(), source: "manual".into(), rule: None, stop_bps: 0, take_bps: 0, limit_bps: 1000, max_hold_days: 0, ai_generated: false }
}
fn raw_bar() -> RawBar {
    RawBar { date: "2026-09-21".into(), open: "100000".into(), high: "102000".into(), low: "98000".into(), close: "100000".into(), prev_close: "100000".into(), volume: 1_000_000 }
}
fn adjust(db: &Database, id: i64, cny: f64) -> Result<SimAccount, String> {
    db.adjust_sim_capital(&CapitalAdjustmentInput { account_id: id, initial_cash: cny })
}
fn model_account(db: &Database) -> FollowBinding {
    let mut input = account_input(); input.manual_source_enabled = false;
    let account = db.save_sim_account(&input).unwrap();
    let source = {
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO model_research_runs(identity,bundle_json,enabled,created_at,updated_at) VALUES('capital-fixture','{}',0,'2026-09-30','2026-09-30')", []).unwrap(); conn.last_insert_rowid()
    };
    let binding = FollowBinding { account_id: account.id, source_run_id: source, model_id: "breadth22_h20".into(), model_name: "frozen breadth".into(), max_positions: 2, allocation_policy: "baseline8".into(), execution_policy: "baseline".into(), enabled: true, auto_execute: true, initial_cash_cny: 100000., as_of: "2026-09-30".into(), source_sha256: "fixture".into(), context_json: "{}".into(), state: "listening".into(), message: "unchanged".into(), allocation_date: Some("2026-10-09".into()), allocation_equity: Some(1_050_000_000), valuation_blocked: false, valuation_block_reason: None, candidate_cursor: 7 };
    db.save_follow_binding(&binding).unwrap(); binding
}
fn research_experiment(db: &Database) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.execute("INSERT INTO strategy_cards(id,name,source,created_at,updated_at) VALUES('capital','capital','fixture','2026-09-30','2026-09-30')", []).unwrap();
    conn.execute("INSERT INTO strategy_versions(card_id,version,definition_json,engine_revision,status,created_at,updated_at) VALUES('capital',1,'{}','fixture','active','2026-09-30','2026-09-30')", []).unwrap();
    let version = conn.last_insert_rowid();
    conn.execute("INSERT INTO research_experiments(version_id,name,hypothesis,rule,filter_json,config_json,state,created_at,source) VALUES(?1,'capital','fixture','trend_follow','{}','{}','observing','2026-09-30','fixture')", [version]).unwrap(); conn.last_insert_rowid()
}
fn fixture_pending(db: &Database, account: i64, follow: bool) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.execute("INSERT INTO sim_orders(account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,limit_bps,status,created_at) VALUES(?1,'pending-reserved','sh600000','浦发银行','buy',100,'2026-10-09','rule',1000,'pending','2026-10-09T01:30:00Z')", [account]).unwrap();
    let id = conn.last_insert_rowid();
    conn.execute("INSERT INTO sim_live_orders(order_id,limit_price,submitted_at) VALUES(?1,100000,0)", [id]).unwrap();
    if follow {
        let meta = FollowOrderMeta { order_id: id, limit_price: 100000, estimated_fee: 50000, factor: 1., atr: 0.2, quote_at: 0, valid_until: i64::MAX, context_as_of: "2026-09-30".into(), context_sha256: "fixture".into(), allocation_policy: "baseline8".into(), execution_policy: "baseline".into(), automatic_submission: true };
        conn.execute("INSERT INTO model_follow_orders(order_id,meta_json) VALUES(?1,?2)", params![id, serde_json::to_string(&meta).unwrap()]).unwrap();
    }
    id
}

#[test]
fn capital_input_rejects_non_cent_values_ranges_and_spoofed_fields() {
    assert_eq!(capital_cny_scaled(1000.01).unwrap(), 10_000_100);
    assert_eq!(capital_cny_scaled(100_000_000.).unwrap(), 1_000_000_000_000);
    for value in [0., 999.99, 100_000_000.01, 1000.001, f64::NAN, f64::INFINITY] { assert!(capital_cny_scaled(value).is_err()); }
    assert!(serde_json::from_value::<CapitalAdjustmentInput>(serde_json::json!({"accountId":1,"initialCash":1000,"managedBy":"manual"})).is_err());
}

#[test]
fn capital_changes_keep_fills_positions_orders_and_do_not_create_profit() {
    let db = database(); let account = db.save_sim_account(&account_input()).unwrap();
    let order = db.submit_sim_order(&order_input(account.id, "buy")).unwrap();
    assert_eq!(db.match_sim_order_raw(order.id, &raw_bar()).unwrap().status, "filled");
    db.mark_sim_equity(account.id, "2026-09-21", 10_000_000, None).unwrap();
    let before = db.get_sim_detail(account.id).unwrap();
    let profit = before.metrics.equity.parse::<i64>().unwrap() - 1_000_000_000;
    let updated = adjust(&db, account.id, 150000.01).unwrap();
    assert_eq!(updated.initial_cash, "1500000100");
    assert_eq!(updated.current_cash.parse::<i64>().unwrap(), before.account.current_cash.parse::<i64>().unwrap() + 500_000_100);
    let after = db.get_sim_detail(account.id).unwrap();
    assert_eq!(serde_json::to_value(&after.positions).unwrap(), serde_json::to_value(&before.positions).unwrap());
    assert_eq!(serde_json::to_value(&after.orders).unwrap(), serde_json::to_value(&before.orders).unwrap());
    assert_eq!(after.metrics.equity.parse::<i64>().unwrap() - 1_500_000_100, profit);
    assert_eq!(after.capital_adjustments.len(), 1); assert!(after.performance_note.contains("最新初始资金基准"));
    assert_eq!(db.conn.lock().unwrap().query_row("SELECT equity FROM sim_equity_daily WHERE account_id=?1", [account.id], |r| r.get::<_,i64>(0)).unwrap(), before.metrics.equity.parse::<i64>().unwrap());
    adjust(&db, account.id, 150000.01).unwrap(); assert_eq!(db.get_sim_detail(account.id).unwrap().capital_adjustments.len(), 1);
    adjust(&db, account.id, 80000.).unwrap();
    let final_detail = db.get_sim_detail(account.id).unwrap(); assert_eq!(final_detail.metrics.equity.parse::<i64>().unwrap() - 800_000_000, profit);
    db.mark_sim_equity(account.id, "2026-09-22", 10_000_000, None).unwrap();
    assert_eq!(db.get_sim_detail(account.id).unwrap().metrics.equity, final_detail.metrics.equity);
}

#[test]
fn ordinary_save_uses_the_same_atomic_capital_delta() {
    let db = database(); let account = db.save_sim_account(&account_input()).unwrap();
    let mut input = account_input(); input.id = Some(account.id); input.initial_cash = "1250000000".into();
    let result = db.save_sim_account(&input).unwrap(); assert_eq!(result.current_cash, "1250000000");
    input.initial_cash = "1250000001".into(); assert!(db.save_sim_account(&input).is_err());
    assert_eq!(db.get_sim_detail(account.id).unwrap().capital_adjustments.len(), 1);
}

#[test]
fn withdrawals_cannot_consume_holdings_or_unknown_daily_buy_cash() {
    let db = database(); let account = db.save_sim_account(&account_input()).unwrap();
    let mut order = order_input(account.id, "large"); order.quantity = 9000;
    let fill = db.submit_sim_order(&order).unwrap(); assert_eq!(db.match_sim_order_raw(fill.id, &raw_bar()).unwrap().status, "filled");
    let before = db.get_sim_detail(account.id).unwrap(); assert!(adjust(&db, account.id, 1000.).is_err());
    assert_eq!(serde_json::to_value(&before).unwrap(), serde_json::to_value(db.get_sim_detail(account.id).unwrap()).unwrap());
    let other = db.save_sim_account(&account_input()).unwrap(); db.submit_sim_order(&order_input(other.id, "unknown")).unwrap();
    assert!(adjust(&db, other.id, 50000.).unwrap_err().contains("未确定成交价格"));
    adjust(&db, other.id, 120000.).unwrap();
}

#[test]
fn reserved_pending_buys_are_protected_for_manual_and_model_accounts() {
    for follow in [false, true] {
        let db = database(); let id = if follow { model_account(&db).account_id } else { db.save_sim_account(&account_input()).unwrap().id };
        let order = fixture_pending(&db, id, follow);
        let before = db.get_sim_detail(id).unwrap(); assert!(adjust(&db, id, 1000.).unwrap_err().contains("未成交买单"));
        assert_eq!(db.get_sim_detail(id).unwrap().account.current_cash, before.account.current_cash);
        assert!(db.get_sim_detail(id).unwrap().capital_adjustments.is_empty());
        let result = adjust(&db, id, 1005.).unwrap(); assert_eq!(result.current_cash, "10050000");
        assert_eq!(db.get_pending_sim_orders(id).unwrap()[0].id, order);
    }
}

#[test]
fn model_capital_moves_binding_and_cached_equity_by_exact_ledger_delta() {
    let db = database(); let before = model_account(&db);
    let result = adjust(&db, before.account_id, 110000.01).unwrap(); assert_eq!(result.managed_by, "model_follow");
    let after = db.follow_binding(before.account_id).unwrap();
    assert_eq!(after.initial_cash_cny, 110000.01); assert_eq!(after.allocation_equity, Some(1_150_000_100));
    assert_eq!(after.candidate_cursor, before.candidate_cursor); assert_eq!(after.allocation_policy, before.allocation_policy);
    assert_eq!(after.execution_policy, before.execution_policy); assert_eq!(after.context_json, before.context_json);
    assert_eq!(after.enabled, before.enabled);
    // A stale in-memory binding cannot put the old cash or allocation snapshot back.
    db.save_follow_binding(&before).unwrap(); let saved = db.follow_binding(before.account_id).unwrap();
    assert_eq!(saved.initial_cash_cny, after.initial_cash_cny); assert_eq!(saved.allocation_equity, after.allocation_equity);
}

#[test]
fn invalid_model_binding_rolls_back_cash_and_audit_together() {
    let db = database(); let binding = model_account(&db);
    db.conn.lock().unwrap().execute("UPDATE model_follow_accounts SET binding_json='invalid' WHERE account_id=?1", [binding.account_id]).unwrap();
    assert!(adjust(&db, binding.account_id, 120000.).is_err());
    let conn = db.conn.lock().unwrap();
    assert_eq!(conn.query_row("SELECT cash FROM sim_accounts WHERE id=?1", [binding.account_id], |r| r.get::<_,i64>(0)).unwrap(), 1_000_000_000);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM sim_capital_adjustments", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
}

#[test]
fn model_accounts_block_all_forged_generic_sources_and_manual_configuration() {
    let db = database(); let binding = model_account(&db); let id = binding.account_id;
    for source in ["manual", "rule", "risk", "ai"] { let mut input = order_input(id, source); input.source = source.into(); assert!(db.submit_sim_order(&input).unwrap_err().contains("专用账户")); }
    let mut input = account_input(); input.id = Some(id); assert!(db.save_sim_account(&input).is_err()); assert!(db.delete_sim_account(id).is_err());
    let order = fixture_pending(&db, id, true);
    assert!(db.confirm_sim_order(order).is_err()); assert!(db.match_sim_order_raw(order, &raw_bar()).is_err());
    assert_eq!(db.get_sim_detail(id).unwrap().account.managed_by, "model_follow");
    assert_eq!(db.get_pending_sim_orders(id).unwrap().len(), 1); assert!(db.get_sim_detail(id).unwrap().positions.is_empty());
    let manual = db.save_sim_account(&account_input()).unwrap(); assert_eq!(db.list_auto_sim_account_ids().unwrap(), vec![manual.id]);
}

#[test]
fn account_slots_cannot_attach_other_strategies_to_model_accounts() {
    let db = database(); let id = model_account(&db).account_id;
    assert!(db.enable_live_account(id, &[]).is_err());
    let plan = crate::db::simulation_live::LivePlan { symbol: "sh600000".into(), buy_low: 100000, buy_high: 100000, stop: 90000, take: 110000, limit_bps: 1000, basis_date: "2026-09-30".into(), reference_close: 100000, position_pct: 8. };
    assert!(db.set_live_manual_plan(id, &plan).is_err());
    let experiment = research_experiment(&db);
    let conn = db.conn.lock().unwrap();
    assert!(conn.execute("INSERT INTO sim_targets(account_id,symbol,rule) VALUES(?1,'sh600000','trend_follow')", [id]).is_err());
    assert!(conn.execute("UPDATE research_experiments SET account_id=?1 WHERE id=?2", params![id, experiment]).is_err());
    assert!(conn.execute("INSERT INTO research_daily_comparison(experiment_id,account_id) VALUES(?1,?2)", params![experiment,id]).is_err());
    assert!(conn.execute("UPDATE sim_accounts SET manual_source_enabled=1 WHERE id=?1", [id]).is_err());
}

#[test]
fn frozen_research_and_comparison_accounts_remain_readonly() {
    let db = database(); let experiment = research_experiment(&db);
    let original = db.save_sim_account(&account_input()).unwrap(); let comparison = db.save_sim_account(&account_input()).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("UPDATE research_experiments SET account_id=?1 WHERE id=?2", params![original.id,experiment]).unwrap();
        conn.execute("INSERT INTO research_daily_comparison(experiment_id,account_id) VALUES(?1,?2)", params![experiment,comparison.id]).unwrap();
    }
    for account in [original, comparison] {
        let before = db.get_sim_detail(account.id).unwrap(); assert_eq!(before.account.managed_by, "research");
        assert!(adjust(&db, account.id, 120000.).unwrap_err().contains("冻结"));
        let mut input = account_input(); input.id = Some(account.id); assert!(db.save_sim_account(&input).is_err());
        assert_eq!(serde_json::to_value(before).unwrap(), serde_json::to_value(db.get_sim_detail(account.id).unwrap()).unwrap());
    }
}

#[test]
fn capital_changes_and_actual_fills_are_serialized_without_lost_cash() {
    let db = std::sync::Arc::new(database()); let account = db.save_sim_account(&account_input()).unwrap();
    let order = db.submit_sim_order(&order_input(account.id, "concurrent-fill")).unwrap();
    let start = std::sync::Arc::new(std::sync::Barrier::new(2));
    let a = { let db=db.clone(); let start=start.clone(); std::thread::spawn(move || { start.wait(); adjust(&db, account.id, 120000.01).unwrap(); }) };
    let b = { let db=db.clone(); let start=start.clone(); std::thread::spawn(move || { start.wait(); db.match_sim_order_raw(order.id, &raw_bar()).unwrap(); }) };
    a.join().unwrap(); b.join().unwrap();
    let detail = db.get_sim_detail(account.id).unwrap(); let fill = &detail.orders[0]; assert_eq!(fill.status, "filled");
    let paid = fill.gross.as_ref().unwrap().parse::<i64>().unwrap() + fill.fee.as_ref().unwrap().parse::<i64>().unwrap();
    assert_eq!(detail.account.current_cash.parse::<i64>().unwrap(), 1_200_000_100 - paid);
    assert_eq!(detail.positions.len(), 1); assert_eq!(detail.capital_adjustments.len(), 1);
}

#[test]
fn capital_audit_and_snapshot_markers_survive_database_reopen() {
    let root = std::env::temp_dir().join(format!("bull-capital-{}", uuid::Uuid::new_v4()));
    { let db=Database::open(root.clone()).unwrap(); let account=db.save_sim_account(&account_input()).unwrap(); db.mark_sim_equity(account.id,"2026-09-30",0,None).unwrap(); adjust(&db,account.id,125000.).unwrap(); }
    { let db=Database::open(root.clone()).unwrap(); let detail=db.get_sim_detail(1).unwrap(); assert_eq!(detail.capital_adjustments.len(),1); assert_eq!(detail.metrics.equity,"1250000000"); assert_eq!(detail.metrics.total_return_bps,0); assert_eq!(detail.metrics.max_drawdown_bps,0); }
    assert_eq!(root.parent(), Some(std::env::temp_dir().as_path())); std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn manual_run_cannot_drive_a_model_account_or_create_a_missing_account() {
    let db=database(); let id=model_account(&db).account_id;
    let before=serde_json::to_value(db.get_sim_detail(id).unwrap()).unwrap();
    assert!(crate::commands::simulation::run_account(&db,id).await.unwrap_err().contains("专用账户"));
    assert_eq!(serde_json::to_value(db.get_sim_detail(id).unwrap()).unwrap(),before);
    assert!(adjust(&db,999,120000.).is_err()); assert_eq!(db.list_sim_accounts().unwrap().len(),1);
}

#[test]
fn partial_simulation_migrations_do_not_install_guards_against_absent_model_tables() {
    let db=Database{conn:std::sync::Mutex::new(Connection::open_in_memory().unwrap())};
    db.migrate_simulation().unwrap();
    let account=db.save_sim_account(&account_input()).unwrap();
    assert_eq!(db.list_auto_sim_account_ids().unwrap(),vec![account.id]);
    assert_eq!(db.submit_sim_order(&order_input(account.id,"before-model-migration")).unwrap().status,"pending");
    adjust(&db,account.id,120000.).unwrap();
    db.mark_sim_equity(account.id,"2026-09-30",0,None).unwrap();
    assert_eq!(db.get_sim_detail(account.id).unwrap().account.managed_by,"manual");
}
