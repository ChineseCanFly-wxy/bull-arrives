use super::Database;
use rusqlite::{params, Result as SqliteResult};
use serde_json::{json, Value};

fn timestamp(value: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    value.as_i64().and_then(chrono::DateTime::from_timestamp_millis).or_else(||
        value.as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d|d.with_timezone(&chrono::Utc)))
}

fn symbol_code(value: &str) -> Option<String> {
    let lower=value.trim().to_ascii_lowercase();
    let code=lower.strip_prefix("sh").or_else(||lower.strip_prefix("sz"))
        .map(|v|v.trim_start_matches('.')).unwrap_or(&lower);
    (code.len()==6 && code.bytes().all(|b|b.is_ascii_digit())
        && ["00","30","60","68"].iter().any(|prefix|code.starts_with(prefix))).then(||code.to_owned())
}

impl Database {
    pub fn migrate_news(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS news_seen (
                dedupe_key TEXT PRIMARY KEY,
                source     TEXT NOT NULL,
                seen_at    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_news_seen_at ON news_seen(seen_at);
            CREATE TABLE IF NOT EXISTS news_archive(id TEXT PRIMARY KEY,payload TEXT NOT NULL,received_at TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS daily_briefs(day TEXT NOT NULL,stage TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(day,stage));
            CREATE TABLE IF NOT EXISTS daily_brief_alerts(id TEXT PRIMARY KEY,received_at TEXT NOT NULL,payload TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS idx_daily_brief_alerts_at ON daily_brief_alerts(received_at);",
        )?;
        conn.execute("DELETE FROM news_archive WHERE id LIKE 'timeline:%'", [])?;
        // 此表只记录已移除的自动额度，不含资讯原文或手动解读。
        conn.execute("DROP TABLE IF EXISTS news_ai_auto_claim", [])?;
        Ok(())
    }

    /// 原子认领一条资讯；false 表示之前已经处理过，重启后仍然有效。
    pub fn claim_news(&self, key: &str, source: &str, seen_at: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let changed = conn.execute(
            "INSERT OR IGNORE INTO news_seen(dedupe_key, source, seen_at) VALUES (?1, ?2, ?3)",
            params![key, source, seen_at],
        )?;
        Ok(changed > 0)
    }

    pub fn purge_news_before(&self, cutoff: &str) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute("DELETE FROM news_seen WHERE seen_at < ?1", [cutoff])
    }

    pub fn archive_news(&self, payload: &serde_json::Value) -> Result<(), String> {
        let id = payload["signal_id"]
            .as_str()
            .or_else(|| payload["id"].as_str())
            .ok_or("资讯缺少标识")?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO news_archive(id,payload,received_at) VALUES(?1,?2,?3)",
            params![id, payload.to_string(), chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        let cutoff=(chrono::Utc::now()-chrono::Duration::days(30)).to_rfc3339();
        conn.execute("DELETE FROM news_archive WHERE datetime(received_at) < datetime(?1)",[cutoff]).map_err(|e|e.to_string())?;
        conn.execute("DELETE FROM news_archive WHERE id NOT IN (SELECT id FROM news_archive ORDER BY received_at DESC LIMIT 5000)",[]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn news_archive(&self) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT payload FROM news_archive ORDER BY received_at DESC LIMIT 500")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }

    /// 研究使用较大的原文窗口；普通资讯界面继续只显示最近 500 条。
    pub fn news_research_archive(&self) -> Result<Vec<Value>, String> {
        let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());
        let mut stmt=conn.prepare("SELECT payload,received_at FROM news_archive ORDER BY received_at DESC LIMIT 5000")
            .map_err(|e|e.to_string())?;
        let rows=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?;
        rows.map(|r| {
            let (raw,archived_at)=r.map_err(|e|e.to_string())?;
            let mut row:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            row["archived_at"]=archived_at.into();
            Ok(row)
        }).collect()
    }

    /// 原文关联只用于解释。行情截点之后的新资讯会被标出，不能回填为当时的模型输入。
    pub fn recent_research_news(&self, sector_names:&[String], symbols:&[String], names:&[String], as_of:&str) -> Result<Vec<Value>,String> {
        let now=chrono::Utc::now();
        let cutoff=now-chrono::Duration::days(7);
        let wanted:Vec<_>=symbols.iter().filter_map(|s|symbol_code(s)).collect();
        let as_of_day=chrono::NaiveDate::parse_from_str(as_of,"%Y-%m-%d")
            .or_else(|_|chrono::NaiveDate::parse_from_str(as_of,"%Y%m%d")).ok();
        let as_of_close=as_of_day.and_then(|d|d.and_hms_opt(7,0,0))
            .map(|d|d.and_utc()); // 北京时间 15:00。
        let local_today=now.with_timezone(&chrono::FixedOffset::east_opt(8*3600).unwrap()).date_naive();
        let rows=self.news_research_archive()?;
        let scanned=rows.len();
        let mut selected=Vec::new();
        for row in rows {
            let received=timestamp(&row["source_received_at"]).or_else(||timestamp(&row["received_at"]))
                .or_else(||timestamp(&row["archived_at"]));
            let Some(received)=received.filter(|t|*t>=cutoff && *t<=now) else {continue;};
            let published=timestamp(&row["published_at"]);
            if published.is_some_and(|t|t>now) {continue;}
            let published_day=row["published_date"].as_str().and_then(|s|chrono::NaiveDate::parse_from_str(s,"%Y-%m-%d").ok());
            if published_day.is_some_and(|d|d>local_today) {continue;}
            let title=row["original_title"].as_str().or(row["title"].as_str()).unwrap_or("");
            let body=row["original_body"].as_str().unwrap_or("");
            let text=format!("{title} {body}");
            let source_symbols_verified=row.get("source_symbols").is_some_and(Value::is_array);
            let source_symbols:Vec<_>=row.get("source_symbols").or_else(||row.get("symbols")).and_then(Value::as_array).into_iter().flatten()
                .filter_map(Value::as_str).filter_map(symbol_code).collect();
            let associated_symbols:Vec<_>=row["symbols"].as_array().into_iter().flatten()
                .filter_map(Value::as_str).filter_map(symbol_code).collect();
            let matched_symbols:Vec<_>=source_symbols.iter().filter(|s|wanted.contains(s)).cloned().collect();
            let name_inferred_symbols:Vec<_>=associated_symbols.iter().filter(|s|source_symbols_verified && source_symbols.is_empty() && wanted.contains(s)).cloned().collect();
            let matched_names:Vec<_>=names.iter().filter(|n|source_symbols.is_empty() && n.chars().count()>=2 && text.contains(n.as_str())).cloned().collect();
            let matched_sectors:Vec<_>=sector_names.iter().filter(|n|n.chars().count()>=2 && text.contains(n.as_str())).cloned().collect();
            let (priority,basis)=if !matched_symbols.is_empty(){(0,if source_symbols_verified {"structured_symbol"}else{"legacy_symbol_unverified"})}
                else if !matched_names.is_empty(){(1,"name")}
                else if !name_inferred_symbols.is_empty(){(1,"name_inferred_symbol")}
                else if !matched_sectors.is_empty(){(2,"theme")}else{continue;};
            let available=match (published,as_of_close,published_day,as_of_day) {
                (Some(p),Some(c),_,_)=>Some(p<=c),
                (None,_,Some(p),Some(d)) if p!=d=>Some(p<d),
                _=>None,
            };
            let index_only=row["source_index_only"].as_bool().unwrap_or(body.is_empty());
            selected.push((priority,received,json!({"id":row["signal_id"],"title":title,"body":body,
                "source":row["news_source"],"source_id":row["news_source_id"],"source_kind":row["news_source_kind"],
                "url":row["url"],"source_url":row["source_url"],"symbols":associated_symbols,
                "source_symbols":source_symbols,"source_symbols_verified":source_symbols_verified,"name_inferred_symbols":name_inferred_symbols,
                "published_at":row["published_at"],"published_date":row["published_date"],
                "publication_precision":row["publication_precision"],"published_at_source":row["published_at_source"],
                "received_at":received.to_rfc3339(),"archived_at":row["archived_at"],"match_basis":basis,
                "matched_symbols":matched_symbols,"matched_names":matched_names,"matched_sector_names":matched_sectors,
                "source_index_only":index_only,"source_coverage":row["source_coverage"],"market_as_of":as_of,
                "available_for_market_asof":available,"published_after_market_asof":available.map(|v|!v),
                "archive_rows_scanned":scanned,"archive_read_limit":5000,"coverage_complete":false,
                "coverage_note":"最近七天采集原文，归档最多五千条且保留三十天；列表轮询及标题匹配不能保证全部资讯覆盖。未知发布时间不能作为历史特征。"})));
        }
        selected.sort_by(|a,b|a.0.cmp(&b.0).then_with(||b.1.cmp(&a.1)));
        Ok(selected.into_iter().take(20).map(|(_,_,row)|row).collect())
    }

    pub fn brief_news_between(&self, start: &str, end: &str) -> Result<Vec<serde_json::Value>, String> {
        self.brief_rows("SELECT payload FROM news_archive WHERE received_at >= ?1 AND received_at < ?2 ORDER BY received_at DESC", start, end)
    }

    pub fn brief_alerts_between(&self, start: &str, end: &str) -> Result<Vec<serde_json::Value>, String> {
        self.brief_rows("SELECT payload FROM daily_brief_alerts WHERE received_at >= ?1 AND received_at < ?2 ORDER BY received_at DESC", start, end)
    }

    fn brief_rows(&self, query: &str, start: &str, end: &str) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![start, end], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
        rows.map(|row| serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())).collect()
    }

    pub fn archive_brief_alert(&self, payload: &serde_json::Value) -> Result<(), String> {
        let id = payload["id"].as_str().ok_or("提醒缺少 ID")?;
        let received_at = payload["received_at"].as_i64()
            .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
            .ok_or("提醒时间无效")?.to_rfc3339();
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("INSERT OR IGNORE INTO daily_brief_alerts(id,received_at,payload) VALUES(?1,?2,?3)", params![id, received_at, payload.to_string()]).map_err(|e| e.to_string())?;
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(14)).to_rfc3339();
        conn.execute("DELETE FROM daily_brief_alerts WHERE received_at < ?1", [cutoff]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn daily_brief(&self, day: &str, stage: &str) -> Result<Option<serde_json::Value>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT payload FROM daily_briefs WHERE day=?1 AND stage=?2").map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![day, stage]).map_err(|e| e.to_string())?;
        rows.next().map_err(|e| e.to_string())?
            .map(|row| row.get::<_, String>(0).map_err(|e| e.to_string()).and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string())))
            .transpose()
    }

    pub fn save_daily_brief(&self, day: &str, stage: &str, payload: &serde_json::Value) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("INSERT OR IGNORE INTO daily_briefs(day,stage,payload) VALUES(?1,?2,?3)", params![day, stage, payload.to_string()]).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM daily_briefs WHERE (day,stage) NOT IN (SELECT day,stage FROM daily_briefs ORDER BY day DESC, CASE stage WHEN 'postclose' THEN 0 ELSE 1 END LIMIT 60)", []).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn daily_briefs(&self) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT payload FROM daily_briefs ORDER BY day DESC, CASE stage WHEN 'postclose' THEN 0 ELSE 1 END LIMIT 60").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
        rows.map(|row| serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())).collect()
    }

    pub fn news_archive_item(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT payload FROM news_archive WHERE id=?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([id]).map_err(|e| e.to_string())?;
        rows.next().map_err(|e| e.to_string())?
            .map(|row| row.get::<_, String>(0).map_err(|e| e.to_string())
                .and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string())))
            .transpose()
    }

    pub fn update_news_analysis(&self, id: &str, payload: &serde_json::Value) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let changed = conn.execute("UPDATE news_archive SET payload=?1 WHERE id=?2", params![payload.to_string(), id])
            .map_err(|e| e.to_string())?;
        if changed != 1 { return Err("资讯记录已不存在".into()); }
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn archive_persists_original_and_is_bounded() {
        let db = Database {
            conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate_news().unwrap();
        for i in 0..505 {
            db.archive_news(&serde_json::json!({"signal_id":format!("news-{i}"),"body":"摘要","original_body":"原文","news_source":"测试"})).unwrap();
        }
        let rows = db.news_archive().unwrap();
        assert_eq!(rows.len(), 500);
        assert_eq!(db.news_research_archive().unwrap().len(), 505);
        assert!(rows.iter().all(|r| r["original_body"] == "原文"));
    }

    #[test]
    fn migration_removes_only_legacy_timeline_cards() {
        let db = Database { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        db.migrate_news().unwrap();
        db.archive_news(&serde_json::json!({"signal_id":"timeline:2026-09-27:preopen"})).unwrap();
        db.archive_news(&serde_json::json!({"signal_id":"news:kept"})).unwrap();
        db.migrate_news().unwrap();
        let rows = db.news_archive().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["signal_id"], "news:kept");
    }

    #[test]
    fn claim_is_restart_safe_and_cleanup_keeps_recent_keys() {
        let db = Database {
            conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate_news().unwrap();
        assert!(db
            .claim_news("flash:1", "flash", "2026-09-10T00:00:00Z")
            .unwrap());
        assert!(!db
            .claim_news("flash:1", "flash", "2026-09-18T00:00:00Z")
            .unwrap());
        assert_eq!(db.purge_news_before("2026-09-11T00:00:00Z").unwrap(), 1);
        assert!(db
            .claim_news("flash:1", "flash", "2026-09-18T00:00:00Z")
            .unwrap());
    }

    #[test]
    fn migration_removes_auto_quota_but_preserves_manual_analysis() {
        let db = Database { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        db.migrate_news().unwrap();
        db.conn.lock().unwrap().execute_batch("CREATE TABLE news_ai_auto_claim(day TEXT,dedupe_key TEXT); INSERT INTO news_ai_auto_claim VALUES('2026-09-27','a');").unwrap();
        db.archive_news(&serde_json::json!({"signal_id":"news:manual", "agent_summary":true,"body":"已有手动解读","original_body":"采集原文"})).unwrap();
        db.migrate_news().unwrap();
        let quota_tables: u32 = db.conn.lock().unwrap().query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='news_ai_auto_claim'",[],|r|r.get(0)).unwrap();
        assert_eq!(quota_tables, 0);
        let saved = db.news_archive_item("news:manual").unwrap().unwrap();
        assert_eq!(saved["agent_summary"], true);
        assert_eq!(saved["body"], "已有手动解读");
        assert_eq!(saved["original_body"], "采集原文");
    }

    #[test]
    fn legacy_auto_modes_migrate_to_direct_and_claude_only() {
        let dir = std::env::temp_dir().join(format!("bull-news-migration-{}-{}", std::process::id(), chrono::Utc::now().timestamp_micros()));
        for mode in ["ai", "hybrid", "direct"] {
            let db = Database::open(dir.clone()).unwrap();
            db.set_setting("news_notification_mode", mode).unwrap();
            db.set_setting("news_ai_daily_limit", "20").unwrap();
            db.set_setting("news_ai_keywords", "机器人").unwrap();
            db.set_setting("agent_provider", "codex").unwrap();
            db.set_setting("agent_codex_model", "legacy").unwrap();
            drop(db);
            let db = Database::open(dir.clone()).unwrap();
            assert_eq!(db.get_setting("news_notification_mode").unwrap().as_deref(), Some("direct"));
            assert_eq!(db.get_setting("agent_provider").unwrap().as_deref(), Some("claude"));
            assert!(db.get_setting("news_ai_daily_limit").unwrap().is_none());
            assert!(db.get_setting("news_ai_keywords").unwrap().is_none());
            assert!(db.get_setting("agent_codex_model").unwrap().is_none());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
