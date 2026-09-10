use super::*;

fn database() -> Database {
    let db = Database { conn: Mutex::new(Connection::open_in_memory().unwrap()) };
    db.migrate().unwrap();
    db.migrate_groups().unwrap();
    db.migrate_holdings().unwrap();
    db.migrate_price_alerts().unwrap();
    db
}

#[test]
fn invalid_group_add_is_atomic() {
    let db = database();
    assert!(db.add_watch_to_group("sz000001","CN","测试",999).is_err());
    assert!(db.get_watchlist().unwrap().is_empty());
    let id = db.create_watch_group("观察").unwrap();
    db.add_watch_to_group("sz000001","CN","测试",id).unwrap();
    db.select_watch_group(id).unwrap();
    let snapshot = db.get_group_snapshot().unwrap();
    assert_eq!(snapshot.active_group_id,id);
    assert_eq!(snapshot.items.len(),1);
}

#[test]
fn changed_rule_cannot_receive_old_evaluation_and_global_delete_cleans_alerts() {
    let db = database();
    db.add_watch("sz000001","CN","测试").unwrap();
    let mut rule = PriceAlert { id:0,code:"sz000001".into(),market:"CN".into(),alert_type:"change_pct".into(),threshold:5.0,enabled:true,repeat_mode:"daily".into(),cooldown_minutes:0,last_triggered_at:None,last_triggered_day:None,last_value:None,last_value_day:None };
    db.upsert_price_alert(&rule).unwrap();
    let old = db.get_price_alerts(&rule.code,&rule.market).unwrap().remove(0);
    rule.threshold = 10.0;
    db.upsert_price_alert(&rule).unwrap();
    assert!(db.persist_price_alert_evaluation(&old,5.0,Some("2026-09-10"),Some("2026-09-10T10:00:00+08:00"),Some("2026-09-10")).is_err());
    assert!(db.get_price_alerts(&rule.code,&rule.market).unwrap()[0].last_triggered_at.is_none());
    db.remove_watch(&rule.code,&rule.market).unwrap();
    assert!(db.get_all_price_alerts().unwrap().is_empty());
}
