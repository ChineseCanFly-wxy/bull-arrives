use super::Database;
use rusqlite::{params, OptionalExtension, Result as SqliteResult};
use serde::{Deserialize, Serialize};

/// 持仓与自选项一一对应。`cost_price` 是每股成本乘以 10_000 后的十进制整数文本；
/// `None` 表示未填写成本，`Some("0")` 表示明确填写了零成本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holding {
    pub watch_id: i64,
    pub cost_price: Option<String>,
    pub shares: i64,
}

/// 前端 Number 能无损表达、同时显著低于 SQLite INTEGER 上限的股数上限。
pub const MAX_HOLDING_SHARES: i64 = 9_000_000_000_000_000;
const MAX_SCALED_COST: u64 = i64::MAX as u64;

impl Database {
    /// 创建持仓表。迁移可重复执行；外键确保持仓不会脱离自选项。
    pub fn migrate_holdings(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS holdings (
                 watch_id   INTEGER PRIMARY KEY NOT NULL,
                 cost_price TEXT,
                 shares     INTEGER NOT NULL DEFAULT 0
                            CHECK (shares >= 0 AND shares <= 9000000000000000),
                 FOREIGN KEY (watch_id) REFERENCES watchlist(id) ON DELETE CASCADE,
                 CHECK (
                     cost_price IS NULL OR
                     (length(cost_price) BETWEEN 1 AND 19 AND
                      cost_price NOT GLOB '*[^0-9]*')
                 )
             );",
        )?;
        Ok(())
    }

    pub fn get_holdings(&self) -> SqliteResult<Vec<Holding>> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn.prepare(
            "SELECT watch_id, cost_price, shares FROM holdings ORDER BY watch_id",
        )?;
        let holdings = statement
            .query_map([], |row| {
                Ok(Holding {
                    watch_id: row.get(0)?,
                    cost_price: row.get(1)?,
                    shares: row.get(2)?,
                })
            })?
            .collect();
        holdings
    }

    /// 新增或覆盖一个持仓。成本采用缩放 10_000 倍的无符号整数文本。
    pub fn save_holding(
        &self,
        watch_id: i64,
        cost_price: Option<&str>,
        shares: i64,
    ) -> Result<(), String> {
        if watch_id <= 0 {
            return Err("自选项编号无效".into());
        }
        if !(0..=MAX_HOLDING_SHARES).contains(&shares) {
            return Err(format!("持仓股数应在 0 到 {MAX_HOLDING_SHARES} 之间"));
        }
        let cost_price = validate_scaled_cost(cost_price)?;
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let watch_exists = conn
            .query_row(
                "SELECT 1 FROM watchlist WHERE id = ?1",
                [watch_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if !watch_exists {
            return Err("自选项不存在".into());
        }
        conn.execute(
            "INSERT INTO holdings (watch_id, cost_price, shares) VALUES (?1, ?2, ?3)
             ON CONFLICT(watch_id) DO UPDATE SET
                 cost_price = excluded.cost_price,
                 shares = excluded.shares",
            params![watch_id, cost_price, shares],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_holding(&self, watch_id: i64) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute("DELETE FROM holdings WHERE watch_id = ?1", [watch_id])?;
        Ok(())
    }
}

fn validate_scaled_cost(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value else { return Ok(None) };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("成本价应为非负的定点整数文本".into());
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "成本价超出安全范围".to_string())?;
    if parsed > MAX_SCALED_COST {
        return Err("成本价超出安全范围".into());
    }
    Ok(Some(parsed.to_string()))
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
        database.migrate_holdings().unwrap();
        database
    }

    fn add_watch(database: &Database) -> i64 {
        database.add_watch("sh600000", "CN", "浦发银行").unwrap();
        database.get_watchlist().unwrap()[0].id
    }

    #[test]
    fn migrates_idempotently_and_upserts_one_holding_per_watch() {
        let database = database();
        database.migrate_holdings().unwrap();
        let watch_id = add_watch(&database);
        database.save_holding(watch_id, Some("123400"), 100).unwrap();
        database.save_holding(watch_id, Some("125001"), 200).unwrap();

        assert_eq!(
            database.get_holdings().unwrap(),
            vec![Holding {
                watch_id,
                cost_price: Some("125001".into()),
                shares: 200,
            }]
        );
    }

    #[test]
    fn distinguishes_null_and_zero_cost_and_allows_clearing_cost() {
        let database = database();
        let watch_id = add_watch(&database);
        database.save_holding(watch_id, Some("0"), 300).unwrap();
        assert_eq!(database.get_holdings().unwrap()[0].cost_price.as_deref(), Some("0"));

        database.save_holding(watch_id, None, 300).unwrap();
        let holding = &database.get_holdings().unwrap()[0];
        assert_eq!(holding.cost_price, None);
        assert_eq!(holding.shares, 300);
    }

    #[test]
    fn validates_references_numbers_and_deletion() {
        let database = database();
        let watch_id = add_watch(&database);
        assert!(database.save_holding(999, Some("10000"), 1).is_err());
        assert!(database.save_holding(watch_id, Some("1.25"), 1).is_err());
        assert!(database.save_holding(watch_id, Some("-1"), 1).is_err());
        assert!(database.save_holding(watch_id, None, -1).is_err());
        assert!(database
            .save_holding(watch_id, None, MAX_HOLDING_SHARES + 1)
            .is_err());

        database.save_holding(watch_id, None, 0).unwrap();
        database.delete_holding(watch_id).unwrap();
        assert!(database.get_holdings().unwrap().is_empty());
    }

    #[test]
    fn cascades_when_watch_is_removed() {
        let database = database();
        let watch_id = add_watch(&database);
        database.save_holding(watch_id, Some("10000"), 10).unwrap();
        database.remove_watch("sh600000", "CN").unwrap();
        assert!(database.get_holdings().unwrap().is_empty());
    }
}
