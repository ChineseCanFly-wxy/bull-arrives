use super::Database;
use rusqlite::{params, Result as SqliteResult};

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
            CREATE TABLE IF NOT EXISTS news_ai_auto_claim(day TEXT NOT NULL, dedupe_key TEXT NOT NULL, PRIMARY KEY(day,dedupe_key));
            CREATE TABLE IF NOT EXISTS daily_briefs(day TEXT NOT NULL,stage TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(day,stage));
            CREATE TABLE IF NOT EXISTS daily_brief_alerts(id TEXT PRIMARY KEY,received_at TEXT NOT NULL,payload TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS idx_daily_brief_alerts_at ON daily_brief_alerts(received_at);",
        )?;
        conn.execute("DELETE FROM news_archive WHERE id LIKE 'timeline:%'", [])?;
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
        conn.execute("DELETE FROM news_archive WHERE id NOT IN (SELECT id FROM news_archive ORDER BY received_at DESC LIMIT 500)",[]).map_err(|e|e.to_string())?;
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

    /// Claim a daily automatic AI slot before launching Claude. Failed calls still count because they may cost money.
    pub fn claim_news_auto_ai(&self, day: &str, key: &str, limit: u32) -> SqliteResult<bool> {
        if limit == 0 { return Ok(false); }
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM news_ai_auto_claim WHERE day < ?1", [day])?;
        let used: u32 = tx.query_row("SELECT COUNT(*) FROM news_ai_auto_claim WHERE day=?1", [day], |row| row.get(0))?;
        let claimed = used < limit && tx.execute(
            "INSERT OR IGNORE INTO news_ai_auto_claim(day,dedupe_key) VALUES (?1,?2)",
            params![day,key],
        )? == 1;
        tx.commit()?;
        Ok(claimed)
    }

    pub fn news_auto_ai_used(&self, day: &str) -> SqliteResult<u32> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row("SELECT COUNT(*) FROM news_ai_auto_claim WHERE day=?1", [day], |row| row.get(0))
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
    fn automatic_ai_slots_are_atomic_and_daily() {
        let db = Database { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        db.migrate_news().unwrap();
        assert!(!db.claim_news_auto_ai("2026-09-27", "a", 0).unwrap());
        assert!(db.claim_news_auto_ai("2026-09-27", "a", 2).unwrap());
        assert!(!db.claim_news_auto_ai("2026-09-27", "a", 2).unwrap());
        assert!(db.claim_news_auto_ai("2026-09-27", "b", 2).unwrap());
        assert!(!db.claim_news_auto_ai("2026-09-27", "c", 2).unwrap());
        assert_eq!(db.news_auto_ai_used("2026-09-27").unwrap(), 2);
        assert!(db.claim_news_auto_ai("2026-09-28", "c", 2).unwrap());
        assert_eq!(db.news_auto_ai_used("2026-09-28").unwrap(), 1);
    }

    #[test]
    fn legacy_all_ai_setting_migrates_to_selective_mode() {
        let dir = std::env::temp_dir().join(format!("bull-news-migration-{}-{}", std::process::id(), chrono::Utc::now().timestamp_micros()));
        let db = Database::open(dir.clone()).unwrap();
        db.set_setting("news_notification_mode", "ai").unwrap();
        drop(db);
        let db = Database::open(dir.clone()).unwrap();
        assert_eq!(db.get_setting("news_notification_mode").unwrap().as_deref(), Some("hybrid"));
        assert_eq!(db.get_setting("news_ai_daily_limit").unwrap().as_deref(), Some("3"));
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
