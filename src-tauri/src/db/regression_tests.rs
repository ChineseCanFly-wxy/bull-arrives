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
        ("visual_style", "modern"),
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
