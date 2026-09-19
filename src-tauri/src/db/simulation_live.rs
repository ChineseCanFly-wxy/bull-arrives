use super::Database;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivePlan {
    pub symbol: String,
    pub buy_low: i64,
    pub buy_high: i64,
    pub stop: i64,
    pub take: i64,
    pub limit_bps: i64,
    pub basis_date: String,
    pub reference_close: i64,
    pub position_pct: f64,
}
#[derive(Debug, Clone, Serialize)]
pub struct LiveStatus {
    pub engine: String,
    pub message: String,
    pub updated_at: Option<String>,
    pub plans: Vec<LivePlan>,
    pub executions: Vec<serde_json::Value>,
    pub marks: usize,
}
impl Database {
    pub fn live_reference_check(
        &self,
        account: i64,
        tick: &crate::simulation_live::LiveTick,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let last: Option<(i64, i64)> = conn
            .query_row(
                "SELECT price,timestamp FROM sim_live_marks WHERE account_id=?1 AND symbol=?2",
                params![account, tick.quote.code],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((price, stamp)) = last {
            let offset = chrono::FixedOffset::east_opt(28800).unwrap();
            let prior = chrono::DateTime::from_timestamp(stamp, 0)
                .ok_or("旧报价时间无效")?
                .with_timezone(&offset);
            let current = chrono::DateTime::from_timestamp(tick.quote.timestamp, 0)
                .ok_or("新报价时间无效")?
                .with_timezone(&offset);
            if current.date_naive() > prior.date_naive()
                && (crate::simulation_live::scaled(tick.quote.prev_close)? - price).abs() > 100
            {
                return Err(
                    "跨日昨收与上次估值不一致（可能除权或缺少收盘记录），请复核，暂停自动成交"
                        .into(),
                );
            }
        }
        Ok(())
    }
    pub fn set_live_manual_plan(&self, account: i64, plan: &LivePlan) -> Result<(), String> {
        if self.experiment_account(account)?.is_some() {
            return Err("研究买卖计划已冻结，不能手动修改".into());
        }
        if self
            .get_sim_detail(account)?
            .positions
            .iter()
            .any(|p| p.symbol == plan.symbol)
        {
            return Err("该标的已有持仓，不能通过再次下单覆盖原来的止损止盈计划".into());
        }
        if !crate::simulation_live::is_a_share(&plan.symbol)
            || plan.stop <= 0
            || plan.stop >= plan.buy_low
            || plan.buy_low > plan.buy_high
            || plan.take <= plan.buy_high
        {
            return Err("价格必须满足：止损 < 买入下限 ≤ 买入上限 < 止盈".into());
        }
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute("INSERT OR REPLACE INTO sim_live_plans(account_id,symbol,plan_json) VALUES(?1,?2,?3)",params![account,plan.symbol,serde_json::to_string(plan).map_err(|e|e.to_string())?]).map(|_|()).map_err(|e|e.to_string())
    }
    pub fn expire_live_orders(&self, account: i64, date: &str) -> Result<(), String> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute("UPDATE sim_orders SET status='rejected',reject_reason='当日实时委托已过期，不在之后的交易日补成交' WHERE account_id=?1 AND status='pending' AND signal_date<?2 AND id IN (SELECT order_id FROM sim_live_orders)",params![account,date]).map(|_|()).map_err(|e|e.to_string())
    }
    pub fn migrate_simulation_live(&self) -> rusqlite::Result<()> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute_batch("CREATE TABLE IF NOT EXISTS sim_live_accounts(account_id INTEGER PRIMARY KEY,message TEXT NOT NULL DEFAULT '',updated_at TEXT,FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE CASCADE);
 CREATE TABLE IF NOT EXISTS sim_live_plans(account_id INTEGER NOT NULL,symbol TEXT NOT NULL,plan_json TEXT NOT NULL,PRIMARY KEY(account_id,symbol),FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE CASCADE);
 CREATE TABLE IF NOT EXISTS sim_live_orders(order_id INTEGER PRIMARY KEY,limit_price INTEGER NOT NULL,submitted_at INTEGER NOT NULL,evidence_json TEXT,FOREIGN KEY(order_id) REFERENCES sim_orders(id) ON DELETE CASCADE);
 CREATE TABLE IF NOT EXISTS sim_live_equity(account_id INTEGER NOT NULL,minute TEXT NOT NULL,equity INTEGER NOT NULL,low_equity INTEGER NOT NULL,high_equity INTEGER NOT NULL,PRIMARY KEY(account_id,minute),FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE CASCADE);
 CREATE TABLE IF NOT EXISTS sim_live_marks(account_id INTEGER NOT NULL,symbol TEXT NOT NULL,price INTEGER NOT NULL,timestamp INTEGER NOT NULL,source TEXT NOT NULL,PRIMARY KEY(account_id,symbol),FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE CASCADE);
 CREATE TABLE IF NOT EXISTS sim_live_risk(account_id INTEGER PRIMARY KEY,peak INTEGER NOT NULL,max_drawdown_bps INTEGER NOT NULL,first_benchmark INTEGER,last_benchmark INTEGER,FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE CASCADE); CREATE TABLE IF NOT EXISTS sim_live_consumed(account_id INTEGER NOT NULL,symbol TEXT NOT NULL,timestamp INTEGER NOT NULL,PRIMARY KEY(account_id,symbol),FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE CASCADE);")
    }
    pub fn enable_live_account(&self, account: i64, plans: &[LivePlan]) -> Result<(), String> {
        let detail = self.get_sim_detail(account)?;
        if !detail.positions.is_empty() || !detail.orders.is_empty() {
            return Err("已有历史订单/持仓，不能混入实时口径，请新建账户".into());
        }
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("INSERT OR IGNORE INTO sim_live_accounts(account_id,message) VALUES(?1,'实时前向：等待 A 股开市后的新报价')",[account]).map_err(|e|e.to_string())?;
        for p in plans {
            tx.execute("INSERT OR REPLACE INTO sim_live_plans(account_id,symbol,plan_json) VALUES(?1,?2,?3)",params![account,p.symbol,serde_json::to_string(p).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn live_account(&self, account: i64) -> Result<bool, String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sim_live_accounts WHERE account_id=?1)",
                [account],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub fn live_order(&self, order: i64, limit: i64, submitted: i64) -> Result<(), String> {
        if limit <= 0 {
            return Err("实时委托需要有效限价".into());
        }
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute("INSERT OR IGNORE INTO sim_live_orders(order_id,limit_price,submitted_at) VALUES(?1,?2,?3)",params![order,limit,submitted]).map(|_|()).map_err(|e|e.to_string())
    }
    pub fn rearm_live_order(&self, order: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute(
                "UPDATE sim_live_orders SET submitted_at=?2 WHERE order_id=?1",
                params![order, chrono::Utc::now().timestamp()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn live_message(&self, account: i64, message: &str) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute(
                "UPDATE sim_live_accounts SET message=?2,updated_at=?3 WHERE account_id=?1",
                params![account, message, chrono::Utc::now().to_rfc3339()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn live_status(&self, account: i64) -> Result<LiveStatus, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let status: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT message,updated_at FROM sim_live_accounts WHERE account_id=?1",
                [account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT plan_json FROM sim_live_plans WHERE account_id=?1 ORDER BY symbol")
            .map_err(|e| e.to_string())?;
        let plans = stmt
            .query_map([account], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .map(|r| {
                serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut stmt=conn.prepare("SELECT l.evidence_json FROM sim_live_orders l JOIN sim_orders o ON o.id=l.order_id WHERE o.account_id=?1 AND l.evidence_json IS NOT NULL ORDER BY o.id DESC LIMIT 20").map_err(|e|e.to_string())?;
        let executions = stmt
            .query_map([account], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .map(|r| {
                serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let marks = conn
            .query_row(
                "SELECT COUNT(*) FROM sim_live_equity WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(LiveStatus {
            engine: if status.is_some() {
                "realtime_a_share"
            } else {
                "legacy_daily"
            }
            .into(),
            message: status.as_ref().map(|s| s.0.clone()).unwrap_or(
                "日线前向账户：按收盘信号与下一交易日开盘价推进，不代表盘中实际触价成交".into(),
            ),
            updated_at: status.and_then(|s| s.1),
            plans,
            executions,
            marks,
        })
    }
    pub fn record_live_equity(
        &self,
        account: i64,
        ticks: &[crate::simulation_live::LiveTick],
        benchmark: Option<i64>,
    ) -> Result<(), String> {
        if ticks.is_empty() {
            return Err("无有效实时行情，净值不更新".into());
        }
        crate::datasource::a_share_calendar::continuous(chrono::Utc::now())?;
        let detail = self.get_sim_detail(account)?;
        let mut value = detail
            .account
            .current_cash
            .parse::<i64>()
            .map_err(|_| "资金无效")?;
        let now = chrono::Utc::now();
        for tick in ticks {
            crate::simulation_live::validate(tick, now)?;
        }
        for position in &detail.positions {
            let tick = ticks
                .iter()
                .find(|t| t.quote.code == position.symbol)
                .ok_or("持仓缺少实时行情，本轮净值未更新")?;
            crate::simulation_live::validate(tick, now)?;
            value = value
                .checked_add(
                    crate::simulation_live::scaled(tick.quote.price)?
                        .checked_mul(position.quantity)
                        .ok_or("市值超限")?,
                )
                .ok_or("权益超限")?;
        }
        let date = now.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
        let minute = date.format("%Y-%m-%d %H:%M").to_string();
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let initial = detail
            .account
            .initial_cash
            .parse::<i64>()
            .map_err(|_| "初始资金无效")?;
        conn.execute("INSERT OR IGNORE INTO sim_live_risk(account_id,peak,max_drawdown_bps,first_benchmark,last_benchmark) VALUES(?1,?2,0,?3,?3)",params![account,initial,benchmark]).map_err(|e|e.to_string())?;
        let peak: i64 = conn
            .query_row(
                "SELECT peak FROM sim_live_risk WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let peak = peak.max(value);
        let drawdown = if peak > 0 {
            ((peak - value) as i128 * 10000 / peak as i128) as i64
        } else {
            0
        };
        conn.execute("UPDATE sim_live_risk SET peak=?2,max_drawdown_bps=MAX(max_drawdown_bps,?3),last_benchmark=?4 WHERE account_id=?1",params![account,peak,drawdown,benchmark]).map_err(|e|e.to_string())?;
        conn.execute("INSERT INTO sim_live_equity(account_id,minute,equity,low_equity,high_equity) VALUES(?1,?2,?3,?3,?3) ON CONFLICT(account_id,minute) DO UPDATE SET equity=excluded.equity,low_equity=MIN(low_equity,excluded.equity),high_equity=MAX(high_equity,excluded.equity)",params![account,minute,value]).map_err(|e|e.to_string())?;
        conn.execute("INSERT INTO sim_equity_daily(account_id,trade_date,equity) VALUES(?1,?2,?3) ON CONFLICT(account_id,trade_date) DO UPDATE SET equity=excluded.equity",params![account,date.format("%Y-%m-%d").to_string(),value]).map_err(|e|e.to_string())?;
        for tick in ticks {
            conn.execute("INSERT INTO sim_live_marks(account_id,symbol,price,timestamp,source) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account_id,symbol) DO UPDATE SET price=excluded.price,timestamp=excluded.timestamp,source=excluded.source",params![account,tick.quote.code,crate::simulation_live::scaled(tick.quote.price)?,tick.quote.timestamp,tick.source]).map_err(|e|e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::simulation::{AccountInput, OrderInput};
    use chrono::TimeZone;
    #[test]
    fn live_ledger_does_not_sell_today_then_sells_next_day_and_persists_evidence() {
        let db = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate().unwrap();
        db.migrate_simulation().unwrap();
        db.migrate_simulation_live().unwrap();
        db.migrate_strategies().unwrap();
        db.migrate_research_loop().unwrap();
        let account = db
            .save_sim_account(&AccountInput {
                id: None,
                name: "实时测试".into(),
                initial_cash: "1000000000".into(),
                mode: "auto".into(),
                auto_enabled: true,
                manual_source_enabled: true,
                rule_source_enabled: false,
                ai_source_enabled: false,
                commission_bps: 0,
                min_commission: "0".into(),
                stamp_tax_bps: 0,
                transfer_fee_bps: 0,
                slippage_bps: 0,
                targets: vec![],
            })
            .unwrap();
        db.enable_live_account(account.id, &[]).unwrap();
        let day = chrono::Utc.with_ymd_and_hms(2026, 9, 21, 2, 0, 0).unwrap();
        let tick =
            |price: f64, now: chrono::DateTime<chrono::Utc>| crate::simulation_live::LiveTick {
                quote: crate::domain::Quote {
                    code: "sh600000".into(),
                    market: "CN".into(),
                    name: "A股测试".into(),
                    price,
                    prev_close: 34.0,
                    change: 0.0,
                    change_pct: 0.0,
                    open: 34.0,
                    high: 35.0,
                    low: 33.0,
                    volume: 10000,
                    turnover: 100000.0,
                    turnover_rate: None,
                    timestamp: now.timestamp(),
                },
                depth: crate::domain::Depth {
                    code: "sh600000".into(),
                    bids: vec![crate::domain::Level {
                        price,
                        volume: 1000,
                    }],
                    asks: vec![crate::domain::Level {
                        price,
                        volume: 1000,
                    }],
                    timestamp: now.timestamp(),
                },
                source: "synthetic".into(),
                received_at: now.timestamp(),
            };
        let input = |key: &str, side: &str, date: &str| OrderInput {
            account_id: account.id,
            idempotency_key: key.into(),
            symbol: "sh600000".into(),
            name: "测试".into(),
            side: side.into(),
            quantity: 100,
            signal_date: date.into(),
            source: "manual".into(),
            rule: None,
            stop_bps: 0,
            take_bps: 0,
            limit_bps: 1000,
            max_hold_days: 0,
            ai_generated: false,
        };
        let buy = db
            .submit_sim_order(&input("buy", "buy", "2026-09-21"))
            .unwrap();
        db.live_order(buy.id, 340000, day.timestamp() - 1).unwrap();
        let buytick = tick(34.0, day);
        assert_eq!(
            db.match_sim_order_live_at(buy.id, &buytick, day)
                .unwrap()
                .status,
            "filled"
        );
        assert_eq!(
            db.match_sim_order_live_at(buy.id, &buytick, day)
                .unwrap()
                .status,
            "filled"
        );
        let sell = db
            .submit_sim_order(&input("sell", "sell", "2026-09-21"))
            .unwrap();
        db.live_order(sell.id, 350000, day.timestamp() - 1).unwrap();
        let fail = db
            .match_sim_order_live_at(
                sell.id,
                &tick(35.0, day + chrono::Duration::seconds(2)),
                day + chrono::Duration::seconds(2),
            )
            .unwrap();
        assert_eq!(fail.status, "pending");
        assert!(fail.reject_reason.unwrap().contains("T+1"));
        db.expire_live_orders(account.id, "2026-09-22").unwrap();
        let sell = db
            .submit_sim_order(&input("sell-next", "sell", "2026-09-22"))
            .unwrap();
        let next = day + chrono::Duration::days(1);
        db.live_order(sell.id, 350000, next.timestamp() - 1)
            .unwrap();
        assert_eq!(
            db.match_sim_order_live_at(sell.id, &tick(35.0, next), next)
                .unwrap()
                .status,
            "filled"
        );
        let d = db.get_sim_detail(account.id).unwrap();
        assert_eq!(d.metrics.sample_count, 1);
        assert_eq!(d.metrics.realized_profit, "1000000");
        assert_eq!(db.live_status(account.id).unwrap().executions.len(), 2);
        assert!(d.positions.is_empty());
    }
}
