use super::*;
use std::time::Instant;

/// Opt-in, offline timing harness. Run with `cargo test --lib cache_stage_timings -- --ignored --nocapture`.
#[test]
#[ignore]
fn cache_stage_timings() {
    let db = database();
    let quotes: Vec<crate::domain::Quote> = (0..500)
        .map(|i| sample_quote(&format!("sz{i:06}"), 12.34))
        .collect();
    let cache = crate::cache::QuoteCache::new(std::sync::Arc::new(db));
    for run in 0..5 {
        let total = Instant::now();
        let start = Instant::now();
        cache.update_quotes_memory(&quotes);
        let memory = start.elapsed();
        let start = Instant::now();
        let json: Vec<_> = quotes
            .iter()
            .map(|q| serde_json::to_string(q).unwrap())
            .collect();
        let serialize = start.elapsed();
        std::hint::black_box(json);
        let start = Instant::now();
        cache.persist_quotes(&quotes);
        let db_write = start.elapsed();
        let start = Instant::now();
        std::hint::black_box(cache.get_all_quotes());
        let memory_read = start.elapsed();
        println!("cache baseline run={run} rows={} memory_update_us={} serialize_us={} db_write_us={} memory_read_us={} total_us={}", quotes.len(), memory.as_micros(), serialize.as_micros(), db_write.as_micros(), memory_read.as_micros(), total.elapsed().as_micros());
    }
}

fn database() -> Database {
    let db = Database {
        conn: Mutex::new(Connection::open_in_memory().unwrap()),
    };
    db.migrate().unwrap();
    db.migrate_groups().unwrap();
    db.migrate_holdings().unwrap();
    db.migrate_price_alerts().unwrap();
    db
}

fn sample_quote(code: &str, price: f64) -> crate::domain::Quote {
    crate::domain::Quote {
        code: code.into(),
        market: "CN".into(),
        name: "测试".into(),
        price,
        prev_close: 12.0,
        open: 12.1,
        high: 12.5,
        low: 12.0,
        volume: 1000,
        turnover: 12000.0,
        turnover_rate: Some(1.2),
        change: price - 12.0,
        change_pct: (price / 12.0 - 1.0) * 100.0,
        timestamp: 1_790_300_000,
    }
}

#[test]
fn quote_cache_write_failure_rolls_back_entire_batch() {
    let db = database();
    db.cache_quotes(&[sample_quote("sz000001", 12.0)]).unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_second_quote BEFORE INSERT ON quote_cache
         WHEN NEW.code = 'sz000002' BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
        )
        .unwrap();
    assert!(db
        .cache_quotes(&[
            sample_quote("sz000001", 13.0),
            sample_quote("sz000002", 14.0),
        ])
        .is_err());
    let cached = db.get_cached_quotes().unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].price, 12.0);
}

#[test]
fn ordered_cache_writes_keep_latest_quote_and_restore_does_not_observe_alerts() {
    let db = std::sync::Arc::new(database());
    db.add_watch("sz000001", "CN", "测试").unwrap();
    db.upsert_price_alert(&PriceAlert {
        id: 0,
        code: "sz000001".into(),
        market: "CN".into(),
        alert_type: "change_pct".into(),
        threshold: 5.0,
        enabled: true,
        repeat_mode: "daily".into(),
        cooldown_minutes: 0,
        last_triggered_at: None,
        last_triggered_day: None,
        last_value: None,
        last_value_day: None,
    })
    .unwrap();
    let cache = crate::cache::QuoteCache::new(db.clone());
    cache.persist_quotes(&[sample_quote("sz000001", 12.0)]);
    cache.persist_quotes(&[sample_quote("sz000001", 13.0)]);
    cache.restore_from_db();
    assert_eq!(cache.get_all_quotes()[0].price, 13.0);
    let alerts = db.get_price_alerts("sz000001", "CN").unwrap();
    assert!(alerts[0].last_value.is_none());
    assert!(alerts[0].last_triggered_at.is_none());
}

#[test]
fn optional_simulation_cost_migration_preserves_research_and_filled_history() {
    let dir=std::env::temp_dir().join(format!("bull-arrives-cost-migration-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let db=Database::open(dir.clone()).unwrap();
    let input: crate::db::simulation::AccountInput=serde_json::from_value(serde_json::json!({
        "name":"ordinary","initial_cash":"1000000000","mode":"record","commission_bps":3,"min_commission":"50000","stamp_tax_bps":5,"transfer_fee_bps":1,"slippage_bps":10,"targets":[]
    })).unwrap();
    let ordinary=db.save_sim_account(&input).unwrap();
    let mut research_input=input.clone();research_input.name="frozen".into();let research=db.save_sim_account(&research_input).unwrap();
    let mut comparison_input=input.clone();comparison_input.name="frozen comparison".into();let comparison=db.save_sim_account(&comparison_input).unwrap();
    {
        let conn=db.conn.lock().unwrap();
        conn.execute("INSERT INTO strategy_cards(id,name,source,created_at,updated_at) VALUES('cost-test','cost-test','test','now','now')",[]).unwrap();
        conn.execute("INSERT INTO strategy_versions(id,card_id,version,definition_json,engine_revision,status,created_at,updated_at) VALUES(900001,'cost-test',1,'{}','test','candidate','now','now')",[]).unwrap();
        conn.execute("INSERT INTO research_experiments(id,version_id,name,hypothesis,rule,filter_json,config_json,account_id,created_at,source) VALUES(900001,900001,'frozen','test','test','{}','{}',?1,'now','test')",[research.id]).unwrap();
        conn.execute("INSERT INTO research_daily_comparison(experiment_id,account_id) VALUES(900001,?1)",[comparison.id]).unwrap();
        conn.execute("INSERT INTO sim_orders(account_id,idempotency_key,symbol,side,quantity,signal_date,source,limit_bps,status,price,gross,fee,cash_delta,created_at,filled_at) VALUES(?1,'historic','sh600001','buy',100,'20260918','manual',1000,'filled',100100,10010000,50101,-10060101,'now','now')",[ordinary.id]).unwrap();
        conn.execute("DELETE FROM settings WHERE key='simulation_zero_optional_costs_v1'",[]).unwrap();
    }
    let before=db.get_sim_detail(ordinary.id).unwrap();
    db.migrate_simulation_optional_costs().unwrap();
    let after=db.get_sim_detail(ordinary.id).unwrap();
    assert_eq!((after.account.transfer_fee_bps,after.account.slippage_bps),(0,0));
    assert_eq!(after.account.current_cash,before.account.current_cash);
    assert_eq!(serde_json::to_value(&after.orders).unwrap(),serde_json::to_value(&before.orders).unwrap(),"Never rewrite filled price, cash delta or historical fees");
    for id in [research.id,comparison.id] {let account=db.get_sim_detail(id).unwrap().account;assert_eq!((account.transfer_fee_bps,account.slippage_bps),(1,10),"Frozen research retains its original cost basis");}
    db.migrate_simulation_optional_costs().unwrap();assert_eq!(db.get_sim_detail(ordinary.id).unwrap().orders[0].fee.as_deref(),Some("50101"));
    drop(db);assert!(dir.file_name().unwrap().to_string_lossy().starts_with("bull-arrives-cost-migration-"));std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn research_notification_migration_preserves_mute_and_then_is_independent() {
    let db = database();
    db.init_defaults().unwrap();
    db.set_setting("alerts_enabled", "0").unwrap();
    db.conn.lock().unwrap().execute("DELETE FROM settings WHERE key='research_notifications_enabled'", []).unwrap();
    assert!(!db.research_notifications_enabled().unwrap(), "Missing new setting respects old mute");
    db.init_defaults().unwrap();
    assert_eq!(db.get_setting("research_notifications_enabled").unwrap().as_deref(), Some("0"));
    db.set_setting("research_notifications_enabled", "1").unwrap();
    assert!(db.research_notifications_enabled().unwrap(), "Research can notify while market alerts are off");
    db.init_defaults().unwrap();
    assert!(db.research_notifications_enabled().unwrap(), "Subsequent startup preserves independent choice");
    db.set_setting("alerts_enabled", "1").unwrap();
    db.set_setting("research_notifications_enabled", "0").unwrap();
    assert!(!db.research_notifications_enabled().unwrap(), "Market alerts cannot unmute research");
}

#[test]
fn category_notification_migration_preserves_old_mute_and_independent_choices() {
    let db = database();
    db.set_setting("research_notifications_enabled", "0").unwrap();
    let keys = ["mainline_notifications_enabled", "model_trade_notifications_enabled", "model_condition_notifications_enabled", "intraday_notifications_enabled"];
    for key in keys { db.conn.lock().unwrap().execute("DELETE FROM settings WHERE key=?1", [key]).unwrap(); }
    db.init_defaults().unwrap();
    for key in keys { assert_eq!(db.get_setting(key).unwrap().as_deref(), Some("0")); }
    db.set_setting("mainline_notifications_enabled", "1").unwrap();
    db.set_setting("model_trade_notifications_enabled", "1").unwrap();
    db.init_defaults().unwrap();
    assert_eq!(db.get_setting("mainline_notifications_enabled").unwrap().as_deref(), Some("1"));
    assert_eq!(db.get_setting("model_trade_notifications_enabled").unwrap().as_deref(), Some("1"));
    assert_eq!(db.get_setting("model_condition_notifications_enabled").unwrap().as_deref(), Some("0"));
    assert_eq!(db.get_setting("research_notifications_enabled").unwrap().as_deref(), Some("0"));
}
#[test]
fn settings_survive_reopening_and_defaults() {
    let dir = std::env::temp_dir().join(format!(
        "bull-arrives-settings-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let values = [
        ("ticker_opacity", "57"),
        ("ticker_single_color", "1"),
        ("ticker_text_color", "#336699"),
        ("theme", "dark"),
        ("visual_style", "elegant"),
        ("ticker_display_mode", "fixed"),
        ("ticker_page_size", "8"),
        ("refresh_interval", "12"),
    ];
    {
        let db = Database::open(dir.clone()).unwrap();
        db.init_defaults().unwrap();
        assert_eq!(
            db.get_setting("visual_style").unwrap().as_deref(),
            Some("classic")
        );
        for (key, value) in values {
            db.set_setting(key, value).unwrap();
        }
    }
    {
        let db = Database::open(dir.clone()).unwrap();
        db.init_defaults().unwrap();
        for (key, value) in values {
            assert_eq!(db.get_setting(key).unwrap().as_deref(), Some(value));
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_group_add_is_atomic() {
    let db = database();
    assert!(db
        .add_watch_to_group("sz000001", "CN", "测试", 999)
        .is_err());
    assert!(db.get_watchlist().unwrap().is_empty());
    let id = db.create_watch_group("观察").unwrap();
    db.add_watch_to_group("sz000001", "CN", "测试", id).unwrap();
    db.select_watch_group(id).unwrap();
    let snapshot = db.get_group_snapshot().unwrap();
    assert_eq!(snapshot.active_group_id, id);
    assert_eq!(snapshot.items.len(), 1);
}

#[test]
fn changed_rule_cannot_receive_old_evaluation_and_global_delete_cleans_alerts() {
    let db = database();
    db.add_watch("sz000001", "CN", "测试").unwrap();
    let mut rule = PriceAlert {
        id: 0,
        code: "sz000001".into(),
        market: "CN".into(),
        alert_type: "change_pct".into(),
        threshold: 5.0,
        enabled: true,
        repeat_mode: "daily".into(),
        cooldown_minutes: 0,
        last_triggered_at: None,
        last_triggered_day: None,
        last_value: None,
        last_value_day: None,
    };
    db.upsert_price_alert(&rule).unwrap();
    let old = db
        .get_price_alerts(&rule.code, &rule.market)
        .unwrap()
        .remove(0);
    rule.threshold = 10.0;
    db.upsert_price_alert(&rule).unwrap();
    assert!(db
        .persist_price_alert_evaluation(
            &old,
            5.0,
            Some("2026-09-10"),
            Some("2026-09-10T10:00:00+08:00"),
            Some("2026-09-10")
        )
        .is_err());
    assert!(db.get_price_alerts(&rule.code, &rule.market).unwrap()[0]
        .last_triggered_at
        .is_none());
    db.remove_watch(&rule.code, &rule.market).unwrap();
    assert!(db.get_all_price_alerts().unwrap().is_empty());
}


#[test]
fn removed_quote_schedule_cannot_survive_default_migration() {
    let db = database();
    db.set_setting("quote_schedule_enabled", "1").unwrap();
    db.set_setting("quote_schedule", "invalid legacy JSON").unwrap();
    db.set_setting("refresh_interval", "12").unwrap();
    db.init_defaults().unwrap();
    assert!(db.get_setting("quote_schedule_enabled").unwrap().is_none());
    assert!(db.get_setting("quote_schedule").unwrap().is_none());
    assert_eq!(db.get_setting("refresh_interval").unwrap().as_deref(), Some("12"));
    db.init_defaults().unwrap();
    assert!(db.get_setting("quote_schedule_enabled").unwrap().is_none());
}


#[test]
fn retired_trading_theme_migrates_without_changing_saved_model_history() {
    let db = database();
    db.migrate_model_research().unwrap();
    let raw = include_str!("../../../research/research-center-runner/checks/verified-rank-forward-c0ae27eb-45f7-4dd6-b678-1fe478201281.json");
    let id = db.save_model_run(raw, None).unwrap();
    db.set_setting("visual_style", "trading").unwrap();
    db.set_setting("theme", "dark").unwrap();
    db.init_defaults().unwrap();
    assert_eq!(db.get_setting("visual_style").unwrap().as_deref(), Some("modern"));
    assert_eq!(db.get_setting("theme").unwrap().as_deref(), Some("dark"));
    assert_eq!(db.model_run_bundle(id).unwrap(), raw);
    db.init_defaults().unwrap();
    assert_eq!(db.model_run_bundle(id).unwrap(), raw);
    db.set_setting("visual_style", "classic").unwrap();
    db.init_defaults().unwrap();
    assert_eq!(db.get_setting("visual_style").unwrap().as_deref(), Some("classic"));
}
