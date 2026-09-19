use super::Database;
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentRunRecord {
    pub id: i64,
    pub fingerprint: String,
    pub task: String,
    pub role: String,
    pub round: u8,
    pub status: String,
    pub started_at: String,
    pub duration_ms: Option<u64>,
    pub prompt_hash: String,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AgentRunDetail {
    pub prompt: String,
    pub input_json: String,
    pub schema_json: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_records_are_bounded_and_keep_exact_task_inputs() {
        let db = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate_agent().unwrap();
        let mut last = 0;
        for index in 0..105 {
            last = db
                .start_agent_run(
                    "snapshot",
                    "single_stock_analysis",
                    "",
                    0,
                    "hash",
                    "实际提示词",
                    &format!("{{\"question\":\"问题{index}\"}}"),
                    "{}",
                )
                .unwrap();
            db.finish_agent_run(last, 25, None).unwrap();
        }
        assert!(db.agent_run_detail(1).is_err());
        let rows = db.agent_run_history("snapshot").unwrap();
        assert_eq!(rows.len(), 20);
        assert_eq!(rows[0].id, last);
        assert_eq!(rows[0].status, "received");
        assert_eq!(db.agent_run_detail(last).unwrap().prompt, "实际提示词");
        assert!(db
            .agent_run_detail(last)
            .unwrap()
            .input_json
            .contains("问题104"));
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            100
        );
    }
}

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
             );
             CREATE TABLE IF NOT EXISTS agent_runs(
               id INTEGER PRIMARY KEY AUTOINCREMENT,fingerprint TEXT NOT NULL,
               task TEXT NOT NULL,role TEXT NOT NULL,round INTEGER NOT NULL,
               status TEXT NOT NULL,started_at TEXT NOT NULL,duration_ms INTEGER,
               prompt_hash TEXT NOT NULL,prompt TEXT NOT NULL,input_json TEXT NOT NULL,
               schema_json TEXT NOT NULL,error TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_agent_runs_fingerprint ON agent_runs(fingerprint,id);",
        )
    }

    pub fn start_agent_run(
        &self,
        fingerprint: &str,
        task: &str,
        role: &str,
        round: u8,
        prompt_hash: &str,
        prompt: &str,
        input_json: &str,
        schema: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("INSERT INTO agent_runs(fingerprint,task,role,round,status,started_at,prompt_hash,prompt,input_json,schema_json) VALUES(?1,?2,?3,?4,'running',?5,?6,?7,?8,?9)",
            params![fingerprint,task,role,round,chrono::Utc::now().to_rfc3339(),prompt_hash,prompt,input_json,schema]).map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        // Retain only the latest 100 diagnostic calls, never an unbounded input archive.
        conn.execute("DELETE FROM agent_runs WHERE id NOT IN (SELECT id FROM agent_runs ORDER BY id DESC LIMIT 100)", []).map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn finish_agent_run(
        &self,
        id: i64,
        duration_ms: u64,
        error: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let status = if error.is_some() {
            "failed"
        } else {
            "received"
        };
        conn.execute(
            "UPDATE agent_runs SET status=?2,duration_ms=?3,error=?4 WHERE id=?1",
            params![id, status, duration_ms, error],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    pub fn agent_run_history(&self, fingerprint: &str) -> Result<Vec<AgentRunRecord>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id,fingerprint,task,role,round,status,started_at,duration_ms,prompt_hash,error FROM agent_runs WHERE fingerprint=?1 ORDER BY id DESC LIMIT 20").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([fingerprint], |r| {
                Ok(AgentRunRecord {
                    id: r.get(0)?,
                    fingerprint: r.get(1)?,
                    task: r.get(2)?,
                    role: r.get(3)?,
                    round: r.get(4)?,
                    status: r.get(5)?,
                    started_at: r.get(6)?,
                    duration_ms: r.get(7)?,
                    prompt_hash: r.get(8)?,
                    error: r.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn agent_run_detail(&self, id: i64) -> Result<AgentRunDetail, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT prompt,input_json,schema_json FROM agent_runs WHERE id=?1",
            [id],
            |r| {
                Ok(AgentRunDetail {
                    prompt: r.get(0)?,
                    input_json: r.get(1)?,
                    schema_json: r.get(2)?,
                })
            },
        )
        .map_err(|_| "记录不存在或已超过最近 100 次的保留范围".to_string())
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
