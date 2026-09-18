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
            CREATE INDEX IF NOT EXISTS idx_news_seen_at ON news_seen(seen_at);",
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
