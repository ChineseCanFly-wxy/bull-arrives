use super::Database;
use crate::datasource::eastmoney_universe::{MarketFilter, PresetInfo};
use crate::db::simulation::SimDetail;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub(super) fn guard_execution(conn: &rusqlite::Connection, account: i64) -> Result<(), String> {
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='research_experiments')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists {
        return Ok(());
    }
    let state: Option<String> = conn
        .query_row(
            "SELECT state FROM research_experiments WHERE account_id=?1 OR id IN (SELECT experiment_id FROM research_daily_comparison WHERE account_id=?1)",
            [account],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if state.is_some_and(|s| !matches!(s.as_str(), "observing" | "extended" | "adopted")) {
        return Err("研究账户已暂停或结束，禁止继续写入交易".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResearchConfig {
    pub execution_mode: String,
    pub auto_research: bool,
    pub observation_days: i64,
    pub min_samples: i64,
    pub max_drawdown_bps: i64,
    pub min_return_bps: i64,
    pub initial_cash: String,
    pub max_active: usize,
    pub stock_count: usize,
    pub commission_bps: i64,
    pub min_commission: String,
    pub stamp_tax_bps: i64,
    pub transfer_fee_bps: i64,
    pub slippage_bps: i64,
}
impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            execution_mode: "realtime".into(),
            auto_research: false,
            observation_days: 28,
            min_samples: 20,
            max_drawdown_bps: 1500,
            min_return_bps: 0,
            initial_cash: "1000000000".into(),
            max_active: 3,
            stock_count: 5,
            commission_bps: 3,
            min_commission: "50000".into(),
            stamp_tax_bps: 5,
            transfer_fee_bps: 1,
            slippage_bps: 5,
        }
    }
}
impl ResearchConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.execution_mode.as_str(),
            "realtime" | "daily" | "parallel"
        ) {
            return Err("执行模式须为 realtime / daily / parallel".into());
        }
        if !(7..=365).contains(&self.observation_days)
            || !(5..=500).contains(&self.min_samples)
            || !(100..=5000).contains(&self.max_drawdown_bps)
            || !(0..=10000).contains(&self.min_return_bps)
            || !(1..=5).contains(&self.max_active)
            || !(1..=10).contains(&self.stock_count)
        {
            return Err("观察期 7–365 天、样本 5–500、回撤 1–50%、收益门槛 0–100%、同时验证 1–5 个、标的 1–10 只".into());
        }
        let cash = crate::simulation::parse_scaled(&self.initial_cash, "初始资金")?;
        if !(10_000_000..=1_000_000_000_000i64).contains(&cash) {
            return Err("初始资金应为 1000–1 亿人民币".into());
        }
        crate::simulation::parse_scaled(&self.min_commission, "最低佣金")?;
        if [
            self.commission_bps,
            self.stamp_tax_bps,
            self.transfer_fee_bps,
            self.slippage_bps,
        ]
        .iter()
        .any(|v| !(0..=1000).contains(v))
        {
            return Err("成本应在 0–1000 基点".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    pub id: i64,
    pub version_id: i64,
    pub name: String,
    pub hypothesis: String,
    pub rule: String,
    pub filter: MarketFilter,
    pub config: ResearchConfig,
    pub state: String,
    pub account_id: Option<i64>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub selection_json: String,
    pub source: String,
    pub last_message: String,
    pub adopted_at: Option<String>,
}
#[derive(Serialize)]
pub struct EquityPoint {
    pub date: String,
    pub equity: String,
    pub return_bps: i64,
    pub drawdown_bps: i64,
}
#[derive(Serialize)]
pub struct ExperimentView {
    pub daily_comparison: Option<SimDetail>,
    pub daily_curve: Vec<EquityPoint>,
    #[serde(flatten)]
    pub experiment: Experiment,
    pub detail: Option<SimDetail>,
    pub curve: Vec<EquityPoint>,
    pub elapsed_days: i64,
    pub remaining_days: i64,
    pub remaining_samples: i64,
    pub verdict: String,
    pub live: Option<crate::db::simulation_live::LiveStatus>,
}

pub fn assessment(
    config: &ResearchConfig,
    days: i64,
    samples: i64,
    return_bps: i64,
    drawdown: i64,
    benchmark: Option<i64>,
    covered_days: usize,
) -> (&'static str, String) {
    if drawdown > config.max_drawdown_bps {
        return ("rejected", "触及冻结的回撤上限，暂停模拟并保留账本".into());
    }
    if days < config.observation_days {
        return (
            "observing",
            format!(
                "观察期剩余 {} 天；已完成 {} 笔卖出样本",
                config.observation_days - days,
                samples
            ),
        );
    }
    if samples < config.min_samples || covered_days < 5 {
        return (
            "extended",
            format!(
                "观察期已到，样本或净值记录不足，继续观察；尚缺 {} 笔卖出",
                (config.min_samples - samples).max(0)
            ),
        );
    }
    let Some(benchmark) = benchmark else {
        return ("extended", "缺少同区间基准，不给出通过结论".into());
    };
    if return_bps <= config.min_return_bps || return_bps <= benchmark {
        return (
            "rejected",
            "收益未超过冻结门槛或同区间基准，淘汰本次候选".into(),
        );
    }
    (
        "qualified",
        "达到本次前向实验门槛，可人工保留为研究策略；不等于实战准入".into(),
    )
}

impl Database {
    pub fn comparison_account(&self, id: i64) -> Result<Option<i64>, String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT account_id FROM research_daily_comparison WHERE experiment_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn link_comparison(&self, id: i64, account: i64) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO research_daily_comparison(experiment_id,account_id) VALUES(?1,?2)",
            params![id, account],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE sim_accounts SET auto_enabled=1 WHERE id=?1",
            [account],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn migrate_research_loop(&self) -> rusqlite::Result<()> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute_batch("CREATE TABLE IF NOT EXISTS research_experiments(
          id INTEGER PRIMARY KEY AUTOINCREMENT,version_id INTEGER NOT NULL UNIQUE,name TEXT NOT NULL,hypothesis TEXT NOT NULL,rule TEXT NOT NULL,
          filter_json TEXT NOT NULL,config_json TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'candidate',account_id INTEGER UNIQUE,
          created_at TEXT NOT NULL,started_at TEXT,selection_json TEXT NOT NULL DEFAULT '[]',source TEXT NOT NULL,last_message TEXT NOT NULL DEFAULT '',adopted_at TEXT,
          FOREIGN KEY(version_id) REFERENCES strategy_versions(id) ON DELETE RESTRICT,FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE RESTRICT);
          CREATE TABLE IF NOT EXISTS research_events(id INTEGER PRIMARY KEY,experiment_id INTEGER NOT NULL,state TEXT NOT NULL,message TEXT NOT NULL,created_at TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS research_daily_comparison(experiment_id INTEGER PRIMARY KEY,account_id INTEGER NOT NULL UNIQUE,FOREIGN KEY(experiment_id) REFERENCES research_experiments(id) ON DELETE RESTRICT,FOREIGN KEY(account_id) REFERENCES sim_accounts(id) ON DELETE RESTRICT);")
    }
    pub fn research_config(&self) -> Result<ResearchConfig, String> {
        let config = self
            .get_setting("research_loop_config")
            .map_err(|e| e.to_string())?
            .map(|v| serde_json::from_str(&v).map_err(|_| "研究配置损坏".to_string()))
            .transpose()?
            .unwrap_or_default();
        Ok(config)
    }
    pub fn register_experiment(
        &self,
        preset: &PresetInfo,
        config: &ResearchConfig,
        source: &str,
    ) -> Result<i64, String> {
        config.validate()?;
        self.sync_strategy_presets(std::slice::from_ref(preset))?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let version:i64=conn.query_row("SELECT v.id FROM strategy_versions v JOIN strategy_cards c ON c.id=v.card_id AND c.current_version=v.version WHERE c.id=?1",[&preset.id],|r|r.get(0)).map_err(|e|e.to_string())?;
        conn.execute("INSERT OR IGNORE INTO research_experiments(version_id,name,hypothesis,rule,filter_json,config_json,created_at,source) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![version,preset.label,preset.description,preset.rule,serde_json::to_string(&preset.filter).map_err(|e|e.to_string())?,serde_json::to_string(config).map_err(|e|e.to_string())?,chrono::Utc::now().to_rfc3339(),source]).map_err(|e|e.to_string())?;
        conn.query_row(
            "SELECT id FROM research_experiments WHERE version_id=?1",
            [version],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
    }
    pub fn research_experiments(&self) -> Result<Vec<Experiment>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt=conn.prepare("SELECT id,version_id,name,hypothesis,rule,filter_json,config_json,state,account_id,created_at,started_at,selection_json,source,last_message,adopted_at FROM research_experiments ORDER BY id DESC LIMIT 100").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, Option<String>>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, String>(12)?,
                    r.get::<_, String>(13)?,
                    r.get::<_, Option<String>>(14)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            let (
                id,
                version_id,
                name,
                hypothesis,
                rule,
                filter,
                config,
                state,
                account_id,
                created_at,
                started_at,
                selection_json,
                source,
                last_message,
                adopted_at,
            ) = row.map_err(|e| e.to_string())?;
            Ok(Experiment {
                id,
                version_id,
                name,
                hypothesis,
                rule,
                filter: serde_json::from_str(&filter).map_err(|e| e.to_string())?,
                config: serde_json::from_str(&config).map_err(|e| e.to_string())?,
                state,
                account_id,
                created_at,
                started_at,
                selection_json,
                source,
                last_message,
                adopted_at,
            })
        })
        .collect()
    }
    pub fn experiment_account(&self, account: i64) -> Result<Option<String>, String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT state FROM research_experiments WHERE account_id=?1 OR id IN (SELECT experiment_id FROM research_daily_comparison WHERE account_id=?1)",
                [account],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn link_experiment(&self, id: i64, account: i64, selection: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let changed=tx.execute("UPDATE research_experiments SET account_id=?2,state='observing',started_at=?3,selection_json=?4,last_message='已冻结选股与规则，等待本地日线推进' WHERE id=?1 AND account_id IS NULL AND state='candidate'",params![id,account,chrono::Utc::now().to_rfc3339(),selection]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("实验已经启动，请刷新".into());
        }
        tx.execute(
            "UPDATE sim_accounts SET auto_enabled=1 WHERE id=?1",
            [account],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn set_experiment_state(&self, id: i64, state: &str, message: &str) -> Result<(), String> {
        self.set_experiment_state_if(id, state, message, None)
    }

    pub fn set_experiment_state_if(
        &self,
        id: i64,
        state: &str,
        message: &str,
        expected: Option<&str>,
    ) -> Result<(), String> {
        if !matches!(
            state,
            "observing" | "extended" | "qualified" | "adopted" | "paused" | "rejected"
        ) {
            return Err("实验状态无效".into());
        }
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let current: String = tx
            .query_row(
                "SELECT state FROM research_experiments WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "实验不存在")?;
        if expected.is_some_and(|e| e != current) {
            return Ok(());
        }
        tx.execute("UPDATE research_experiments SET state=?2,last_message=?3,adopted_at=CASE WHEN ?2='adopted' THEN COALESCE(adopted_at,?4) ELSE adopted_at END WHERE id=?1",params![id,state,message,chrono::Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        if matches!(state, "paused" | "rejected" | "qualified") {
            tx.execute("UPDATE sim_accounts SET auto_enabled=0 WHERE id=(SELECT account_id FROM research_experiments WHERE id=?1)",[id]).map_err(|e|e.to_string())?;
        } else if matches!(state, "observing" | "extended" | "adopted") {
            tx.execute("UPDATE sim_accounts SET auto_enabled=1 WHERE id=(SELECT account_id FROM research_experiments WHERE id=?1)",[id]).map_err(|e|e.to_string())?;
        }
        tx.execute("UPDATE sim_accounts SET auto_enabled=?2 WHERE id IN (SELECT account_id FROM research_daily_comparison WHERE experiment_id=?1)",params![id,i64::from(matches!(state,"observing"|"extended"|"adopted"))]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO research_events(experiment_id,state,message,created_at) VALUES(?1,?2,?3,?4)",params![id,state,message,chrono::Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn experiment_views(&self) -> Result<Vec<ExperimentView>, String> {
        self.research_experiments()?
            .into_iter()
            .map(|experiment| {
                let detail = experiment
                    .account_id
                    .map(|id| self.get_sim_detail(id))
                    .transpose()?;
                let curve = experiment
                    .account_id
                    .map(|id| self.sim_equity_curve(id))
                    .transpose()?
                    .unwrap_or_default();
                // Only elapsed market data dates count; wall clock cannot make a stale account mature.
                let elapsed_days = experiment
                    .started_at
                    .as_ref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .and_then(|start| {
                        curve
                            .last()
                            .and_then(|p| {
                                chrono::NaiveDate::parse_from_str(&p.date, "%Y-%m-%d").ok()
                            })
                            .map(|last| {
                                (last
                                    - start
                                        .with_timezone(
                                            &chrono::FixedOffset::east_opt(28800).unwrap(),
                                        )
                                        .date_naive())
                                .num_days()
                                .max(0)
                            })
                    })
                    .unwrap_or(0);
                let samples = detail.as_ref().map(|d| d.metrics.sample_count).unwrap_or(0);
                let verdict = detail
                    .as_ref()
                    .map(|d| {
                        assessment(
                            &experiment.config,
                            elapsed_days,
                            samples,
                            d.metrics.total_return_bps,
                            d.metrics.max_drawdown_bps,
                            d.metrics.benchmark_return_bps,
                            curve.len(),
                        )
                        .0
                        .to_string()
                    })
                    .unwrap_or("candidate".into());
                Ok(ExperimentView {
                    daily_comparison: self
                        .comparison_account(experiment.id)?
                        .map(|id| self.get_sim_detail(id))
                        .transpose()?,
                    daily_curve: self
                        .comparison_account(experiment.id)?
                        .map(|id| self.sim_equity_curve(id))
                        .transpose()?
                        .unwrap_or_default(),
                    live: experiment
                        .account_id
                        .map(|id| self.live_status(id))
                        .transpose()
                        .ok()
                        .flatten(),
                    remaining_days: (experiment.config.observation_days - elapsed_days).max(0),
                    remaining_samples: (experiment.config.min_samples - samples).max(0),
                    experiment,
                    detail,
                    curve,
                    elapsed_days,
                    verdict,
                })
            })
            .collect()
    }
    pub fn sim_equity_curve(&self, account: i64) -> Result<Vec<EquityPoint>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let initial: i64 = conn
            .query_row(
                "SELECT initial_cash FROM sim_accounts WHERE id=?1",
                [account],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut peak = initial;
        let mut stmt=conn.prepare("SELECT trade_date,equity FROM sim_equity_daily WHERE account_id=?1 ORDER BY trade_date").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([account], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            let (date, equity) = r.map_err(|e| e.to_string())?;
            peak = peak.max(equity);
            Ok(EquityPoint {
                date,
                equity: equity.to_string(),
                return_bps: if initial > 0 {
                    ((equity as i128 - initial as i128) * 10000 / initial as i128) as i64
                } else {
                    0
                },
                drawdown_bps: if peak > 0 {
                    ((peak as i128 - equity as i128) * 10000 / peak as i128) as i64
                } else {
                    0
                },
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simulated_orders_produce_forward_verdict_and_keep_research_separate_from_live_admission() {
        use crate::db::simulation::{AccountInput, OrderInput};
        let db = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate().unwrap();
        db.migrate_simulation().unwrap();
        db.migrate_strategies().unwrap();
        db.migrate_research_loop().unwrap();
        let mut preset = crate::datasource::eastmoney_universe::preset_infos().remove(0);
        preset.id = "fixture-strategy".into();
        let config = ResearchConfig {
            observation_days: 7,
            min_samples: 5,
            ..Default::default()
        };
        let id = db
            .register_experiment(&preset, &config, "synthetic-test")
            .unwrap();
        let account = db
            .save_sim_account(&AccountInput {
                id: None,
                name: "合成数据流程测试".into(),
                initial_cash: config.initial_cash.clone(),
                mode: "auto".into(),
                auto_enabled: false,
                manual_source_enabled: false,
                rule_source_enabled: true,
                ai_source_enabled: false,
                commission_bps: 0,
                min_commission: "0".into(),
                stamp_tax_bps: 0,
                transfer_fee_bps: 0,
                slippage_bps: 0,
                targets: Vec::new(),
            })
            .unwrap();
        db.link_experiment(id, account.id, "[]").unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE research_experiments SET started_at='2026-09-01T00:00:00Z' WHERE id=?1",
                [id],
            )
            .unwrap();
        db.mark_sim_equity(account.id, "2026-09-01", 0, Some(10000))
            .unwrap();
        for index in 0..5 {
            for (offset, side, price, value) in
                [(0, "buy", 100000, 10000000), (1, "sell", 110000, 0)]
            {
                let day = 2 + index * 2 + offset;
                let order = db
                    .submit_sim_order(&OrderInput {
                        account_id: account.id,
                        idempotency_key: format!("test-{index}-{side}"),
                        symbol: "sh600000".into(),
                        name: "测试".into(),
                        side: side.into(),
                        quantity: 100,
                        signal_date: format!("2026-09-{:02}", day - 1),
                        source: "rule".into(),
                        rule: Some("trend_follow".into()),
                        stop_bps: 500,
                        take_bps: 1000,
                        limit_bps: 3000,
                        max_hold_days: 10,
                        ai_generated: false,
                    })
                    .unwrap();
                let date = format!("2026-09-{day:02}");
                let bar = crate::simulation::RawBar {
                    date: date.clone(),
                    open: price.to_string(),
                    high: (price + 1000).to_string(),
                    low: (price - 1000).to_string(),
                    close: price.to_string(),
                    prev_close: "100000".into(),
                    volume: 10000,
                };
                assert_eq!(
                    db.match_sim_order_raw(order.id, &bar).unwrap().status,
                    "filled"
                );
                db.mark_sim_equity(account.id, &date, value, Some(10000))
                    .unwrap();
            }
        }
        let view = db.experiment_views().unwrap().remove(0);
        assert_eq!(view.remaining_samples, 0);
        assert_eq!(view.verdict, "qualified");
        crate::commands::research::evaluate(&db, id).unwrap();
        assert_eq!(db.research_experiments().unwrap()[0].state, "qualified");
        assert!(!db
            .list_auto_sim_account_ids()
            .unwrap()
            .contains(&account.id));
        let card = db.strategy_library().unwrap().cards.remove(0);
        assert!(db
            .transition_strategy(card.version_id, "activate", true, card.revision)
            .is_err());
        db.set_experiment_state(id, "adopted", "人工保留研究")
            .unwrap();
        assert!(db
            .list_auto_sim_account_ids()
            .unwrap()
            .contains(&account.id));
        db.set_experiment_state(id, "rejected", "后续失效").unwrap();
        assert!(db.get_sim_detail(account.id).unwrap().orders.len() >= 10);
        assert!(!db
            .list_auto_sim_account_ids()
            .unwrap()
            .contains(&account.id));
    }
    #[test]
    fn linked_experiment_freezes_account_and_preserves_initial_drawdown() {
        let db = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate().unwrap();
        db.migrate_simulation().unwrap();
        db.migrate_strategies().unwrap();
        db.migrate_research_loop().unwrap();
        let preset = crate::datasource::eastmoney_universe::preset_infos().remove(0);
        let id = db
            .register_experiment(&preset, &ResearchConfig::default(), "test")
            .unwrap();
        let c = ResearchConfig::default();
        let a = db
            .save_sim_account(&crate::db::simulation::AccountInput {
                id: None,
                name: "实验".into(),
                initial_cash: c.initial_cash,
                mode: "auto".into(),
                auto_enabled: false,
                manual_source_enabled: false,
                rule_source_enabled: true,
                ai_source_enabled: false,
                commission_bps: 3,
                min_commission: c.min_commission,
                stamp_tax_bps: 5,
                transfer_fee_bps: 1,
                slippage_bps: 5,
                targets: Vec::new(),
            })
            .unwrap();
        db.link_experiment(id, a.id, "[]").unwrap();
        assert!(db.list_auto_sim_account_ids().unwrap().contains(&a.id));
        assert!(db.link_experiment(id, a.id, "[]").is_err());
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("UPDATE sim_accounts SET cash=900000000 WHERE id=?1", [a.id])
                .unwrap();
        }
        db.mark_sim_equity(a.id, "2026-09-18", 0, Some(10000))
            .unwrap();
        assert_eq!(
            db.get_sim_detail(a.id).unwrap().metrics.max_drawdown_bps,
            1000
        );
        assert_eq!(db.sim_equity_curve(a.id).unwrap()[0].drawdown_bps, 1000);
        db.set_experiment_state(id, "paused", "pause").unwrap();
        assert!(!db.list_auto_sim_account_ids().unwrap().contains(&a.id));
        {
            let conn = db.conn.lock().unwrap();
            assert!(guard_execution(&conn, a.id).is_err());
        }
        db.set_experiment_state(id, "observing", "resume").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            assert!(guard_execution(&conn, a.id).is_ok());
        }
        assert!(db.delete_sim_account(a.id).is_err());
    }

    #[test]
    fn parallel_accounts_are_independent_and_pause_together() {
        let db = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate().unwrap();
        db.migrate_simulation().unwrap();
        db.migrate_simulation_live().unwrap();
        db.migrate_strategies().unwrap();
        db.migrate_research_loop().unwrap();
        let mut c = ResearchConfig::default();
        c.execution_mode = "parallel".into();
        c.validate().unwrap();
        let p = crate::datasource::eastmoney_universe::preset_infos().remove(0);
        let id = db.register_experiment(&p, &c, "test").unwrap();
        let input = crate::db::simulation::AccountInput {
            id: None,
            name: "对照测试".into(),
            initial_cash: c.initial_cash.clone(),
            mode: "auto".into(),
            auto_enabled: false,
            manual_source_enabled: false,
            rule_source_enabled: true,
            ai_source_enabled: false,
            commission_bps: 0,
            min_commission: "0".into(),
            stamp_tax_bps: 0,
            transfer_fee_bps: 0,
            slippage_bps: 0,
            targets: vec![],
        };
        let real = db.save_sim_account(&input).unwrap();
        let daily = db.save_sim_account(&input).unwrap();
        db.enable_live_account(real.id, &[]).unwrap();
        db.link_experiment(id, real.id, "[]").unwrap();
        db.link_comparison(id, daily.id).unwrap();
        assert_ne!(real.id, daily.id);
        assert!(db.live_account(real.id).unwrap());
        assert!(!db.live_account(daily.id).unwrap());
        assert_eq!(
            db.experiment_account(daily.id).unwrap().as_deref(),
            Some("observing")
        );
        db.set_experiment_state(id, "paused", "pause").unwrap();
        assert!(db.list_auto_sim_account_ids().unwrap().is_empty());
        {
            let conn = db.conn.lock().unwrap();
            assert!(guard_execution(&conn, daily.id).is_err());
        }
        let view = db.experiment_views().unwrap().remove(0);
        assert_eq!(view.daily_comparison.unwrap().account.id, daily.id);
        assert!(db.delete_sim_account(daily.id).is_err());
    }
    #[test]
    fn probation_never_passes_early_or_without_samples_or_benchmark() {
        let c = ResearchConfig::default();
        assert_eq!(assessment(&c, 2, 40, 1000, 50, Some(0), 30).0, "observing");
        assert_eq!(assessment(&c, 40, 1, 1000, 50, Some(0), 30).0, "extended");
        assert_eq!(assessment(&c, 40, 40, 1000, 50, None, 30).0, "extended");
        assert_eq!(assessment(&c, 40, 40, 1000, 50, Some(0), 30).0, "qualified");
        assert_eq!(assessment(&c, 1, 0, 0, 1600, Some(0), 1).0, "rejected");
    }
}
