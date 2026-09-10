pub mod groups;
pub mod holdings;
#[cfg(test)]
mod regression_tests;

use rusqlite::{Connection, Result as SqliteResult, params};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open or create database, auto-run migrations
    pub fn open(app_dir: PathBuf) -> SqliteResult<Self> {
        if let Err(e) = std::fs::create_dir_all(&app_dir) {
            log::warn!("Failed to create app data dir {:?}: {}", app_dir, e);
        }
        let db_path = app_dir.join("bull-arrives.db");
        let conn = Connection::open(db_path)?;
        let db = Self { conn: Mutex::new(conn) };
        db.migrate()?;
        db.migrate_groups()?;
        db.migrate_holdings()?;
        db.migrate_price_alerts()?;
        db.migrate_watchlist_codes()?;
        db.init_defaults()?;
        Ok(db)
    }

    fn migrate(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS watchlist (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                code        TEXT NOT NULL,
                market      TEXT NOT NULL DEFAULT 'CN',
                name        TEXT NOT NULL,
                sort_order  INTEGER DEFAULT 0,
                added_at    TEXT NOT NULL,
                UNIQUE(code, market)
            );
            CREATE TABLE IF NOT EXISTS settings (
                key         TEXT PRIMARY KEY,
                value       TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS quote_cache (
                code        TEXT NOT NULL,
                market      TEXT NOT NULL DEFAULT 'CN',
                data        TEXT NOT NULL,
                cached_at   TEXT NOT NULL,
                PRIMARY KEY (code, market)
            );
            CREATE TABLE IF NOT EXISTS price_alerts (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                code            TEXT NOT NULL,
                market          TEXT NOT NULL DEFAULT 'CN',
                alert_type      TEXT NOT NULL,
                threshold       REAL NOT NULL,
                enabled         INTEGER NOT NULL DEFAULT 1,
                repeat_mode     TEXT NOT NULL DEFAULT 'daily',
                cooldown_minutes INTEGER NOT NULL DEFAULT 0,
                last_triggered_at TEXT,
                last_triggered_day TEXT,
                last_value       REAL,
                last_value_day   TEXT,
                UNIQUE(code, market, alert_type)
            );"
        )?;
        Ok(())
    }

    /// Extend an existing `price_alerts` table without dropping user rules.
    fn migrate_price_alerts(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(price_alerts)")?;
            let columns = stmt
                .query_map([], |row| row.get(1))?
                .collect::<SqliteResult<_>>()?;
            columns
        };
        for (name, data_type) in [("last_value", "REAL"), ("last_value_day", "TEXT")] {
            if !columns.iter().any(|column| column == name) {
                conn.execute(
                    &format!("ALTER TABLE price_alerts ADD COLUMN {name} {data_type}"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    /// One-time data migration: prefix bare 6-digit CN watchlist codes with their
    /// exchange (`sh`/`sz`). Historical rows stored the bare code (e.g. "600519"),
    /// which is ambiguous when a code is shared between an index and a stock —
    /// e.g. 000852 is both sh000852 (中证1000 index) and sz000852 (石化机械 stock).
    /// The quote pipeline now keys on the full symbol, so legacy rows are upgraded.
    ///
    /// Idempotent: a full symbol already carries the sh/sz prefix (length 8) and is
    /// left untouched. The quote_cache is cleared because it holds serialized quotes
    /// keyed by the old bare code; it repopulates on the next poll.
    fn migrate_watchlist_codes(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id, code FROM watchlist WHERE market = 'CN'")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<SqliteResult<_>>()?;
        drop(stmt);

        let mut migrated = false;
        for (id, code) in rows {
            if code.starts_with("sh") || code.starts_with("sz") {
                continue;
            }
            if code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()) {
                let prefix = if code.starts_with('6')
                    || code.starts_with('5')
                    || code.starts_with('9')
                {
                    "sh"
                } else {
                    "sz"
                };
                let full = format!("{}{}", prefix, code);
                conn.execute(
                    "UPDATE watchlist SET code = ?1 WHERE id = ?2",
                    params![full, id],
                )?;
                migrated = true;
            }
        }
        // Only clear the cache when legacy codes were actually rewritten. The
        // cache keys change from bare to full code, so stale entries are invalid;
        // but clearing it on every launch would defeat startup cache restoration.
        if migrated {
            conn.execute("DELETE FROM quote_cache", [])?;
        }
        Ok(())
    }

    /// Insert default settings values (default data source is Tencent)
    pub fn init_defaults(&self) -> SqliteResult<()> {
        let defaults = [
            ("active_datasource", "tencent"),
            // 0 = AUTO (follow trading session cadence: 2s during trading,
            // 5s pre-open, 10s lunch break, 30s closed)
            ("refresh_interval", "0"),
            ("theme", "light"),
            ("ticker_visible", "1"),
            ("auto_launch", "false"),
            ("alerts_enabled", "1"),
        ];
        for (k, v) in defaults {
            if self.get_setting(k)?.is_none() {
                self.set_setting(k, v)?;
            }
        }
        Ok(())
    }

    // ── Watchlist CRUD ──

    pub fn get_watchlist(&self) -> SqliteResult<Vec<WatchItem>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, code, market, name, sort_order, added_at
             FROM watchlist ORDER BY sort_order ASC, id ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WatchItem {
                id: row.get(0)?,
                code: row.get(1)?,
                market: row.get(2)?,
                name: row.get(3)?,
                sort_order: row.get(4)?,
                added_at: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_watch(&self, code: &str, market: &str, name: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Place new items at the end by computing the next sort_order from the
        // current maximum.  Without this, every new item would get DEFAULT 0
        // and appear at an unpredictable position after deletions leave gaps.
        let max_sort: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) FROM watchlist",
                [],
                |row| row.get(0),
            )
            .unwrap_or(-1);
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        conn.execute(
            "INSERT OR IGNORE INTO watchlist (code, market, name, sort_order, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![code, market, name, max_sort + 1, now],
        )?;
        Ok(())
    }

    pub fn remove_watch(&self, code: &str, market: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.unchecked_transaction()?;
        let groups_migrated: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='watch_group_members')",
            [],
            |row| row.get(0),
        )?;
        if groups_migrated {
            tx.execute(
                "DELETE FROM watch_group_members WHERE watch_id IN
                    (SELECT id FROM watchlist WHERE code = ?1 AND market = ?2)",
                params![code, market],
            )?;
        }
        tx.execute("DELETE FROM price_alerts WHERE code=?1 AND market=?2", params![code, market])?;
        tx.execute("DELETE FROM quote_cache WHERE code=?1 AND market=?2", params![code, market])?;
        tx.execute(
            "DELETE FROM watchlist WHERE code = ?1 AND market = ?2",
            params![code, market],
        )?;
        tx.commit()
    }

    pub fn reorder_watch(&self, ids: &[i64]) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Wrap in a transaction so that a crash mid-reorder doesn't leave
        // sort_orders in an inconsistent half-updated state.
        let tx = conn.unchecked_transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE watchlist SET sort_order = ?1 WHERE id = ?2",
                params![i as i32, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_watch_codes(&self) -> SqliteResult<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT code, market FROM watchlist ORDER BY sort_order ASC, id ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    // ── Settings CRUD ──

    pub fn get_setting(&self, key: &str) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(Ok(v)) => Ok(Some(v)),
            _ => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_all_settings(&self) -> SqliteResult<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    // ── Quote Cache ──

    pub fn cache_quotes(&self, quotes: &[crate::domain::Quote]) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        // Wrap all INSERTs in a single transaction for better performance.
        // SAFETY: `unchecked_transaction` is safe here because:
        // - The connection is protected by a Mutex (no concurrent access).
        // - The loop below only executes INSERT/REPLACE (no reads that depend
        //   on uncommitted state within this transaction).
        // - If this function is refactored to remove the Mutex, replace with
        //   a regular `transaction()` to avoid data races.
        let tx = conn.unchecked_transaction()?;
        for q in quotes {
            let data = serde_json::to_string(q).unwrap_or_else(|e| {
                log::warn!(
                    "Failed to serialize quote {}:{} for cache: {}",
                    q.market, q.code, e
                );
                String::new()
            });
            if data.is_empty() {
                continue;
            }
            tx.execute(
                "INSERT OR REPLACE INTO quote_cache (code, market, data, cached_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![q.code, q.market, data, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ── Atomic Watchlist Reorder Operations ──
    // Each method acquires the DB lock once and completes the entire
    // operation within that lock, preventing TOCTOU races.

    /// Move a watchlist entry to the top (sort_order = 0).
    /// All other entries are shifted down by one position.
    pub fn move_watch_top(&self, id: i64) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id FROM watchlist ORDER BY sort_order ASC, id ASC"
        )?;
        let ids: Vec<i64> = stmt.query_map([], |row| row.get(0))?
            .collect::<SqliteResult<Vec<_>>>()?;

        let mut sort_order = 0i32;
        conn.execute(
            "UPDATE watchlist SET sort_order = ?1 WHERE id = ?2",
            params![sort_order, id],
        )?;
        sort_order += 1;
        for other_id in &ids {
            if *other_id != id {
                conn.execute(
                    "UPDATE watchlist SET sort_order = ?1 WHERE id = ?2",
                    params![sort_order, other_id],
                )?;
                sort_order += 1;
            }
        }
        Ok(())
    }

    /// Swap the target entry with the one above it (decrease sort_order).
    pub fn move_watch_up(&self, id: i64) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id FROM watchlist ORDER BY sort_order ASC, id ASC"
        )?;
        let ids: Vec<i64> = stmt.query_map([], |row| row.get(0))?
            .collect::<SqliteResult<Vec<_>>>()?;

        if let Some(pos) = ids.iter().position(|&x| x == id) {
            if pos > 0 {
                let prev_id = ids[pos - 1];
                // Swap sort_order: target takes the previous entry's position,
                // and the previous entry takes the target's position.
                conn.execute(
                    "UPDATE watchlist SET sort_order = ?1 WHERE id = ?2",
                    params![(pos - 1) as i32, id],
                )?;
                conn.execute(
                    "UPDATE watchlist SET sort_order = ?1 WHERE id = ?2",
                    params![pos as i32, prev_id],
                )?;
            }
        }
        Ok(())
    }

    /// Swap the target entry with the one below it (increase sort_order).
    pub fn move_watch_down(&self, id: i64) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id FROM watchlist ORDER BY sort_order ASC, id ASC"
        )?;
        let ids: Vec<i64> = stmt.query_map([], |row| row.get(0))?
            .collect::<SqliteResult<Vec<_>>>()?;

        if let Some(pos) = ids.iter().position(|&x| x == id) {
            if pos + 1 < ids.len() {
                let next_id = ids[pos + 1];
                conn.execute(
                    "UPDATE watchlist SET sort_order = ?1 WHERE id = ?2",
                    params![(pos + 1) as i32, id],
                )?;
                conn.execute(
                    "UPDATE watchlist SET sort_order = ?1 WHERE id = ?2",
                    params![pos as i32, next_id],
                )?;
            }
        }
        Ok(())
    }

    pub fn get_cached_quotes(&self) -> SqliteResult<Vec<crate::domain::Quote>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT data FROM quote_cache")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut quotes = Vec::new();
        for row in rows {
            if let Ok(data) = row {
                match serde_json::from_str::<crate::domain::Quote>(&data) {
                    Ok(quote) => quotes.push(quote),
                    Err(e) => log::warn!(
                        "Failed to deserialize cached quote (skipping): {}",
                        e
                    ),
                }
            }
        }
        Ok(quotes)
    }

    pub fn get_all_price_alerts(&self) -> SqliteResult<Vec<PriceAlert>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, code, market, alert_type, threshold, enabled, repeat_mode,
                    cooldown_minutes, last_triggered_at, last_triggered_day,
                    last_value, last_value_day FROM price_alerts ORDER BY id",
        )?;
        let rows = stmt.query_map([], price_alert_from_row)?;
        rows.collect()
    }

    pub fn get_price_alerts(&self, code: &str, market: &str) -> SqliteResult<Vec<PriceAlert>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, code, market, alert_type, threshold, enabled, repeat_mode,
                    cooldown_minutes, last_triggered_at, last_triggered_day,
                    last_value, last_value_day
             FROM price_alerts WHERE code = ?1 AND market = ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![code, market], price_alert_from_row)?;
        rows.collect()
    }

    pub fn watch_exists(&self, code: &str, market: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM watchlist WHERE code = ?1 AND market = ?2)",
            params![code, market],
            |row| row.get(0),
        )
    }

    pub fn upsert_price_alert(&self, alert: &PriceAlert) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO price_alerts
                (code, market, alert_type, threshold, enabled, repeat_mode, cooldown_minutes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(code, market, alert_type) DO UPDATE SET
                threshold = excluded.threshold, enabled = excluded.enabled,
                repeat_mode = excluded.repeat_mode,
                cooldown_minutes = excluded.cooldown_minutes,
                last_triggered_at = CASE WHEN threshold <> excluded.threshold THEN NULL ELSE last_triggered_at END,
                last_triggered_day = CASE WHEN threshold <> excluded.threshold THEN NULL ELSE last_triggered_day END,
                last_value = CASE WHEN threshold <> excluded.threshold OR enabled <> excluded.enabled THEN NULL ELSE last_value END,
                last_value_day = CASE WHEN threshold <> excluded.threshold OR enabled <> excluded.enabled THEN NULL ELSE last_value_day END",
            params![alert.code, alert.market, alert.alert_type, alert.threshold,
                    i64::from(alert.enabled), alert.repeat_mode, alert.cooldown_minutes],
        )?;
        Ok(())
    }

    /// Atomically persist the observation and optional trigger state. Events must
    /// only be emitted after this call commits successfully.
    pub fn persist_price_alert_evaluation(
        &self,
        expected: &PriceAlert,
        value: f64,
        value_day: Option<&str>,
        triggered_at: Option<&str>,
        triggered_day: Option<&str>,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let changed = conn.execute(
            "UPDATE price_alerts SET last_value=?1, last_value_day=?2,
                last_triggered_at=COALESCE(?3,last_triggered_at),
                last_triggered_day=COALESCE(?4,last_triggered_day) WHERE id=?5
                AND threshold=?6 AND enabled=?7 AND repeat_mode=?8 AND cooldown_minutes=?9
                AND last_value IS ?10 AND last_triggered_at IS ?11",
            params![value, value_day, triggered_at, triggered_day, expected.id,
                expected.threshold, i64::from(expected.enabled), expected.repeat_mode, expected.cooldown_minutes,
                expected.last_value, expected.last_triggered_at],
        )?;
        if changed == 1 { Ok(()) } else { Err(rusqlite::Error::QueryReturnedNoRows) }
    }

    pub fn delete_price_alert(&self, code: &str, market: &str, alert_type: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM price_alerts WHERE code=?1 AND market=?2 AND alert_type=?3", params![code, market, alert_type])?;
        Ok(())
    }
}

fn price_alert_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<PriceAlert> {
    Ok(PriceAlert {
        id: row.get(0)?, code: row.get(1)?, market: row.get(2)?, alert_type: row.get(3)?,
        threshold: row.get(4)?, enabled: row.get::<_, i64>(5)? != 0,
        repeat_mode: row.get(6)?, cooldown_minutes: row.get(7)?,
        last_triggered_at: row.get(8)?, last_triggered_day: row.get(9)?,
        last_value: row.get(10)?, last_value_day: row.get(11)?,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PriceAlert {
    pub id: i64,
    pub code: String,
    pub market: String,
    pub alert_type: String,
    pub threshold: f64,
    pub enabled: bool,
    pub repeat_mode: String,
    pub cooldown_minutes: i64,
    #[serde(default)]
    pub last_triggered_at: Option<String>,
    #[serde(default)]
    pub last_triggered_day: Option<String>,
    #[serde(default)]
    pub last_value: Option<f64>,
    #[serde(default)]
    pub last_value_day: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WatchItem {
    pub id: i64,
    pub code: String,
    pub market: String,
    pub name: String,
    pub sort_order: i32,
    pub added_at: String,
}
