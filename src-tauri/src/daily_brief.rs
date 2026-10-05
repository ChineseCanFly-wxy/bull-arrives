//! 本地盘前/盘后简报；只使用已保存的资讯和提醒，不自动调用 AI。

use crate::db::Database;
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use serde_json::{json, Value};
use std::sync::Arc;

const PREOPEN_MINUTE: u32 = 9 * 60 + 10;
const POSTCLOSE_MINUTE: u32 = 15 * 60 + 15;

fn china() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8")
}

fn at(date: NaiveDate, minute: u32) -> DateTime<FixedOffset> {
    china().from_local_datetime(&date.and_hms_opt(minute / 60, minute % 60, 0).expect("valid minute"))
        .single().expect("fixed offset has no DST")
}

fn previous_trading_day(now: DateTime<Utc>, today: NaiveDate) -> Result<NaiveDate, String> {
    let mut date = today;
    for _ in 0..14 {
        date = date.pred_opt().ok_or("无法计算前一交易日")?;
        if crate::datasource::trading_calendar::is_trading_day_at(now, date)? {
            return Ok(date);
        }
    }
    Err("近 14 天没有可确认的前一交易日".into())
}

fn brief_window(now: DateTime<Utc>, stage: &str) -> Result<(String, String), String> {
    let today = now.with_timezone(&china()).date_naive();
    let (start, end) = match stage {
        "preopen" => (at(previous_trading_day(now, today)?, POSTCLOSE_MINUTE), at(today, PREOPEN_MINUTE)),
        "postclose" => (at(today, PREOPEN_MINUTE), at(today, POSTCLOSE_MINUTE)),
        _ => return Err("未知简报时段".into()),
    };
    Ok((start.with_timezone(&Utc).to_rfc3339(), end.with_timezone(&Utc).to_rfc3339()))
}

fn compact_news(item: &Value) -> Value {
    let title = item["original_title"].as_str().or_else(|| item["title"].as_str()).unwrap_or("资讯");
    let excerpt = item["original_body"].as_str().filter(|body| !body.is_empty()).unwrap_or(title);
    json!({
        "signal_id": item["signal_id"],
        "title": title,
        "excerpt": excerpt.chars().take(140).collect::<String>(),
        "source": item["news_source"],
        "tag": item["signal_tag"],
        "watchlist_match": item["watchlist_match"],
        "received_at": item["received_at"],
    })
}

fn build_brief(db: &Database, now: DateTime<Utc>, stage: &str) -> Result<Option<Value>, String> {
    let day = now.with_timezone(&china()).format("%Y-%m-%d").to_string();
    if let Some(saved) = db.daily_brief(&day, stage)? { return Ok(Some(saved)); }
    let (start, end) = brief_window(now, stage)?;
    let news = db.brief_news_between(&start, &end)?;
    let alerts = if stage == "postclose" { db.brief_alerts_between(&start, &end)?.into_iter().filter(|item|matches!(item["signal_kind"].as_str(),Some("price"|"risk"))).collect::<Vec<_>>() } else { Vec::new() };
    if news.is_empty() && alerts.is_empty() { return Ok(None); }
    let major = news.iter().filter(|item| item["severity"] == "high").take(5).map(compact_news).collect::<Vec<_>>();
    let watchlist = news.iter().filter(|item| item["severity"] != "high" && item["watchlist_match"] == true)
        .take(5).map(compact_news).collect::<Vec<_>>();
    let other = news.iter().filter(|item| item["severity"] != "high" && item["watchlist_match"] != true)
        .take(3).map(compact_news).collect::<Vec<_>>();
    let alert_count = alerts.len();
    let alerts = alerts.iter().take(10).map(|item| json!({
        "title": item["title"], "body": item["body"], "received_at": item["received_at"], "kind": item["signal_kind"]
    })).collect::<Vec<_>>();
    let brief = json!({
        "day": day, "stage": stage, "generated_at": now.to_rfc3339(),
        "window_start": start, "window_end": end, "news_count": news.len(), "alert_count": alert_count,
        "major": major, "watchlist": watchlist, "other": other, "alerts": alerts,
    });
    db.save_daily_brief(&day, stage, &brief)?;
    db.daily_brief(&day, stage)
}

pub fn ensure_due_brief(db: &Database, now: DateTime<Utc>) -> Result<(), String> {
    let local = now.with_timezone(&china());
    if !crate::datasource::trading_calendar::is_trading_day_at(now, local.date_naive())? { return Ok(()); }
    let minute = local.hour() * 60 + local.minute();
    let stage = if minute >= POSTCLOSE_MINUTE { "postclose" }
        else if minute >= PREOPEN_MINUTE { "preopen" }
        else { return Ok(()); };
    build_brief(db, now, stage)?;
    Ok(())
}

#[tauri::command]
pub fn get_daily_briefs(db: tauri::State<'_, Arc<Database>>) -> Result<Vec<Value>, String> {
    if let Err(error) = ensure_due_brief(&db, Utc::now()) {
        log::warn!("[news] 每日简报生成失败：{error}");
    }
    db.daily_briefs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn brief_uses_saved_news_and_alerts_once() {
        let dir = std::env::temp_dir().join(format!("bull-daily-brief-{}-{}", std::process::id(), Utc::now().timestamp_micros()));
        let db = Database::open(dir.clone()).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 28, 7, 16, 0).single().unwrap();
        let recorded = now - chrono::Duration::minutes(10);
        let news = json!({"signal_id":"news:major","original_title":"甲公司收到立案告知书","original_body":"公告称收到立案告知书","signal_kind":"news","severity":"high","watchlist_match":true,"received_at":recorded.timestamp_millis()});
        db.archive_news(&news).unwrap();
        rusqlite::Connection::open(dir.join("bull-arrives.db")).unwrap()
            .execute("UPDATE news_archive SET received_at=?1 WHERE id='news:major'", [recorded.to_rfc3339()]).unwrap();
        let alert = json!({"id":"alert:1","title":"甲公司价格提醒","body":"价格触发","signal_kind":"price","received_at":recorded.timestamp_millis()});
        db.archive_brief_alert(&alert).unwrap();
        db.archive_brief_alert(&json!({"id":"research:1","title":"模型观察","body":"研究信号","signal_kind":"research","received_at":recorded.timestamp_millis()})).unwrap();
        // Even a stale client/database writing the removed keys must not block briefs.
        db.set_setting("quote_schedule_enabled", "1").unwrap();
        db.set_setting("quote_schedule", "invalid legacy JSON").unwrap();
        let holiday = Utc.with_ymd_and_hms(2026, 10, 1, 7, 16, 0).single().unwrap();
        ensure_due_brief(&db, holiday).unwrap();
        assert!(db.daily_briefs().unwrap().is_empty());
        ensure_due_brief(&db, now).unwrap();
        let brief = db.daily_brief("2026-09-28", "postclose").unwrap().unwrap();
        assert_eq!(brief["news_count"], 1);
        assert_eq!(brief["alert_count"], 1);
        assert_eq!(brief["major"][0]["title"], "甲公司收到立案告知书");
        assert_eq!(brief["alerts"][0]["title"], "甲公司价格提醒");
        db.archive_news(&json!({"signal_id":"news:later"})).unwrap();
        assert_eq!(build_brief(&db, now, "postclose").unwrap().unwrap(), brief);
        drop(db);
        let reopened = Database::open(dir.clone()).unwrap();
        assert_eq!(reopened.daily_briefs().unwrap()[0], brief);
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preopen_window_starts_after_previous_trading_close() {
        let now = Utc.with_ymd_and_hms(2026, 9, 28, 1, 12, 0).single().unwrap();
        let (start, end) = brief_window(now, "preopen").unwrap();
        assert_eq!(start, "2026-09-24T07:15:00+00:00"); // 中秋休市跨周末
        assert_eq!(end, "2026-09-28T01:10:00+00:00");
    }
}
