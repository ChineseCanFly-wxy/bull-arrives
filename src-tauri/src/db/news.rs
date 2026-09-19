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
            CREATE TABLE IF NOT EXISTS news_archive(id TEXT PRIMARY KEY,payload TEXT NOT NULL,received_at TEXT NOT NULL);",
        )
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
}
