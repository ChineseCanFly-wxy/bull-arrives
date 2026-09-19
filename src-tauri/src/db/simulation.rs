use super::Database;
use crate::simulation::{self, FeeConfig, MatchRequest, RawBar, Side};
use rusqlite::{params, OptionalExtension, Result as SqliteResult, Row, Transaction};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimAccount {
    pub id: i64,
    pub name: String,
    pub initial_cash: String,
    pub current_cash: String,
    pub mode: String,
    pub auto_enabled: bool,
    pub manual_source_enabled: bool,
    pub rule_source_enabled: bool,
    pub ai_source_enabled: bool,
    pub commission_bps: i64,
    pub min_commission: String,
    pub stamp_tax_bps: i64,
    pub transfer_fee_bps: i64,
    pub slippage_bps: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInput {
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    pub initial_cash: String,
    pub mode: String,
    #[serde(default)]
    pub auto_enabled: bool,
    #[serde(default = "yes")]
    pub manual_source_enabled: bool,
    #[serde(default = "yes")]
    pub rule_source_enabled: bool,
    #[serde(default)]
    pub ai_source_enabled: bool,
    #[serde(default = "default_commission")]
    pub commission_bps: i64,
    #[serde(default = "default_min_commission")]
    pub min_commission: String,
    #[serde(default = "default_stamp_tax")]
    pub stamp_tax_bps: i64,
    #[serde(default = "default_transfer_fee")]
    pub transfer_fee_bps: i64,
    #[serde(default)]
    pub slippage_bps: i64,
    #[serde(default)]
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub account_id: i64,
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub rule: String,
    #[serde(default = "default_limit_bps")]
    pub limit_bps: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderInput {
    pub account_id: i64,
    pub idempotency_key: String,
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    pub side: String,
    pub quantity: i64,
    pub signal_date: String,
    pub source: String,
    #[serde(default)]
    pub rule: Option<String>,
    #[serde(default)]
    pub stop_bps: i64,
    #[serde(default)]
    pub take_bps: i64,
    #[serde(default = "default_limit_bps")]
    pub limit_bps: i64,
    #[serde(default)]
    pub max_hold_days: i64,
    #[serde(skip)]
    pub ai_generated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub lot_id: i64,
    pub account_id: i64,
    pub symbol: String,
    pub name: String,
    pub quantity: i64,
    pub available_quantity: i64,
    pub cost_price: String,
    pub acquired_date: String,
    pub stop_bps: i64,
    pub take_bps: i64,
    pub max_hold_days: i64,
    pub limit_bps: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimOrder {
    pub id: i64,
    pub account_id: i64,
    pub idempotency_key: String,
    pub symbol: String,
    pub name: String,
    pub side: String,
    pub quantity: i64,
    pub signal_date: String,
    pub source: String,
    pub rule: Option<String>,
    pub stop_price: Option<String>,
    pub take_price: Option<String>,
    pub stop_bps: i64,
    pub take_bps: i64,
    pub limit_bps: i64,
    pub max_hold_days: i64,
    pub status: String,
    pub price: Option<String>,
    pub gross: Option<String>,
    pub fee: Option<String>,
    pub reject_reason: Option<String>,
    pub created_at: String,
    pub confirmed_at: Option<String>,
    pub filled_at: Option<String>,
    pub holding_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimRun {
    pub id: i64,
    pub account_id: i64,
    pub run_key: String,
    pub status: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub message: Option<String>,
    pub progress: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimMetrics {
    pub equity: String,
    pub realized_profit: String,
    pub filled_sell_count: i64,
    pub winning_sell_count: i64,
    pub total_return_bps: i64,
    pub win_rate_bps: i64,
    pub max_drawdown_bps: i64,
    pub profit_loss_ratio_bps: i64,
    pub expectancy: String,
    pub annualized_return_bps: i64,
    pub average_holding_days_x100: i64,
    pub sample_count: i64,
    pub benchmark_return_bps: Option<i64>,
    pub excess_return_bps: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStats {
    pub source: String,
    pub orders: i64,
    pub filled: i64,
    pub rejected: i64,
    pub realized_profit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimDetail {
    pub account: SimAccount,
    pub targets: Vec<Target>,
    pub positions: Vec<Position>,
    pub orders: Vec<SimOrder>,
    pub recent_runs: Vec<SimRun>,
    pub metrics: SimMetrics,
    pub source_stats: Vec<SourceStats>,
}

fn yes() -> bool {
    true
}
fn default_commission() -> i64 {
    3
}
fn default_min_commission() -> String {
    "50000".into()
}
fn default_stamp_tax() -> i64 {
    5
}
fn default_transfer_fee() -> i64 {
    1
}
fn default_limit_bps() -> i64 {
    1_000
}
fn now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}
fn invalid(message: &str) -> String {
    message.to_string()
}

impl Database {
    pub fn migrate_simulation(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS sim_accounts (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL,
                 initial_cash INTEGER NOT NULL CHECK(initial_cash >= 0),
                 cash INTEGER NOT NULL CHECK(cash >= 0),
                 mode TEXT NOT NULL CHECK(mode IN ('record','confirm','auto')),
                 auto_enabled INTEGER NOT NULL DEFAULT 0 CHECK(auto_enabled IN (0,1)),
                 manual_source_enabled INTEGER NOT NULL DEFAULT 1,
                 rule_source_enabled INTEGER NOT NULL DEFAULT 1,
                 ai_source_enabled INTEGER NOT NULL DEFAULT 0,
                 commission_bps INTEGER NOT NULL DEFAULT 3,
                 min_commission INTEGER NOT NULL DEFAULT 50000,
                 stamp_tax_bps INTEGER NOT NULL DEFAULT 5,
                 transfer_fee_bps INTEGER NOT NULL DEFAULT 1,
                 slippage_bps INTEGER NOT NULL DEFAULT 0,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sim_targets (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 account_id INTEGER NOT NULL REFERENCES sim_accounts(id) ON DELETE CASCADE,
                 symbol TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', rule TEXT NOT NULL,
                 limit_bps INTEGER NOT NULL DEFAULT 1000,
                 UNIQUE(account_id, symbol, rule)
             );
             CREATE TABLE IF NOT EXISTS sim_orders (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 account_id INTEGER NOT NULL REFERENCES sim_accounts(id) ON DELETE CASCADE,
                 idempotency_key TEXT NOT NULL,
                 symbol TEXT NOT NULL, name TEXT NOT NULL DEFAULT '',
                 side TEXT NOT NULL CHECK(side IN ('buy','sell')),
                 quantity INTEGER NOT NULL CHECK(quantity > 0), signal_date TEXT NOT NULL,
                 source TEXT NOT NULL, rule TEXT, stop_price INTEGER, take_price INTEGER,
                 stop_bps INTEGER NOT NULL DEFAULT 0, take_bps INTEGER NOT NULL DEFAULT 0,
                 limit_bps INTEGER NOT NULL, max_hold_days INTEGER NOT NULL DEFAULT 0,
                 status TEXT NOT NULL, price INTEGER, gross INTEGER, fee INTEGER,
                 cash_delta INTEGER, cost_basis INTEGER, reject_reason TEXT,
                 created_at TEXT NOT NULL, confirmed_at TEXT, filled_at TEXT, holding_days INTEGER,
                 UNIQUE(account_id, idempotency_key)
             );
             CREATE TABLE IF NOT EXISTS sim_lots (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 account_id INTEGER NOT NULL REFERENCES sim_accounts(id) ON DELETE CASCADE,
                 symbol TEXT NOT NULL, name TEXT NOT NULL DEFAULT '',
                 quantity INTEGER NOT NULL CHECK(quantity > 0), unit_cost INTEGER NOT NULL CHECK(unit_cost >= 0),
                 cost_basis INTEGER NOT NULL CHECK(cost_basis >= 0),
                 acquired_date TEXT NOT NULL, stop_bps INTEGER NOT NULL DEFAULT 0,
                 take_bps INTEGER NOT NULL DEFAULT 0, max_hold_days INTEGER NOT NULL DEFAULT 0,
                 limit_bps INTEGER NOT NULL DEFAULT 1000
             );
             CREATE INDEX IF NOT EXISTS idx_sim_lots_position ON sim_lots(account_id, symbol, acquired_date, id);
             CREATE TABLE IF NOT EXISTS sim_runs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 account_id INTEGER NOT NULL REFERENCES sim_accounts(id) ON DELETE CASCADE,
                 run_key TEXT NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL,
                 finished_at TEXT, message TEXT, progress INTEGER NOT NULL DEFAULT 0, UNIQUE(account_id, run_key)
             );
             CREATE TABLE IF NOT EXISTS sim_equity_daily (
                 account_id INTEGER NOT NULL REFERENCES sim_accounts(id) ON DELETE CASCADE,
                 trade_date TEXT NOT NULL, equity INTEGER NOT NULL CHECK(equity >= 0), benchmark_close INTEGER,
                 PRIMARY KEY(account_id, trade_date)
             );
             UPDATE sim_runs SET status='failed',finished_at=datetime('now'),message='应用重启后自动恢复' WHERE status='running';",
        )?;
        for (table, column, definition) in [
            ("sim_lots", "limit_bps", "INTEGER NOT NULL DEFAULT 1000"),
            ("sim_orders", "holding_days", "INTEGER"),
            ("sim_equity_daily", "benchmark_close", "INTEGER"),
            ("sim_runs", "progress", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<SqliteResult<Vec<_>>>()?;
            drop(statement);
            if !columns.iter().any(|name| name == column) {
                conn.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    pub fn list_sim_accounts(&self) -> SqliteResult<Vec<SimAccount>> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn.prepare(
            "SELECT id,name,initial_cash,cash,mode,auto_enabled,manual_source_enabled,rule_source_enabled,ai_source_enabled,commission_bps,min_commission,stamp_tax_bps,transfer_fee_bps,slippage_bps,created_at,updated_at FROM sim_accounts ORDER BY id",
        )?;
        let rows = statement.query_map([], account_from_row)?.collect();
        rows
    }

    pub fn save_sim_account(&self, input: &AccountInput) -> Result<SimAccount, String> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(invalid("账户名称不能为空"));
        }
        if !matches!(input.mode.as_str(), "record" | "confirm" | "auto") {
            return Err(invalid("账户模式只能为 record / confirm / auto"));
        }
        let initial_cash = simulation::parse_scaled(&input.initial_cash, "初始资金")?;
        let min_commission = simulation::parse_scaled(&input.min_commission, "最低佣金")?;
        for (name, value) in [
            ("佣金", input.commission_bps),
            ("印花税", input.stamp_tax_bps),
            ("过户费", input.transfer_fee_bps),
            ("滑点", input.slippage_bps),
        ] {
            if !(0..=1_000).contains(&value) {
                return Err(format!("{name}应在 0~1000 基点之间"));
            }
        }
        if input.auto_enabled && input.ai_source_enabled {
            return Err(invalid("AI 来源不得启用自动成交"));
        }
        for target in &input.targets {
            if target.symbol.trim().is_empty()
                || target.rule.trim().is_empty()
                || !(1..=3_000).contains(&target.limit_bps)
            {
                return Err(invalid(
                    "标的、规则不能为空，涨跌幅限制应在 1~3000 基点之间",
                ));
            }
        }
        let timestamp = now();
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let account_id = if let Some(id) = input.id {
            let changed = tx.execute(
                "UPDATE sim_accounts SET name=?1,initial_cash=?2,mode=?3,auto_enabled=?4,manual_source_enabled=?5,rule_source_enabled=?6,ai_source_enabled=?7,commission_bps=?8,min_commission=?9,stamp_tax_bps=?10,transfer_fee_bps=?11,slippage_bps=?12,updated_at=?13 WHERE id=?14",
                params![name,initial_cash,input.mode,i64::from(input.auto_enabled),i64::from(input.manual_source_enabled),i64::from(input.rule_source_enabled),i64::from(input.ai_source_enabled),input.commission_bps,min_commission,input.stamp_tax_bps,input.transfer_fee_bps,input.slippage_bps,timestamp,id],
            ).map_err(|error| error.to_string())?;
            if changed == 0 {
                return Err(invalid("模拟账户不存在"));
            }
            id
        } else {
            tx.execute(
                "INSERT INTO sim_accounts(name,initial_cash,cash,mode,auto_enabled,manual_source_enabled,rule_source_enabled,ai_source_enabled,commission_bps,min_commission,stamp_tax_bps,transfer_fee_bps,slippage_bps,created_at,updated_at) VALUES(?1,?2,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
                params![name,initial_cash,input.mode,i64::from(input.auto_enabled),i64::from(input.manual_source_enabled),i64::from(input.rule_source_enabled),i64::from(input.ai_source_enabled),input.commission_bps,min_commission,input.stamp_tax_bps,input.transfer_fee_bps,input.slippage_bps,timestamp],
            ).map_err(|error| error.to_string())?;
            tx.last_insert_rowid()
        };
        tx.execute("DELETE FROM sim_targets WHERE account_id=?1", [account_id])
            .map_err(|error| error.to_string())?;
        for target in &input.targets {
            // 标的的涨跌幅按板块规则落库；规则无法判定时保留用户配置（例如历史遗留标的）。
            let limit_bps = crate::market_rules::ensure_simulatable(target.symbol.trim(), &target.name)
                .unwrap_or(target.limit_bps);
            tx.execute(
                "INSERT INTO sim_targets(account_id,symbol,name,rule,limit_bps) VALUES(?1,?2,?3,?4,?5)",
                params![account_id,target.symbol.trim(),target.name,target.rule,limit_bps],
            ).map_err(|error| error.to_string())?;
        }
        let account = tx.query_row(
            "SELECT id,name,initial_cash,cash,mode,auto_enabled,manual_source_enabled,rule_source_enabled,ai_source_enabled,commission_bps,min_commission,stamp_tax_bps,transfer_fee_bps,slippage_bps,created_at,updated_at FROM sim_accounts WHERE id=?1",
            [account_id], account_from_row,
        ).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(account)
    }

    pub fn delete_sim_account(&self, account_id: i64) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute("DELETE FROM sim_accounts WHERE id=?1", [account_id])?;
        Ok(())
    }

    pub fn list_auto_sim_account_ids(&self) -> SqliteResult<Vec<i64>> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn.prepare(
            "SELECT id FROM sim_accounts WHERE mode='auto' AND auto_enabled=1 ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?.collect();
        rows
    }

    pub fn submit_sim_order(&self, input: &OrderInput) -> Result<SimOrder, String> {
        let side = Side::parse(&input.side)?;
        if input.idempotency_key.trim().is_empty()
            || input.symbol.trim().is_empty()
            || input.signal_date.trim().is_empty()
            || input.quantity <= 0
        {
            return Err(invalid("委托编号、代码、信号日期不能为空，数量必须大于 0"));
        }
        if !matches!(input.source.as_str(), "manual" | "rule" | "risk" | "ai") {
            return Err(invalid("委托来源无效"));
        }
        chrono::NaiveDate::parse_from_str(&input.signal_date, "%Y-%m-%d")
            .map_err(|_| invalid("信号日期应为 YYYY-MM-DD"))?;
        if !(1..=3_000).contains(&input.limit_bps) {
            return Err(invalid("涨跌幅限制应在 1~3000 基点之间"));
        }
        // 涨跌幅与申报数量不按用户配置执行：一律按代码所属板块的交易所规则重算并落库，
        // 这样历史委托、实时委托与后续撮合看到的是同一个数字。
        let limit_bps = crate::market_rules::ensure_simulatable(input.symbol.trim(), &input.name)?;
        if limit_bps != input.limit_bps {
            log::warn!(
                "[simulation] {} 涨跌幅配置 {} 与板块规则 {} 不符，已按交易所规则记录",
                input.symbol,
                input.limit_bps,
                limit_bps
            );
        }
        if side == Side::Buy {
            crate::market_rules::validate_buy_quantity(input.symbol.trim(), input.quantity)?;
        }
        if input.stop_bps < 0 || input.take_bps < 0 || input.max_hold_days < 0 {
            return Err(invalid("止损、止盈和最大持有天数不能为负数"));
        }
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let (mode, auto_enabled, source_enabled): (String, bool, bool) = conn.query_row(
            "SELECT mode,auto_enabled,CASE ?2 WHEN 'manual' THEN manual_source_enabled WHEN 'rule' THEN rule_source_enabled WHEN 'risk' THEN 1 ELSE ai_source_enabled END FROM sim_accounts WHERE id=?1", params![input.account_id,input.source],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0, row.get::<_, i64>(2)? != 0)),
        ).optional().map_err(|error| error.to_string())?.ok_or_else(|| invalid("模拟账户不存在"))?;
        if !source_enabled {
            return Err(invalid("该委托来源已禁用"));
        }
        super::research_loop::guard_execution(&conn, input.account_id)?;
        if (input.ai_generated || input.source == "ai") && auto_enabled {
            return Err(invalid("AI 委托不得启用自动成交"));
        }
        let status = match mode.as_str() {
            "record" => "recorded",
            "confirm" => "awaiting_confirmation",
            "auto" if auto_enabled => "pending",
            "auto" => "awaiting_confirmation",
            _ => return Err(invalid("账户模式无效")),
        };
        let timestamp = now();
        conn.execute(
            "INSERT INTO sim_orders(account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,rule,stop_price,take_price,stop_bps,take_bps,limit_bps,max_hold_days,status,created_at,confirmed_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,CASE WHEN ?16='pending' THEN ?17 END)
             ON CONFLICT(account_id,idempotency_key) DO NOTHING",
            params![input.account_id,input.idempotency_key.trim(),input.symbol.trim(),input.name,side.as_str(),input.quantity,input.signal_date,input.source,input.rule,Option::<i64>::None,Option::<i64>::None,input.stop_bps,input.take_bps,limit_bps,input.max_hold_days,status,timestamp],
        ).map_err(|error| error.to_string())?;
        conn.query_row(
            "SELECT id,account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,rule,stop_price,take_price,stop_bps,take_bps,limit_bps,max_hold_days,status,price,gross,fee,reject_reason,created_at,confirmed_at,filled_at,holding_days
             FROM sim_orders WHERE account_id=?1 AND idempotency_key=?2",
            params![input.account_id,input.idempotency_key.trim()], order_from_row,
        ).map_err(|error| error.to_string())
    }

    pub fn confirm_sim_order(&self, order_id: i64) -> Result<SimOrder, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute(
            "UPDATE sim_orders SET status='pending',confirmed_at=?1 WHERE id=?2 AND status='awaiting_confirmation'",
            params![now(), order_id],
        ).map_err(|error| error.to_string())?;
        conn.query_row(
            "SELECT id,account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,rule,stop_price,take_price,stop_bps,take_bps,limit_bps,max_hold_days,status,price,gross,fee,reject_reason,created_at,confirmed_at,filled_at,holding_days FROM sim_orders WHERE id=?1",
            [order_id], order_from_row,
        ).optional().map_err(|error| error.to_string())?.ok_or_else(|| invalid("模拟委托不存在"))
    }

    pub fn match_sim_order_raw(&self, order_id: i64, bar: &RawBar) -> Result<SimOrder, String> {
        self.match_sim_order_internal(order_id, bar, None, chrono::Utc::now())
    }
    pub fn match_sim_order_live(
        &self,
        order_id: i64,
        tick: &crate::simulation_live::LiveTick,
    ) -> Result<SimOrder, String> {
        self.match_sim_order_live_at(order_id, tick, chrono::Utc::now())
    }
    pub(crate) fn match_sim_order_live_at(
        &self,
        order_id: i64,
        tick: &crate::simulation_live::LiveTick,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<SimOrder, String> {
        let date = chrono::DateTime::from_timestamp(tick.quote.timestamp, 0)
            .ok_or("报价时间无效")?
            .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        // Only the shared ledger uses this date carrier; live prices never enter the OHLC matcher.
        let date_carrier = RawBar {
            date,
            open: "0".into(),
            high: "0".into(),
            low: "0".into(),
            close: "0".into(),
            prev_close: "0".into(),
            volume: 0,
        };
        self.match_sim_order_internal(order_id, &date_carrier, Some(tick), now)
    }
    fn match_sim_order_internal(
        &self,
        order_id: i64,
        bar: &RawBar,
        live: Option<&crate::simulation_live::LiveTick>,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<SimOrder, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let stored = conn.query_row(
            "SELECT o.account_id,o.symbol,o.name,o.side,o.quantity,o.signal_date,o.status,a.cash,o.limit_bps,
                    a.commission_bps,a.min_commission,a.stamp_tax_bps,a.transfer_fee_bps,a.slippage_bps,
                    o.stop_price,o.take_price,o.max_hold_days,o.stop_bps,o.take_bps
             FROM sim_orders o JOIN sim_accounts a ON a.id=o.account_id WHERE o.id=?1",
            [order_id], |row| Ok((row.get::<_, i64>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, i64>(4)?,row.get::<_, String>(5)?,row.get::<_, String>(6)?,row.get::<_, i64>(7)?,row.get::<_, i64>(8)?,row.get::<_, i64>(9)?,row.get::<_, i64>(10)?,row.get::<_, i64>(11)?,row.get::<_, i64>(12)?,row.get::<_, i64>(13)?,row.get::<_, Option<i64>>(14)?,row.get::<_, Option<i64>>(15)?,row.get::<_, i64>(16)?,row.get::<_, i64>(17)?,row.get::<_, i64>(18)?)),
        ).optional().map_err(|error| error.to_string())?.ok_or_else(|| invalid("模拟委托不存在"))?;
        if stored.6 == "filled" {
            return load_order(&conn, order_id).map_err(|error| error.to_string());
        }
        super::research_loop::guard_execution(&conn, stored.0)?;
        if stored.6 != "pending" {
            return Err(invalid("委托尚未确认或不可成交"));
        }
        let side = Side::parse(&stored.3)?;
        let available_quantity = if side == Side::Sell {
            conn.query_row(
                "SELECT COALESCE(SUM(quantity),0) FROM sim_lots WHERE account_id=?1 AND symbol=?2 AND acquired_date<?3",
                params![stored.0,stored.1,bar.date], |row| row.get(0),
            ).map_err(|error| error.to_string())?
        } else {
            0
        };
        // 持仓总量用于「卖出的不能超过持仓 / 零股必须一次性卖出」；
        // T+1 可用数量单独算，两者都交给撮合层判定后落成 rejected，而不是直接报错。
        let position_quantity: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(quantity),0) FROM sim_lots WHERE account_id=?1 AND symbol=?2",
                params![stored.0, stored.1],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let request = MatchRequest {
            symbol: stored.1.clone(),
            name: stored.2.clone(),
            side,
            quantity: stored.4,
            signal_date: stored.5.clone(),
            available_quantity,
            position_quantity,
            cash: stored.7,
            limit_bps: stored.8,
            fees: FeeConfig {
                commission_bps: stored.9,
                min_commission: stored.10,
                stamp_tax_bps: stored.11,
                transfer_fee_bps: stored.12,
                slippage_bps: stored.13,
            },
        };
        let result = if let Some(tick) = live {
            if stored.5 != bar.date {
                return Err("当日委托已过期，不以之后日期补成交".into());
            }
            let consumed: Option<i64> = conn
                .query_row(
                    "SELECT timestamp FROM sim_live_consumed WHERE account_id=?1 AND symbol=?2",
                    params![stored.0, stored.1],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if consumed.is_some_and(|stamp| stamp >= tick.depth.timestamp) {
                return Err("该盘口时点已用于成交，等待更新盘口，避免重复使用同一挂单量".into());
            }
            if tick.quote.code != stored.1 {
                return Err("行情与委托标的不一致".into());
            }
            let (limit, submitted): (i64, i64) = conn
                .query_row(
                    "SELECT limit_price,submitted_at FROM sim_live_orders WHERE order_id=?1",
                    [order_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| "历史委托不能通过实时撮合补成交")?;
            crate::simulation_live::quote_fill(
                tick,
                side,
                stored.4,
                limit,
                submitted,
                available_quantity,
                stored.7,
                request.fees,
                // 历史委托可能带着旧配置，这里按板块规则重算，避免老账户永远无法成交。
                crate::market_rules::ensure_simulatable(&stored.1, &tick.quote.name)?,
                observed_at,
            )
        } else {
            let has_live:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sim_live_accounts')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            if has_live
                && conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sim_live_accounts WHERE account_id=?1)",
                        [stored.0],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(|e| e.to_string())?
            {
                return Err("实时账户不能调用日线撮合".into());
            }
            simulation::match_raw_bar(&request, bar)
        };
        let quote = match result {
            Ok(quote) => quote,
            Err(reason) => {
                if live.is_some() {
                    conn.execute(
                        "UPDATE sim_orders SET reject_reason=?1 WHERE id=?2 AND status='pending'",
                        params![reason, order_id],
                    )
                    .map_err(|e| e.to_string())?;
                    return load_order(&conn, order_id).map_err(|e| e.to_string());
                }
                conn.execute("UPDATE sim_orders SET status='rejected',reject_reason=?1 WHERE id=?2 AND status='pending'",params![reason,order_id]).map_err(|e|e.to_string())?;
                return load_order(&conn, order_id).map_err(|e| e.to_string());
            }
        };
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        if let Some(tick) = live {
            tx.execute("INSERT INTO sim_live_consumed(account_id,symbol,timestamp) VALUES(?1,?2,?3) ON CONFLICT(account_id,symbol) DO UPDATE SET timestamp=excluded.timestamp",params![stored.0,stored.1,tick.depth.timestamp]).map_err(|e|e.to_string())?;
            let evidence = serde_json::json!({"order_id":order_id,"symbol":stored.1,"side":stored.3,"filled_price":quote.price.to_string(),"quantity":stored.4,"fee":quote.fee.to_string(),"tick":tick,"method":"best_visible_level_with_slippage_whole_order","filled_at":chrono::Utc::now().to_rfc3339()});
            tx.execute(
                "UPDATE sim_live_orders SET evidence_json=?1 WHERE order_id=?2",
                params![evidence.to_string(), order_id],
            )
            .map_err(|e| e.to_string())?;
        }
        let (cost_basis, holding_days) = if side == Side::Buy {
            let total_cost = quote
                .gross
                .checked_add(quote.fee)
                .ok_or_else(|| invalid("成本超出安全范围"))?;
            let unit_cost = total_cost
                .checked_add(stored.4 - 1)
                .ok_or_else(|| invalid("成本超出安全范围"))?
                / stored.4;
            let stop_bps = if stored.17 != 0 {
                stored.17
            } else {
                stored
                    .14
                    .map(|p| (quote.price - p).saturating_mul(10_000) / quote.price)
                    .unwrap_or(0)
            };
            let take_bps = if stored.18 != 0 {
                stored.18
            } else {
                stored
                    .15
                    .map(|p| (p - quote.price).saturating_mul(10_000) / quote.price)
                    .unwrap_or(0)
            };
            tx.execute(
                "INSERT INTO sim_lots(account_id,symbol,name,quantity,unit_cost,cost_basis,acquired_date,stop_bps,take_bps,max_hold_days,limit_bps) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![stored.0,stored.1,stored.2,stored.4,unit_cost,total_cost,bar.date,stop_bps,take_bps,stored.16,stored.8],
            ).map_err(|error| error.to_string())?;
            (0, None)
        } else {
            let (cost, days) = consume_lots(&tx, stored.0, &stored.1, &bar.date, stored.4)?;
            (cost, Some(days))
        };
        let changed = tx
            .execute(
                "UPDATE sim_accounts SET cash=cash+?1,updated_at=?2 WHERE id=?3 AND cash+?1>=0",
                params![quote.cash_delta, now(), stored.0],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err(invalid("可用资金不足"));
        }
        let changed = tx.execute(
            "UPDATE sim_orders SET status='filled',reject_reason=NULL,price=?1,gross=?2,fee=?3,cash_delta=?4,cost_basis=?5,filled_at=?6,holding_days=?7
             WHERE id=?8 AND status='pending'",
            params![quote.price, quote.gross, quote.fee, quote.cash_delta, cost_basis, now(), holding_days, order_id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err(invalid("委托已被其他流程处理"));
        }
        let order = load_order(&tx, order_id).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(order)
    }

    pub fn get_sim_detail(&self, account_id: i64) -> Result<SimDetail, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let account = conn.query_row(
            "SELECT id,name,initial_cash,cash,mode,auto_enabled,manual_source_enabled,rule_source_enabled,ai_source_enabled,commission_bps,min_commission,stamp_tax_bps,transfer_fee_bps,slippage_bps,created_at,updated_at FROM sim_accounts WHERE id=?1",
            [account_id], account_from_row,
        ).optional().map_err(|error| error.to_string())?.ok_or_else(|| invalid("模拟账户不存在"))?;
        let targets = collect(&conn, "SELECT id,account_id,symbol,name,rule,limit_bps FROM sim_targets WHERE account_id=?1 ORDER BY id", account_id, target_from_row)?;
        let orders = collect(&conn, "SELECT id,account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,rule,stop_price,take_price,stop_bps,take_bps,limit_bps,max_hold_days,status,price,gross,fee,reject_reason,created_at,confirmed_at,filled_at,holding_days FROM sim_orders WHERE account_id=?1 ORDER BY id DESC", account_id, order_from_row)?;
        let recent_runs = collect(&conn, "SELECT id,account_id,run_key,status,created_at,finished_at,message,progress FROM sim_runs WHERE account_id=?1 ORDER BY id DESC LIMIT 50", account_id, run_from_row)?;
        let mut source_statement = conn.prepare(
            "SELECT source,COUNT(*),SUM(CASE WHEN status='filled' THEN 1 ELSE 0 END),SUM(CASE WHEN status='rejected' THEN 1 ELSE 0 END),COALESCE(SUM(CASE WHEN side='sell' AND status='filled' THEN cash_delta-cost_basis ELSE 0 END),0) FROM sim_orders WHERE account_id=?1 GROUP BY source ORDER BY source",
        ).map_err(|error| error.to_string())?;
        let source_stats = source_statement
            .query_map([account_id], |row| {
                Ok(SourceStats {
                    source: row.get(0)?,
                    orders: row.get(1)?,
                    filled: row.get(2)?,
                    rejected: row.get(3)?,
                    realized_profit: row.get::<_, i64>(4)?.to_string(),
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<SqliteResult<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        let mut statement = conn.prepare(
            "SELECT id,account_id,symbol,name,quantity,CASE WHEN acquired_date<date('now','+8 hours') THEN quantity ELSE 0 END,cost_basis/quantity,acquired_date,stop_bps,take_bps,max_hold_days,limit_bps
             FROM sim_lots WHERE account_id=?1 ORDER BY acquired_date,id",
        ).map_err(|error| error.to_string())?;
        let positions = statement
            .query_map([account_id], |row| {
                Ok(Position {
                    lot_id: row.get(0)?,
                    account_id: row.get(1)?,
                    symbol: row.get(2)?,
                    name: row.get(3)?,
                    quantity: row.get(4)?,
                    available_quantity: row.get(5)?,
                    cost_price: row.get::<_, i64>(6)?.to_string(),
                    acquired_date: row.get(7)?,
                    stop_bps: row.get(8)?,
                    take_bps: row.get(9)?,
                    max_hold_days: row.get(10)?,
                    limit_bps: row.get(11)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<SqliteResult<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        let (realized, sells, wins, gains, losses, loss_count, holding_days): (i64, i64, i64, i64, i64, i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(cash_delta-cost_basis),0),COUNT(*),COALESCE(SUM(CASE WHEN cash_delta>cost_basis THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN cash_delta>cost_basis THEN cash_delta-cost_basis ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN cash_delta<cost_basis THEN cost_basis-cash_delta ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN cash_delta<cost_basis THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(holding_days),0)
             FROM sim_orders WHERE account_id=?1 AND status='filled' AND side='sell'",
            [account_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
        ).map_err(|error| error.to_string())?;
        let mut equity_statement = conn.prepare(
            "SELECT trade_date,equity,benchmark_close FROM sim_equity_daily WHERE account_id=?1 ORDER BY trade_date",
        ).map_err(|error| error.to_string())?;
        let equity_rows = equity_statement
            .query_map([account_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<SqliteResult<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        let equity = equity_rows
            .last()
            .map(|row| row.1)
            .unwrap_or_else(|| account.current_cash.parse().unwrap_or(0));
        let initial = account.initial_cash.parse::<i64>().unwrap_or(0);
        let total_return_bps = if initial == 0 {
            0
        } else {
            (equity - initial).saturating_mul(10_000) / initial
        };
        let mut peak = initial;
        let mut max_drawdown_bps = 0i64;
        for (_, value, _) in &equity_rows {
            peak = peak.max(*value);
            if peak > 0 {
                max_drawdown_bps = max_drawdown_bps.max((peak - value) * 10_000 / peak);
            }
        }
        let annualized_return_bps = if initial > 0 && equity_rows.len() > 1 {
            let first =
                chrono::NaiveDate::parse_from_str(&equity_rows.first().unwrap().0, "%Y-%m-%d").ok();
            let last =
                chrono::NaiveDate::parse_from_str(&equity_rows.last().unwrap().0, "%Y-%m-%d").ok();
            let days = first
                .zip(last)
                .map(|(first, last)| last.signed_duration_since(first).num_days().max(1))
                .unwrap_or(1);
            (((equity as f64 / initial as f64).powf(365.0 / days as f64) - 1.0) * 10_000.0).round()
                as i64
        } else {
            0
        };
        // Require the same endpoints as account returns, not a shorter cherry-picked interval.
        let first_benchmark = equity_rows.first().and_then(|row| row.2);
        let last_benchmark = equity_rows.last().and_then(|row| row.2);
        let mut benchmark_return_bps =
            first_benchmark
                .zip(last_benchmark)
                .and_then(|(first, last)| {
                    (first > 0).then_some((last - first).saturating_mul(10_000) / first)
                });
        let has_live:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sim_live_risk')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if has_live {
            let live:Option<(i64,Option<i64>,Option<i64>)>=conn.query_row("SELECT max_drawdown_bps,first_benchmark,last_benchmark FROM sim_live_risk WHERE account_id=?1",[account_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
            if let Some((dd, first, last)) = live {
                max_drawdown_bps = dd;
                benchmark_return_bps = first
                    .zip(last)
                    .filter(|(a, _)| *a > 0)
                    .map(|(a, b)| ((b - a) as i128 * 10000 / a as i128) as i64);
            }
        }
        let profit_loss_ratio_bps = if wins > 0 && loss_count > 0 && losses > 0 {
            (gains / wins).saturating_mul(10_000) / (losses / loss_count).max(1)
        } else {
            0
        };
        Ok(SimDetail {
            account,
            targets,
            positions,
            orders,
            recent_runs,
            source_stats,
            metrics: SimMetrics {
                equity: equity.to_string(),
                realized_profit: realized.to_string(),
                filled_sell_count: sells,
                winning_sell_count: wins,
                total_return_bps,
                win_rate_bps: if sells == 0 { 0 } else { wins * 10_000 / sells },
                max_drawdown_bps,
                profit_loss_ratio_bps,
                expectancy: if sells == 0 {
                    "0".into()
                } else {
                    (realized / sells).to_string()
                },
                annualized_return_bps,
                average_holding_days_x100: if sells == 0 {
                    0
                } else {
                    holding_days * 100 / sells
                },
                sample_count: sells,
                benchmark_return_bps,
                excess_return_bps: benchmark_return_bps.map(|value| total_return_bps - value),
            },
        })
    }

    pub fn mark_sim_equity(
        &self,
        account_id: i64,
        trade_date: &str,
        market_value: i64,
        benchmark_close: Option<i64>,
    ) -> Result<(), String> {
        if market_value < 0 || benchmark_close.is_some_and(|value| value <= 0) {
            return Err(invalid("权益快照数据无效"));
        }
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let cash: i64 = conn
            .query_row(
                "SELECT cash FROM sim_accounts WHERE id=?1",
                [account_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| invalid("模拟账户不存在"))?;
        let equity = cash
            .checked_add(market_value)
            .ok_or_else(|| invalid("权益超出安全范围"))?;
        conn.execute(
            "INSERT INTO sim_equity_daily(account_id,trade_date,equity,benchmark_close) VALUES(?1,?2,?3,?4)
             ON CONFLICT(account_id,trade_date) DO UPDATE SET equity=excluded.equity,benchmark_close=excluded.benchmark_close",
            params![account_id,trade_date,equity,benchmark_close],
        ).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn begin_sim_run(&self, account_id: i64, run_key: &str) -> Result<Option<SimRun>, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut changed = conn.execute("INSERT INTO sim_runs(account_id,run_key,status,created_at,progress) VALUES(?1,?2,'running',?3,0) ON CONFLICT(account_id,run_key) DO NOTHING",params![account_id,run_key,now()]).map_err(|e| e.to_string())?;
        if changed == 0 {
            changed = conn.execute("UPDATE sim_runs SET status='running',created_at=?3,finished_at=NULL,message=NULL,progress=0 WHERE account_id=?1 AND run_key=?2 AND status='failed'",params![account_id,run_key,now()]).map_err(|e| e.to_string())?;
            if changed == 0 {
                return Ok(None);
            }
        }
        conn.query_row("SELECT id,account_id,run_key,status,created_at,finished_at,message,progress FROM sim_runs WHERE account_id=?1 AND run_key=?2",params![account_id,run_key],run_from_row).map(Some).map_err(|e| e.to_string())
    }

    pub fn finish_sim_run(
        &self,
        run_id: i64,
        status: &str,
        message: Option<&str>,
    ) -> Result<SimRun, String> {
        if !matches!(status, "completed" | "failed") {
            return Err(invalid("运行状态无效"));
        }
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute("UPDATE sim_runs SET status=?1,finished_at=?2,message=?3,progress=CASE WHEN ?1='completed' THEN 100 ELSE progress END WHERE id=?4 AND status='running'",params![status,now(),message,run_id]).map_err(|e| e.to_string())?;
        conn.query_row("SELECT id,account_id,run_key,status,created_at,finished_at,message,progress FROM sim_runs WHERE id=?1",[run_id],run_from_row).map_err(|e| e.to_string())
    }

    pub fn update_sim_run_progress(&self, run_id: i64, progress: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute(
            "UPDATE sim_runs SET progress=?1 WHERE id=?2 AND status='running'",
            params![progress.clamp(0, 99), run_id],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn get_sim_targets(&self, account_id: i64) -> Result<Vec<Target>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        collect(&conn,"SELECT id,account_id,symbol,name,rule,limit_bps FROM sim_targets WHERE account_id=?1 ORDER BY id",account_id,target_from_row)
    }

    pub fn get_pending_sim_orders(&self, account_id: i64) -> Result<Vec<SimOrder>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        collect(&conn,"SELECT id,account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,rule,stop_price,take_price,stop_bps,take_bps,limit_bps,max_hold_days,status,price,gross,fee,reject_reason,created_at,confirmed_at,filled_at,holding_days FROM sim_orders WHERE account_id=?1 AND status='pending' ORDER BY id",account_id,order_from_row)
    }

    pub fn get_sim_positions(&self, account_id: i64) -> Result<Vec<Position>, String> {
        Ok(self.get_sim_detail(account_id)?.positions)
    }
}

fn consume_lots(
    tx: &Transaction<'_>,
    account_id: i64,
    symbol: &str,
    date: &str,
    mut quantity: i64,
) -> Result<(i64, i64), String> {
    let mut statement = tx.prepare(
        "SELECT id,quantity,cost_basis,acquired_date FROM sim_lots WHERE account_id=?1 AND symbol=?2 AND acquired_date<?3 ORDER BY acquired_date,id",
    ).map_err(|error| error.to_string())?;
    let lots = statement
        .query_map(params![account_id, symbol, date], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<SqliteResult<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let mut cost = 0i64;
    let mut holding_day_shares = 0i64;
    let sell_date =
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| invalid("成交日期无效"))?;
    let requested = quantity;
    for (id, available, lot_cost, acquired_date) in lots {
        if quantity == 0 {
            break;
        }
        let used = quantity.min(available);
        let used_cost = if used == available {
            lot_cost
        } else {
            lot_cost
                .checked_mul(used)
                .ok_or_else(|| invalid("成本超出安全范围"))?
                / available
        };
        cost = cost
            .checked_add(used_cost)
            .ok_or_else(|| invalid("成本超出安全范围"))?;
        let acquired = chrono::NaiveDate::parse_from_str(&acquired_date, "%Y-%m-%d")
            .map_err(|_| invalid("持仓日期无效"))?;
        holding_day_shares = holding_day_shares.saturating_add(
            sell_date
                .signed_duration_since(acquired)
                .num_days()
                .max(0)
                .saturating_mul(used),
        );
        if used == available {
            tx.execute("DELETE FROM sim_lots WHERE id=?1", [id])
        } else {
            tx.execute(
                "UPDATE sim_lots SET quantity=quantity-?1,cost_basis=cost_basis-?2 WHERE id=?3",
                params![used, used_cost, id],
            )
        }
        .map_err(|error| error.to_string())?;
        quantity -= used;
    }
    if quantity == 0 {
        Ok((cost, holding_day_shares / requested.max(1)))
    } else {
        Err(invalid("可用持仓不足"))
    }
}

fn collect<T>(
    conn: &rusqlite::Connection,
    sql: &str,
    id: i64,
    map: fn(&Row<'_>) -> SqliteResult<T>,
) -> Result<Vec<T>, String> {
    let mut statement = conn.prepare(sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([id], map)
        .map_err(|error| error.to_string())?
        .collect::<SqliteResult<Vec<_>>>()
        .map_err(|error| error.to_string());
    rows
}

fn account_from_row(row: &Row<'_>) -> SqliteResult<SimAccount> {
    Ok(SimAccount {
        id: row.get(0)?,
        name: row.get(1)?,
        initial_cash: row.get::<_, i64>(2)?.to_string(),
        current_cash: row.get::<_, i64>(3)?.to_string(),
        mode: row.get(4)?,
        auto_enabled: row.get::<_, i64>(5)? != 0,
        manual_source_enabled: row.get::<_, i64>(6)? != 0,
        rule_source_enabled: row.get::<_, i64>(7)? != 0,
        ai_source_enabled: row.get::<_, i64>(8)? != 0,
        commission_bps: row.get(9)?,
        min_commission: row.get::<_, i64>(10)?.to_string(),
        stamp_tax_bps: row.get(11)?,
        transfer_fee_bps: row.get(12)?,
        slippage_bps: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}
fn target_from_row(row: &Row<'_>) -> SqliteResult<Target> {
    Ok(Target {
        id: row.get(0)?,
        account_id: row.get(1)?,
        symbol: row.get(2)?,
        name: row.get(3)?,
        rule: row.get(4)?,
        limit_bps: row.get(5)?,
    })
}
fn order_from_row(row: &Row<'_>) -> SqliteResult<SimOrder> {
    Ok(SimOrder {
        id: row.get(0)?,
        account_id: row.get(1)?,
        idempotency_key: row.get(2)?,
        symbol: row.get(3)?,
        name: row.get(4)?,
        side: row.get(5)?,
        quantity: row.get(6)?,
        signal_date: row.get(7)?,
        source: row.get(8)?,
        rule: row.get(9)?,
        stop_price: row.get::<_, Option<i64>>(10)?.map(|v| v.to_string()),
        take_price: row.get::<_, Option<i64>>(11)?.map(|v| v.to_string()),
        stop_bps: row.get(12)?,
        take_bps: row.get(13)?,
        limit_bps: row.get(14)?,
        max_hold_days: row.get(15)?,
        status: row.get(16)?,
        price: row.get::<_, Option<i64>>(17)?.map(|v| v.to_string()),
        gross: row.get::<_, Option<i64>>(18)?.map(|v| v.to_string()),
        fee: row.get::<_, Option<i64>>(19)?.map(|v| v.to_string()),
        reject_reason: row.get(20)?,
        created_at: row.get(21)?,
        confirmed_at: row.get(22)?,
        filled_at: row.get(23)?,
        holding_days: row.get(24)?,
    })
}
fn run_from_row(row: &Row<'_>) -> SqliteResult<SimRun> {
    Ok(SimRun {
        id: row.get(0)?,
        account_id: row.get(1)?,
        run_key: row.get(2)?,
        status: row.get(3)?,
        created_at: row.get(4)?,
        finished_at: row.get(5)?,
        message: row.get(6)?,
        progress: row.get(7)?,
    })
}
fn load_order(conn: &rusqlite::Connection, order_id: i64) -> SqliteResult<SimOrder> {
    conn.query_row(
    "SELECT id,account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,rule,stop_price,take_price,stop_bps,take_bps,limit_bps,max_hold_days,status,price,gross,fee,reject_reason,created_at,confirmed_at,filled_at,holding_days FROM sim_orders WHERE id=?1",
    [order_id], order_from_row,
)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn database() -> Database {
        let db = Database {
            conn: std::sync::Mutex::new(Connection::open_in_memory().unwrap()),
        };
        db.migrate_simulation().unwrap();
        db
    }

    fn order(
        account_id: i64,
        key: &str,
        side: &str,
        quantity: i64,
        signal_date: &str,
    ) -> OrderInput {
        OrderInput {
            account_id,
            idempotency_key: key.into(),
            symbol: "sh600000".into(),
            name: "浦发银行".into(),
            side: side.into(),
            quantity,
            signal_date: signal_date.into(),
            source: "manual".into(),
            rule: None,
            stop_bps: 500,
            take_bps: 1_000,
            limit_bps: 1_000,
            max_hold_days: 5,
            ai_generated: false,
        }
    }

    fn bar(
        date: &str,
        open: i64,
        high: i64,
        low: i64,
        close: i64,
        prev: i64,
        volume: u64,
    ) -> RawBar {
        RawBar {
            date: date.into(),
            open: open.to_string(),
            high: high.to_string(),
            low: low.to_string(),
            close: close.to_string(),
            prev_close: prev.to_string(),
            volume,
        }
    }

    #[test]
    fn ledger_rejects_invalid_market_states_and_is_idempotent() {
        let db = database();
        let account = db
            .save_sim_account(&AccountInput {
                id: None,
                name: "验收账户".into(),
                initial_cash: "1000000000".into(),
                mode: "auto".into(),
                auto_enabled: true,
                manual_source_enabled: true,
                rule_source_enabled: true,
                ai_source_enabled: false,
                commission_bps: 0,
                min_commission: "0".into(),
                stamp_tax_bps: 0,
                transfer_fee_bps: 0,
                slippage_bps: 0,
                targets: vec![Target {
                    id: 0,
                    account_id: 0,
                    symbol: "sh600000".into(),
                    name: "浦发银行".into(),
                    rule: "trend_follow".into(),
                    limit_bps: 1_000,
                }],
            })
            .unwrap();

        let buy = db
            .submit_sim_order(&order(account.id, "buy-1", "buy", 100, "2026-01-02"))
            .unwrap();
        assert_eq!(
            db.submit_sim_order(&order(account.id, "buy-1", "buy", 100, "2026-01-02"))
                .unwrap()
                .id,
            buy.id
        );
        let filled = db
            .match_sim_order_raw(
                buy.id,
                &bar(
                    "2026-01-05",
                    100_000,
                    102_000,
                    99_000,
                    101_000,
                    100_000,
                    1000,
                ),
            )
            .unwrap();
        assert_eq!(filled.status, "filled");
        assert_eq!(
            db.match_sim_order_raw(
                buy.id,
                &bar(
                    "2026-01-05",
                    100_000,
                    102_000,
                    99_000,
                    101_000,
                    100_000,
                    1000
                )
            )
            .unwrap()
            .id,
            buy.id
        );

        let same_day = db
            .submit_sim_order(&order(account.id, "same-day", "buy", 100, "2026-01-05"))
            .unwrap();
        assert_eq!(
            db.match_sim_order_raw(
                same_day.id,
                &bar(
                    "2026-01-05",
                    100_000,
                    102_000,
                    99_000,
                    101_000,
                    100_000,
                    1000
                )
            )
            .unwrap()
            .status,
            "rejected"
        );
        let suspended = db
            .submit_sim_order(&order(account.id, "suspended", "buy", 100, "2026-01-05"))
            .unwrap();
        assert_eq!(
            db.match_sim_order_raw(
                suspended.id,
                &bar("2026-01-06", 100_000, 100_000, 100_000, 100_000, 100_000, 0)
            )
            .unwrap()
            .status,
            "rejected"
        );
        let limit_up = db
            .submit_sim_order(&order(account.id, "limit-up", "buy", 100, "2026-01-05"))
            .unwrap();
        assert_eq!(
            db.match_sim_order_raw(
                limit_up.id,
                &bar(
                    "2026-01-06",
                    110_000,
                    110_000,
                    110_000,
                    110_000,
                    100_000,
                    1000
                )
            )
            .unwrap()
            .status,
            "rejected"
        );
        let no_cash = db
            .submit_sim_order(&order(account.id, "no-cash", "buy", 100_000, "2026-01-05"))
            .unwrap();
        assert_eq!(
            db.match_sim_order_raw(
                no_cash.id,
                &bar(
                    "2026-01-06",
                    100_000,
                    101_000,
                    99_000,
                    100_000,
                    100_000,
                    1000
                )
            )
            .unwrap()
            .status,
            "rejected"
        );

        let frozen = db
            .submit_sim_order(&order(account.id, "frozen", "sell", 100, "2026-01-04"))
            .unwrap();
        assert_eq!(
            db.match_sim_order_raw(
                frozen.id,
                &bar(
                    "2026-01-05",
                    100_000,
                    101_000,
                    99_000,
                    100_000,
                    100_000,
                    1000
                )
            )
            .unwrap()
            .status,
            "rejected"
        );
        let limit_down = db
            .submit_sim_order(&order(account.id, "limit-down", "sell", 100, "2026-01-05"))
            .unwrap();
        assert_eq!(
            db.match_sim_order_raw(
                limit_down.id,
                &bar("2026-01-06", 90_000, 90_000, 90_000, 90_000, 100_000, 1000)
            )
            .unwrap()
            .status,
            "rejected"
        );

        let sell = db
            .submit_sim_order(&order(account.id, "sell-1", "sell", 100, "2026-01-06"))
            .unwrap();
        assert_eq!(
            db.match_sim_order_raw(
                sell.id,
                &bar(
                    "2026-01-07",
                    105_000,
                    106_000,
                    104_000,
                    105_000,
                    100_000,
                    1000
                )
            )
            .unwrap()
            .status,
            "filled"
        );
        for (date, equity) in [
            ("2026-01-05", 0),
            ("2026-01-06", 0),
            ("2026-01-07", 0),
            ("2026-01-08", 0),
            ("2026-01-09", 0),
            ("2026-01-12", 0),
            ("2026-01-13", 0),
        ] {
            db.mark_sim_equity(account.id, date, equity, Some(4_000_000))
                .unwrap();
        }
        let detail = db.get_sim_detail(account.id).unwrap();
        assert_eq!(detail.metrics.sample_count, 1);
        assert_eq!(detail.metrics.winning_sell_count, 1);
        assert_eq!(detail.positions.len(), 0);
    }

    #[test]
    fn migration_adds_lot_limit_to_existing_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sim_lots (
                id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, symbol TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '', quantity INTEGER NOT NULL, unit_cost INTEGER NOT NULL,
                cost_basis INTEGER NOT NULL, acquired_date TEXT NOT NULL, stop_bps INTEGER NOT NULL DEFAULT 0,
                take_bps INTEGER NOT NULL DEFAULT 0, max_hold_days INTEGER NOT NULL DEFAULT 0
            );",
        ).unwrap();
        let db = Database {
            conn: std::sync::Mutex::new(conn),
        };
        db.migrate_simulation().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut statement = conn.prepare("PRAGMA table_info(sim_lots)").unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<SqliteResult<Vec<_>>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "limit_bps"));
    }
}
