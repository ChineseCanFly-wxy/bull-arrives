use super::Database;
use rusqlite::{params, OptionalExtension};

impl Database {
    pub fn migrate_agent(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_snapshots(
               fingerprint TEXT PRIMARY KEY,input_json TEXT NOT NULL,created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS agent_results(
               cache_key TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,result_json TEXT NOT NULL,created_at TEXT NOT NULL,
               FOREIGN KEY(fingerprint) REFERENCES agent_snapshots(fingerprint) ON DELETE RESTRICT
             );",
        )
    }

    pub fn save_agent_snapshot(&self, fingerprint: &str, input_json: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO agent_snapshots(fingerprint,input_json,created_at) VALUES(?1,?2,?3)",
            params![fingerprint, input_json, chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()],
        )
        .map_err(|error| error.to_string())?;
        let stored: String = conn
            .query_row(
                "SELECT input_json FROM agent_snapshots WHERE fingerprint=?1",
                [fingerprint],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if stored == input_json {
            Ok(())
        } else {
            Err("Agent 快照指纹冲突".into())
        }
    }

    pub fn get_agent_snapshot(&self, fingerprint: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.query_row(
            "SELECT input_json FROM agent_snapshots WHERE fingerprint=?1",
            [fingerprint],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
    }

    pub fn get_agent_result(&self, cache_key: &str) -> Result<Option<(String, String)>, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.query_row(
            "SELECT result_json,created_at FROM agent_results WHERE cache_key=?1",
            [cache_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())
    }

    pub fn save_agent_result(
        &self,
        cache_key: &str,
        fingerprint: &str,
        result_json: &str,
        created_at: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute(
            "INSERT INTO agent_results(cache_key,fingerprint,result_json,created_at) VALUES(?1,?2,?3,?4)
             ON CONFLICT(cache_key) DO UPDATE SET fingerprint=excluded.fingerprint,result_json=excluded.result_json,created_at=excluded.created_at",
            params![cache_key, fingerprint, result_json, created_at],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    pub fn delete_agent_result(&self, cache_key: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute("DELETE FROM agent_results WHERE cache_key=?1", [cache_key])
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
