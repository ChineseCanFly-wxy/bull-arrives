// src-tauri/src/db/monitors.rs
// 个股监控（量化自动止损/止盈）的持久化。
//
// 与用户手动填参数的 `price_alerts` 不同：这里的止损价/止盈价由**量化模型**
// （ATR 波动止损）自动计算，用户只选择「监控哪只股票」。`stop_price`/`take_price`
// 是命令层算好后写入的，用户不直接填数字。

use super::Database;
use rusqlite::{params, Result as SqliteResult};
use serde::{Deserialize, Serialize};

/// 一条个股监控规则。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Monitor {
    pub id: i64,
    pub code: String,
    pub market: String,
    pub name: String,
    pub enabled: bool,
    /// 参考价（开启监控时的最新收盘价，止损/止盈围绕它计算）
    pub reference_price: f64,
    /// 量化自动计算的止损价（跌破触发止损提醒）
    pub stop_price: f64,
    /// 量化自动计算的止盈价（突破触发止盈提醒）
    pub take_price: f64,
    /// 已触发的条件（"stop_loss" / "take_profit"），用于一次性去重；重新计算时清空
    pub last_triggered: Option<String>,
    /// 止损/止盈位的计算时间
    pub updated_at: String,
}

impl Database {
    /// 创建监控表。幂等。
    pub fn migrate_monitors(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS monitors (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                code            TEXT NOT NULL,
                market          TEXT NOT NULL DEFAULT 'CN',
                name            TEXT NOT NULL,
                enabled         INTEGER NOT NULL DEFAULT 1,
                reference_price REAL NOT NULL,
                stop_price      REAL NOT NULL,
                take_price      REAL NOT NULL,
                last_triggered  TEXT,
                updated_at      TEXT NOT NULL,
                UNIQUE(code, market)
            );",
        )?;
        Ok(())
    }

    pub fn get_monitors(&self) -> SqliteResult<Vec<Monitor>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, code, market, name, enabled, reference_price,
                    stop_price, take_price, last_triggered, updated_at
             FROM monitors ORDER BY id",
        )?;
        let rows = stmt.query_map([], monitor_from_row)?;
        rows.collect()
    }

    /// 新增或覆盖一条监控（止损/止盈价由调用方算好传入）。覆盖时清空触发状态。
    pub fn upsert_monitor(&self, monitor: &Monitor) -> Result<(), String> {
        if monitor.code.trim().is_empty() {
            return Err("股票代码不能为空".into());
        }
        if !monitor.reference_price.is_finite() || monitor.reference_price <= 0.0 {
            return Err("参考价无效".into());
        }
        if !monitor.stop_price.is_finite()
            || monitor.stop_price <= 0.0
            || monitor.stop_price >= monitor.reference_price
        {
            return Err("止损价应低于参考价".into());
        }
        if !monitor.take_price.is_finite() || monitor.take_price <= monitor.reference_price {
            return Err("止盈价应高于参考价".into());
        }

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO monitors
                (code, market, name, enabled, reference_price, stop_price, take_price, last_triggered, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)
             ON CONFLICT(code, market) DO UPDATE SET
                name = excluded.name,
                enabled = excluded.enabled,
                reference_price = excluded.reference_price,
                stop_price = excluded.stop_price,
                take_price = excluded.take_price,
                last_triggered = NULL,
                updated_at = excluded.updated_at",
            params![
                monitor.code,
                monitor.market,
                monitor.name,
                i64::from(monitor.enabled),
                monitor.reference_price,
                monitor.stop_price,
                monitor.take_price,
                monitor.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_monitor(&self, code: &str, market: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM monitors WHERE code = ?1 AND market = ?2",
            params![code, market],
        )?;
        Ok(())
    }

    /// 停止 / 恢复一条监控 —— 只改 `enabled` 开关，不碰其他字段。
    ///
    /// ⚠️ 与 `delete_monitor` 的分工要守住：
    /// - **停止**：参考价、止损/止盈位、`last_triggered` 全部保留，恢复后立刻按原价位继续判断；
    ///   用户只是暂时不想被这只票打扰（比如已经减仓、或者想先观察几天）。
    /// - **删除**：不再监控这只票了。
    ///
    /// 之所以不能靠「删掉再开启」代替停止：`save_monitor` 会用**当时的收盘价**重算参考价，
    /// 止损/止盈位会整体挪位，`last_triggered` 也会被清空 —— 等于换了一套规则，
    /// 而不是"暂停一下"。
    ///
    /// 返回是否真的改到了行：没有对应记录时返回 `false`，由调用方决定怎么提示。
    pub fn set_monitor_enabled(
        &self,
        code: &str,
        market: &str,
        enabled: bool,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let changed = conn.execute(
            "UPDATE monitors SET enabled = ?3 WHERE code = ?1 AND market = ?2",
            params![code, market, i64::from(enabled)],
        )?;
        Ok(changed > 0)
    }

    /// 记录触发的条件（一次性去重：之后同条件不再重复触发）。
    pub fn mark_monitor_triggered(&self, id: i64, triggered: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE monitors SET last_triggered = ?1 WHERE id = ?2",
            params![triggered, id],
        )?;
        Ok(())
    }
}

fn monitor_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<Monitor> {
    Ok(Monitor {
        id: row.get(0)?,
        code: row.get(1)?,
        market: row.get(2)?,
        name: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        reference_price: row.get(5)?,
        stop_price: row.get(6)?,
        take_price: row.get(7)?,
        last_triggered: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn database() -> Database {
        let database = Database {
            conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        database.migrate().unwrap();
        database.migrate_monitors().unwrap();
        database
    }

    fn sample() -> Monitor {
        Monitor {
            id: 0,
            code: "sh600519".into(),
            market: "CN".into(),
            name: "贵州茅台".into(),
            enabled: true,
            reference_price: 1000.0,
            stop_price: 950.0,
            take_price: 1080.0,
            last_triggered: None,
            updated_at: "2026-09-12T00:00:00".into(),
        }
    }

    #[test]
    fn upsert_and_read_roundtrip() {
        let db = database();
        let m = sample();
        db.upsert_monitor(&m).unwrap();
        let list = db.get_monitors().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].stop_price, 950.0);
        assert_eq!(list[0].take_price, 1080.0);

        let mut m2 = sample();
        m2.id = list[0].id;
        m2.stop_price = 940.0;
        db.upsert_monitor(&m2).unwrap();
        let list = db.get_monitors().unwrap();
        assert_eq!(list[0].stop_price, 940.0);
    }

    #[test]
    fn validation_rejects_bad_prices() {
        let db = database();
        let mut m = sample();

        m.reference_price = 0.0;
        assert!(db.upsert_monitor(&m).is_err());

        m = sample();
        m.stop_price = 1001.0; // 止损必须低于参考价
        assert!(db.upsert_monitor(&m).is_err());

        m = sample();
        m.take_price = 999.0; // 止盈必须高于参考价
        assert!(db.upsert_monitor(&m).is_err());
    }

    #[test]
    fn trigger_marking_and_delete() {
        let db = database();
        let m = sample();
        db.upsert_monitor(&m).unwrap();
        let id = db.get_monitors().unwrap()[0].id;

        db.mark_monitor_triggered(id, "stop_loss").unwrap();
        assert_eq!(
            db.get_monitors().unwrap()[0].last_triggered.as_deref(),
            Some("stop_loss")
        );

        db.delete_monitor("sh600519", "CN").unwrap();
        assert!(db.get_monitors().unwrap().is_empty());
    }

    #[test]
    fn set_enabled_pauses_without_losing_the_rule() {
        let db = database();
        db.upsert_monitor(&sample()).unwrap();
        let id = db.get_monitors().unwrap()[0].id;
        db.mark_monitor_triggered(id, "take_profit").unwrap();

        assert!(db.set_monitor_enabled("sh600519", "CN", false).unwrap());
        let paused = db.get_monitors().unwrap();
        assert_eq!(paused.len(), 1, "停止监控不应删掉记录");
        assert!(!paused[0].enabled);
        // 关键：止损/止盈位与触发状态都必须原样留着 —— 恢复后按同一套价位继续判断
        assert_eq!(paused[0].stop_price, 950.0);
        assert_eq!(paused[0].take_price, 1080.0);
        assert_eq!(paused[0].last_triggered.as_deref(), Some("take_profit"));

        assert!(db.set_monitor_enabled("sh600519", "CN", true).unwrap());
        assert!(db.get_monitors().unwrap()[0].enabled);
    }

    #[test]
    fn set_enabled_reports_missing_row() {
        let db = database();
        assert!(!db.set_monitor_enabled("sh600519", "CN", false).unwrap());
    }

    /// `upsert_monitor` 覆盖时会把 `enabled` 一起写进去 —— 「刷新」不该把停掉的监控偷偷打开。
    #[test]
    fn upsert_preserves_explicitly_disabled_state() {
        let db = database();
        let mut m = sample();
        m.enabled = false;
        db.upsert_monitor(&m).unwrap();
        assert!(!db.get_monitors().unwrap()[0].enabled);
    }
}
