//! Frozen-model follow accounts. Research ledgers remain immutable reference accounts.
use super::Database;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const AUTOMATIC_RECORD_KEY: &str = "research_auto_trade_record";
pub const AUTOMATIC_ENABLED_KEY: &str = "research_auto_trade_enabled";
pub fn automatic_excluded_key(model: &str) -> String { format!("research_auto_excluded_{model}") }

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomaticRecord {
    pub day: String,
    pub as_of: String,
    pub update_at: String,
    pub runner_sha256: String,
    pub last_completed_day: Option<String>,
    pub last_completed_at: Option<String>,
    pub running_owner: Option<u32>,
    pub started_at: Option<String>,
    pub retry_at: Option<String>,
    pub failures: u32,
    pub last_error: Option<String>,
    pub models: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FollowBinding {
    pub account_id: i64,
    pub source_run_id: i64,
    pub model_id: String,
    pub model_name: String,
    pub max_positions: i64,
    #[serde(default = "baseline_allocation")]
    pub allocation_policy: String,
    #[serde(default = "baseline_execution")]
    pub execution_policy: String,
    pub enabled: bool,
    #[serde(default)]
    pub auto_execute: bool,
    pub initial_cash_cny: f64,
    pub as_of: String,
    pub source_sha256: String,
    pub context_json: String,
    pub state: String,
    pub message: String,
    #[serde(default)]
    pub allocation_date: Option<String>,
    #[serde(default)]
    pub allocation_equity: Option<i64>,
    #[serde(default)]
    pub valuation_blocked: bool,
    #[serde(default)]
    pub valuation_block_reason: Option<String>,
    #[serde(default)]
    pub candidate_cursor: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FollowOrderMeta {
    pub order_id: i64,
    pub limit_price: i64,
    pub estimated_fee: i64,
    pub factor: f64,
    pub atr: f64,
    pub quote_at: i64,
    pub valid_until: i64,
    pub context_as_of: String,
    pub context_sha256: String,
    #[serde(default = "baseline_allocation")]
    pub allocation_policy: String,
    #[serde(default = "baseline_execution")]
    pub execution_policy: String,
    #[serde(default)]
    pub automatic_submission: bool,
}
fn baseline_execution() -> String {
    "baseline".into()
}
fn baseline_allocation() -> String {
    "baseline8".into()
}
pub(crate) fn decode_binding(raw: &str) -> Result<FollowBinding, String> {
    let mut binding: FollowBinding = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if value.get("valuation_blocked").is_none() && binding.state == "company_action_review" {
        binding.valuation_blocked = true;
        binding.valuation_block_reason = Some("legacy_review".into());
    }
    Ok(binding)
}
pub(crate) fn ensure_follow_source(
    conn: &rusqlite::Connection,
    binding: &FollowBinding,
    expected: &str,
) -> Result<(), String> {
    let globally_paused: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key IN ('ai_enabled','research_auto_trade_enabled') AND value='0')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if globally_paused { return Err("自动模型交易总开关已暂停".into()); }
    crate::commands::model_follow::allocation_targets(binding)?;
    crate::commands::model_follow::ensure_execution_policy(binding)?;
    if !binding.enabled || binding.valuation_blocked || binding.source_sha256 != expected {
        return Err("跟随已暂停、公司行动待核对或原模型计划已更新".into());
    }
    let raw: String = conn
        .query_row(
            "SELECT bundle_json FROM model_research_runs WHERE id=?1",
            [binding.source_run_id],
            |r| r.get(0),
        )
        .map_err(|_| "原模型账户不存在")?;
    let run = super::model_research::decode_run(&raw)?;
    if run["content_sha256"].as_str() != Some(expected)
        || run["model_id"].as_str() != Some(binding.model_id.as_str())
        || run["comparison"] != "baseline"
        || run["mode"] != "forward"
        || run["holding_days"] != 20
    {
        return Err("原模型账户已更新，等待重新生成操作计划".into());
    }
    Ok(())
}

/// Move the allocation snapshot only by the ledger cash flow. Existing quantities,
/// order limits, model policies and the candidate cursor are left intact.
pub(super) fn adjust_follow_capital(tx: &rusqlite::Transaction<'_>, account: i64, initial: i64, delta: i64) -> Result<(), String> {
    let raw: String = tx.query_row("SELECT binding_json FROM model_follow_accounts WHERE account_id=?1", [account], |r| r.get(0)).map_err(|e| e.to_string())?;
    let mut binding = decode_binding(&raw)?;
    binding.initial_cash_cny = initial as f64 / crate::simulation::SCALE as f64;
    if let Some(equity) = binding.allocation_equity {
        binding.allocation_equity = equity.checked_add(delta).filter(|v| (0..=crate::simulation::MAX_MONEY).contains(v));
        if binding.allocation_equity.is_none() { binding.allocation_date = None; }
    }
    tx.execute("UPDATE model_follow_accounts SET binding_json=?2 WHERE account_id=?1", params![account, serde_json::to_string(&binding).map_err(|e| e.to_string())?]).map(|_| ()).map_err(|e| e.to_string())
}
impl Database {
    pub fn follow_automatic_record(&self) -> Result<AutomaticRecord, String> {
        self.get_setting(AUTOMATIC_RECORD_KEY).map_err(|e| e.to_string())?
            .map(|raw| serde_json::from_str(&raw).map_err(|_| "自动模型调度记录损坏，暂停重复执行".to_string()))
            .transpose().map(|record| record.unwrap_or_default())
    }

    pub fn change_follow_automatic_record<T>(&self, change: impl FnOnce(&mut AutomaticRecord) -> Result<T, String>) -> Result<T, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        let raw: Option<String> = tx.query_row("SELECT value FROM settings WHERE key=?1", [AUTOMATIC_RECORD_KEY], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        let mut record: AutomaticRecord = raw.map(|raw| serde_json::from_str(&raw).map_err(|_| "自动模型调度记录损坏，暂停重复执行".to_string())).transpose()?.unwrap_or_default();
        let result = change(&mut record)?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![AUTOMATIC_RECORD_KEY, serde_json::to_string(&record).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }

    /// The global setting and every unfilled follow order change in one transaction.
    pub fn set_follow_automatic_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![AUTOMATIC_ENABLED_KEY, if enabled { "1" } else { "0" }]).map_err(|e| e.to_string())?;
        if !enabled {
            tx.execute("UPDATE sim_orders SET status='rejected',reject_reason='自动模型交易已统一暂停，未成交委托撤销' WHERE account_id IN (SELECT account_id FROM model_follow_accounts) AND status IN ('pending','awaiting_confirmation')", []).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn automatic_model_excluded(&self, model: &str) -> Result<bool, String> {
        Ok(self.get_setting(&automatic_excluded_key(model)).map_err(|e| e.to_string())?.as_deref() == Some("1"))
    }

    pub fn migrate_model_follow(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch("CREATE TABLE IF NOT EXISTS model_follow_accounts (
            account_id INTEGER PRIMARY KEY REFERENCES sim_accounts(id) ON DELETE CASCADE,
            source_run_id INTEGER NOT NULL REFERENCES model_research_runs(id), binding_json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS model_follow_orders (order_id INTEGER PRIMARY KEY REFERENCES sim_orders(id) ON DELETE CASCADE, meta_json TEXT NOT NULL, notified_status TEXT);
            CREATE TRIGGER IF NOT EXISTS model_follow_account_exclusive BEFORE INSERT ON model_follow_accounts
            WHEN NOT EXISTS(SELECT 1 FROM model_follow_accounts WHERE account_id=NEW.account_id) AND (
                 EXISTS(SELECT 1 FROM sim_live_plans WHERE account_id=NEW.account_id)
              OR EXISTS(SELECT 1 FROM sim_targets WHERE account_id=NEW.account_id)
              OR EXISTS(SELECT 1 FROM sim_orders WHERE account_id=NEW.account_id)
              OR EXISTS(SELECT 1 FROM research_experiments WHERE account_id=NEW.account_id)
              OR EXISTS(SELECT 1 FROM research_daily_comparison WHERE account_id=NEW.account_id))
            BEGIN SELECT RAISE(ABORT,'自动模型必须使用独立专用账户，不能借用手动或其他策略账户'); END;
            CREATE TRIGGER IF NOT EXISTS model_follow_binding_identity BEFORE UPDATE OF source_run_id,account_id ON model_follow_accounts
            WHEN NEW.source_run_id<>OLD.source_run_id OR NEW.account_id<>OLD.account_id
            BEGIN SELECT RAISE(ABORT,'自动模型专用账户不能改绑到其他模型'); END;
            CREATE TRIGGER IF NOT EXISTS model_follow_account_config BEFORE UPDATE OF mode,manual_source_enabled,rule_source_enabled,ai_source_enabled ON sim_accounts
            WHEN EXISTS(SELECT 1 FROM model_follow_accounts WHERE account_id=OLD.id)
              AND (NEW.manual_source_enabled<>0 OR NEW.rule_source_enabled<>1 OR NEW.ai_source_enabled<>0 OR NEW.mode<>'auto')
            BEGIN SELECT RAISE(ABORT,'自动模型专用账户不允许手动或其他策略使用'); END;")?;
        // Guard the shared account slots in SQLite, including internal scan/research callers.
        for table in ["sim_live_accounts", "sim_live_plans", "sim_targets", "research_experiments", "research_daily_comparison"] {
            for (event, suffix) in [("INSERT", "insert"), ("UPDATE OF account_id", "update")] {
                conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS follow_isolate_{table}_{suffix} BEFORE {event} ON {table}
                    WHEN EXISTS(SELECT 1 FROM model_follow_accounts WHERE account_id=NEW.account_id)
                    BEGIN SELECT RAISE(ABORT,'自动模型专用账户不能被手动、观察或其他策略使用'); END;"))?;
            }
        }
        Ok(())
    }
    /// Create the empty model-only ledger, engine marker and binding atomically.
    /// This avoids exposing an unbound account to ordinary account editing during setup.
    pub fn create_follow_account(&self, binding: &mut FollowBinding) -> Result<super::simulation::SimAccount, String> {
        self.create_follow_account_inner(binding, false)
    }

    pub fn create_automatic_follow_account(&self, binding: &mut FollowBinding) -> Result<super::simulation::SimAccount, String> {
        self.create_follow_account_inner(binding, true)
    }

    fn create_follow_account_inner(&self, binding: &mut FollowBinding, automatic: bool) -> Result<super::simulation::SimAccount, String> {
        if binding.account_id != 0 || !binding.enabled || !binding.auto_execute { return Err("新自动模型账户必须从独立空仓开始".into()); }
        let initial = super::simulation::capital_cny_scaled(binding.initial_cash_cny)?;
        let timestamp = chrono::Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        ensure_follow_source(&tx, binding, &binding.source_sha256)?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM model_follow_accounts WHERE json_extract(binding_json,'$.model_id')=?1)", [&binding.model_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if exists { return Err("该模型已有独立自动账户，请复用；暂停账户不会重复新建".into()); }
        let excluded: Option<String> = tx.query_row("SELECT value FROM settings WHERE key=?1", [automatic_excluded_key(&binding.model_id)], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        if automatic && excluded.as_deref() == Some("1") { return Err("该模型已由用户暂停或删除，后台不重新建账户".into()); }
        // Only an explicit user setup can clear a prior deletion marker.
        if !automatic { tx.execute("DELETE FROM settings WHERE key=?1", [automatic_excluded_key(&binding.model_id)]).map_err(|e| e.to_string())?; }
        tx.execute("INSERT INTO sim_accounts(name,initial_cash,cash,mode,auto_enabled,manual_source_enabled,rule_source_enabled,ai_source_enabled,commission_bps,min_commission,stamp_tax_bps,transfer_fee_bps,slippage_bps,created_at,updated_at) VALUES(?1,?2,?2,'auto',1,0,1,0,3,50000,5,0,0,?3,?3)", params![format!("跟随 · {}", binding.model_name), initial, timestamp]).map_err(|e| e.to_string())?;
        let id = tx.last_insert_rowid();
        let mut created = binding.clone(); created.account_id = id; created.initial_cash_cny = initial as f64 / crate::simulation::SCALE as f64;
        tx.execute("INSERT INTO sim_live_accounts(account_id,message) VALUES(?1,'自动模型专用：等待原模型条件和新盘口')", [id]).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO model_follow_accounts(account_id,source_run_id,binding_json) VALUES(?1,?2,?3)", params![id, created.source_run_id, serde_json::to_string(&created).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        let mut account = tx.query_row("SELECT id,name,initial_cash,cash,mode,auto_enabled,manual_source_enabled,rule_source_enabled,ai_source_enabled,commission_bps,min_commission,stamp_tax_bps,transfer_fee_bps,slippage_bps,created_at,updated_at FROM sim_accounts WHERE id=?1", [id], super::simulation::account_from_row).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        account.managed_by = "model_follow".into(); *binding = created;
        Ok(account)
    }
    /// Delete only a dedicated follow ledger. The caller must hold the global follow gate.
    pub fn delete_follow_account(&self, account_id: i64) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // migrate_simulation enables this, but legacy connections may have disabled it.
        // Always remove leaves explicitly instead of depending on ON DELETE CASCADE.
        let foreign_keys: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if !foreign_keys {
            log::debug!("Deleting follow ledger with explicit child cleanup; foreign_keys is off");
        }
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (source_run_id, binding_json): (i64, String) = tx
            .query_row(
                "SELECT f.source_run_id,f.binding_json FROM model_follow_accounts f
                 JOIN sim_accounts a ON a.id=f.account_id WHERE f.account_id=?1",
                [account_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("自动模型专用账户不存在；不能删除手动或研究源账户")?;
        let removed = decode_binding(&binding_json)?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,'1') ON CONFLICT(key) DO UPDATE SET value='1'", [automatic_excluded_key(&removed.model_id)]).map_err(|e| e.to_string())?;
        // Refuse corrupted/legacy cross-links even when foreign key enforcement is off.
        let research_account: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM research_experiments WHERE account_id=?1)
                     OR EXISTS(SELECT 1 FROM research_daily_comparison WHERE account_id=?1)",
                [account_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if research_account {
            return Err("研究源账户不能通过自动模型删除接口删除".into());
        }
        tx.execute(
            "UPDATE sim_orders SET status='rejected',reject_reason='用户删除自动模型专用账户，未成交委托撤销'
             WHERE account_id=?1 AND status IN ('pending','awaiting_confirmation')",
            [account_id],
        )
        .map_err(|e| e.to_string())?;
        // The pending-buy reservation is computed from these orders and their metadata;
        // removing them and the final cash ledger leaves no reservation to carry forward.
        for table in ["model_follow_orders", "sim_live_orders"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE order_id IN (SELECT id FROM sim_orders WHERE account_id=?1)"),
                [account_id],
            )
            .map_err(|e| e.to_string())?;
        }
        for table in [
            "sim_live_consumed",
            "sim_live_equity",
            "sim_live_marks",
            "sim_live_risk",
            "sim_live_plans",
            "sim_live_accounts",
            "sim_lots",
            "sim_targets",
            "sim_runs",
            "sim_capital_adjustments",
            "sim_equity_daily",
            "sim_orders",
            "model_follow_accounts",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE account_id=?1"),
                [account_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM sim_accounts WHERE id=?1", [account_id])
            .map_err(|e| e.to_string())?;

        // A paused follow still owns its source. Manual watches share maintenance by the
        // frozen model identity, not by job/run ID or runner version.
        tx.execute(
            "UPDATE model_research_runs SET enabled=0 WHERE id=?1
             AND NOT EXISTS(SELECT 1 FROM model_follow_accounts WHERE source_run_id=?1)
             AND NOT EXISTS(
                 SELECT 1 FROM model_condition_watches w WHERE w.enabled=1
                 AND json_extract(w.config_json,'$.model_id')=
                     json_extract(json_extract(model_research_runs.bundle_json,'$.content'),'$.model_id')
                 AND json_extract(w.config_json,'$.model_sha256')=
                     json_extract(json_extract(model_research_runs.bundle_json,'$.content'),'$.model_sha256')
             )",
            [source_run_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn is_follow_account(&self, account: i64) -> Result<bool, String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM model_follow_accounts WHERE account_id=?1)",
                [account],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub fn follow_bindings(&self) -> Result<Vec<FollowBinding>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut statement = conn
            .prepare("SELECT binding_json FROM model_follow_accounts ORDER BY account_id DESC")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| decode_binding(&r.map_err(|e| e.to_string())?))
            .collect()
    }
    pub fn follow_binding(&self, account: i64) -> Result<FollowBinding, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let raw: String = conn
            .query_row(
                "SELECT binding_json FROM model_follow_accounts WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .map_err(|_| "跟随账户不存在")?;
        decode_binding(&raw)
    }
    pub fn save_follow_binding(&self, binding: &FollowBinding) -> Result<(), String> {
        self.save_follow_binding_inner(binding, false)
    }
    pub fn save_follow_binding_for_user(&self, binding: &FollowBinding) -> Result<(), String> {
        self.save_follow_binding_inner(binding, true)
    }
    fn save_follow_binding_inner(&self, binding: &FollowBinding, user_action: bool) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        let mut updated = binding.clone();
        let initial: i64 = tx.query_row("SELECT initial_cash FROM sim_accounts WHERE id=?1", [binding.account_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        updated.initial_cash_cny = initial as f64 / crate::simulation::SCALE as f64;
        let stored: Option<String> = tx.query_row("SELECT binding_json FROM model_follow_accounts WHERE account_id=?1", [binding.account_id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        if let Some(raw) = stored {
            let current = decode_binding(&raw)?;
            if current.enabled != updated.enabled && !user_action { return Err("账户启停已由用户改变，旧后台任务不能覆盖".into()); }
            if current.source_run_id != binding.source_run_id || current.model_id != binding.model_id { return Err("自动模型专用账户不能改绑到其他模型".into()); }
            if !updated.enabled {
                tx.execute("UPDATE sim_orders SET status='rejected',reject_reason='用户暂停自动模型，未成交委托撤销' WHERE account_id=?1 AND status IN ('pending','awaiting_confirmation')", [binding.account_id]).map_err(|e| e.to_string())?;
                tx.execute("INSERT INTO settings(key,value) VALUES(?1,'1') ON CONFLICT(key) DO UPDATE SET value='1'", [automatic_excluded_key(&binding.model_id)]).map_err(|e| e.to_string())?;
            } else if !current.enabled {
                let other_active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM model_follow_accounts WHERE account_id<>?1 AND json_extract(binding_json,'$.model_id')=?2 AND json_extract(binding_json,'$.enabled')=1)", params![binding.account_id, binding.model_id], |r| r.get(0)).map_err(|e| e.to_string())?;
                if other_active { return Err("同一模型已有活动自动账户，不能同时恢复另一账户".into()); }
                tx.execute("DELETE FROM settings WHERE key=?1", [automatic_excluded_key(&binding.model_id)]).map_err(|e| e.to_string())?;
            }
            if (updated.initial_cash_cny - binding.initial_cash_cny).abs() > 0.000001 { updated.allocation_equity = current.allocation_equity; updated.allocation_date = current.allocation_date; }
        }
        tx.execute("INSERT INTO model_follow_accounts(account_id,source_run_id,binding_json) VALUES(?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET binding_json=excluded.binding_json", params![binding.account_id,binding.source_run_id,serde_json::to_string(&updated).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    /// Upgrade only model-follow accounts. Retire old manual plans instead of executing stale orders.
    pub fn enable_follow_automation(&self, binding: &mut FollowBinding) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let (mode,auto,manual,rule,ai):(String,bool,bool,bool,bool)=tx.query_row(
            "SELECT mode,auto_enabled,manual_source_enabled,rule_source_enabled,ai_source_enabled FROM sim_accounts WHERE id=?1",[binding.account_id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))
        ).map_err(|_|"跟随模拟账户不存在")?;
        let known: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM model_follow_accounts WHERE account_id=?1)",
                [binding.account_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !known {
            return Err("只升级原模型跟随账户，不改变其他模拟账户".into());
        }
        let stored: String = tx.query_row("SELECT binding_json FROM model_follow_accounts WHERE account_id=?1", [binding.account_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if decode_binding(&stored)?.enabled != binding.enabled { return Err("账户启停已由用户改变，旧后台任务不能覆盖".into()); }
        let retire = !binding.auto_execute || mode != "auto" || manual || !rule || ai;
        if !retire && auto == binding.enabled {
            return Ok(());
        }
        if retire {
            tx.execute("UPDATE sim_orders SET status='rejected',reject_reason='跟随切换为全自动模拟，旧人工计划撤销；等待当前模型条件和新盘口' WHERE account_id=?1 AND status IN ('awaiting_confirmation','pending')",[binding.account_id]).map_err(|e|e.to_string())?;
        }
        tx.execute("UPDATE sim_accounts SET mode='auto',auto_enabled=?2,manual_source_enabled=0,rule_source_enabled=1,ai_source_enabled=0 WHERE id=?1",params![binding.account_id,binding.enabled]).map_err(|e|e.to_string())?;
        let mut updated = binding.clone();
        updated.auto_execute = true;
        if retire {
            updated.candidate_cursor = 0;
            updated.message =
                "已切换全自动模拟；旧未成交计划撤销，保留实际现金与持仓，等待当前模型和新盘口。"
                    .into();
        }
        tx.execute(
            "UPDATE model_follow_accounts SET binding_json=?2 WHERE account_id=?1",
            params![
                updated.account_id,
                serde_json::to_string(&updated).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        *binding = updated;
        Ok(())
    }
    pub fn stage_follow_order(
        &self,
        input: &super::simulation::OrderInput,
        meta: &FollowOrderMeta,
        reason: &str,
        submitted: i64,
    ) -> Result<i64, String> {
        let side = crate::simulation::Side::parse(&input.side)?;
        if input.source != "rule"
            || input.ai_generated
            || input.quantity <= 0
            || meta.limit_price <= 0
            || meta.valid_until <= submitted
        {
            return Err("跟随操作参数或有效期无效".into());
        }
        let board_limit = crate::market_rules::ensure_simulatable(&input.symbol, &input.name)?;
        if side == crate::simulation::Side::Buy {
            crate::market_rules::validate_buy_quantity(&input.symbol, input.quantity)?;
        }
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let raw: String = tx
            .query_row(
                "SELECT binding_json FROM model_follow_accounts WHERE account_id=?1",
                [input.account_id],
                |r| r.get(0),
            )
            .map_err(|_| "跟随账户不存在")?;
        let binding = decode_binding(&raw)?;
        ensure_follow_source(&tx, &binding, &meta.context_sha256)?;
        if meta.allocation_policy != binding.allocation_policy {
            return Err("仓位配置已改变，请等待新操作单".into());
        }
        if meta.execution_policy != binding.execution_policy {
            return Err("模型时点配置已改变，请等待新操作单".into());
        }
        if meta.automatic_submission != binding.auto_execute {
            return Err("委托执行方式已改变，请等待程序生成的新操作单".into());
        }
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM sim_orders WHERE account_id=?1 AND idempotency_key=?2",
                params![input.account_id, input.idempotency_key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = existing {
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(id);
        }
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sim_orders WHERE account_id=?1 AND symbol=?2 AND status IN ('awaiting_confirmation','pending'))",params![input.account_id,input.symbol],|r|r.get(0)).map_err(|e|e.to_string())?;
        if active {
            return Err("该股票已有待确认或待成交操作".into());
        }
        let (cash, mode, auto, rule): (i64, String, bool, bool) = tx
            .query_row(
                "SELECT cash,mode,auto_enabled,rule_source_enabled FROM sim_accounts WHERE id=?1",
                [input.account_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        let valid_mode = if binding.auto_execute {
            mode == "auto" && auto
        } else {
            mode == "confirm" && !auto
        };
        if !valid_mode || !rule {
            return Err("跟随执行模式不一致，请等待自动账户同步".into());
        }
        if side == crate::simulation::Side::Buy {
            crate::commands::model_follow::ensure_entry_condition(&binding, &input.symbol)?;
            let count:i64=tx.query_row("SELECT COUNT(*) FROM (SELECT symbol FROM sim_lots WHERE account_id=?1 UNION SELECT symbol FROM sim_orders WHERE account_id=?1 AND side='buy' AND status IN ('awaiting_confirmation','pending'))",[input.account_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if count >= binding.max_positions {
                return Err("持仓和待买订单已占满名额".into());
            }
            let reserved:i64=tx.query_row("SELECT COALESCE(SUM(o.quantity*json_extract(f.meta_json,'$.limit_price')+json_extract(f.meta_json,'$.estimated_fee')),0) FROM sim_orders o JOIN model_follow_orders f ON f.order_id=o.id WHERE o.account_id=?1 AND o.side='buy' AND o.status IN ('awaiting_confirmation','pending')",[input.account_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            let required = input
                .quantity
                .checked_mul(meta.limit_price)
                .and_then(|g| g.checked_add(meta.estimated_fee))
                .ok_or("订单金额超限")?;
            if required > cash - reserved {
                return Err("现金已被其他待买单预留".into());
            }
        }
        let created = DateTime::<Utc>::from_timestamp(submitted, 0)
            .ok_or("操作时间无效")?
            .to_rfc3339();
        let status = if binding.auto_execute {
            "pending"
        } else {
            "awaiting_confirmation"
        };
        tx.execute("INSERT INTO sim_orders(account_id,idempotency_key,symbol,name,side,quantity,signal_date,source,limit_bps,status,created_at,confirmed_at,decision_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,'rule',?8,?9,?10,CASE WHEN ?9='pending' THEN ?10 END,?11)",params![input.account_id,input.idempotency_key,input.symbol,input.name,input.side,input.quantity,input.signal_date,board_limit,status,created,reason]).map_err(|e|e.to_string())?;
        let id = tx.last_insert_rowid();
        let mut final_meta = meta.clone();
        final_meta.order_id = id;
        tx.execute(
            "INSERT INTO sim_live_orders(order_id,limit_price,submitted_at) VALUES(?1,?2,?3)",
            params![id, meta.limit_price, submitted],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO model_follow_orders(order_id,meta_json) VALUES(?1,?2)",
            params![
                id,
                serde_json::to_string(&final_meta).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }
    pub fn follow_order_meta(&self, id: i64) -> Result<FollowOrderMeta, String> {
        let raw: String = self
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT meta_json FROM model_follow_orders WHERE order_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "订单缺少原模型计划证据")?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }
    pub fn follow_order_account(&self, id: i64) -> Result<i64, String> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).query_row(
            "SELECT o.account_id FROM sim_orders o JOIN model_follow_orders f ON f.order_id=o.id WHERE o.id=?1",[id],|r|r.get(0)).map_err(|_|"跟随操作单不存在".into())
    }
    pub fn reject_follow_order(&self, id: i64, reason: &str) -> Result<(), String> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute(
            "UPDATE sim_orders SET status='rejected',reject_reason=?1 WHERE id=?2 AND status IN ('pending','awaiting_confirmation')",
            params![reason,id]).map(|_|()).map_err(|e|e.to_string())
    }
    pub fn confirm_follow_order(&self, id: i64, submitted: i64) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let (account, status): (i64, String) = tx
            .query_row(
                "SELECT account_id,status FROM sim_orders WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "操作单不存在")?;
        if status != "awaiting_confirmation" {
            return Err("操作单已确认、撤销或过期，请刷新".into());
        }
        let raw: String = tx
            .query_row(
                "SELECT meta_json FROM model_follow_orders WHERE order_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "跟随委托缺计划证据")?;
        let meta: FollowOrderMeta = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if submitted >= meta.valid_until {
            return Err("操作单已过期，请刷新".into());
        }
        let raw: String = tx
            .query_row(
                "SELECT binding_json FROM model_follow_accounts WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .map_err(|_| "跟随账户不存在")?;
        let binding = decode_binding(&raw)?;
        ensure_follow_source(&tx, &binding, &meta.context_sha256)?;
        if meta.allocation_policy != binding.allocation_policy {
            return Err("仓位配置已改变，请等待新操作单".into());
        }
        if meta.execution_policy != binding.execution_policy {
            return Err("模型时点配置已改变，请等待新操作单".into());
        }
        if meta.automatic_submission != binding.auto_execute {
            return Err("委托执行方式已改变，请等待程序生成的新操作单".into());
        }
        let (side, symbol): (String, String) = tx
            .query_row(
                "SELECT side,symbol FROM sim_orders WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if side == "buy" {
            crate::commands::model_follow::ensure_entry_condition(&binding, &symbol)?;
        }
        tx.execute("UPDATE sim_orders SET status='pending',confirmed_at=?1,reject_reason=NULL WHERE id=?2 AND status='awaiting_confirmation'", params![DateTime::from_timestamp(submitted,0).ok_or("确认时间无效")?.to_rfc3339(),id]).map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE sim_live_orders SET submitted_at=?1 WHERE order_id=?2",
            params![submitted, id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn recover_follow_valuation(
        &self,
        binding: &mut FollowBinding,
        ticks: &[crate::simulation_live::LiveTick],
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        if binding.valuation_block_reason.as_deref() != Some("quote_mismatch") {
            return Err("公司行动或未知证据待核对，不能仅凭报价恢复权益".into());
        }
        let ctx: crate::commands::model_follow::FollowContext =
            serde_json::from_str(&binding.context_json).map_err(|e| e.to_string())?;
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut recovered = binding.clone();
        recovered.valuation_blocked = false;
        recovered.valuation_block_reason = None;
        recovered.state = "listening".into();
        ensure_follow_source(&tx, &recovered, &ctx.source_run_sha256)?;
        let raw: String = tx
            .query_row(
                "SELECT bundle_json FROM model_research_runs WHERE id=?1",
                [binding.source_run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let run = super::model_research::decode_run(&raw)?;
        if run["as_of"] != ctx.as_of {
            return Err("恢复证据日期与当前原模型不符".into());
        }
        let mut statement = tx
            .prepare("SELECT DISTINCT symbol FROM sim_lots WHERE account_id=?1")
            .map_err(|e| e.to_string())?;
        let symbols = statement
            .query_map([binding.account_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        for symbol in symbols {
            let tick = ticks
                .iter()
                .find(|t| t.quote.code == symbol)
                .ok_or("尚缺持仓新报价，继续暂停估值")?;
            crate::simulation_live::validate(tick, now)?;
            let row = ctx
                .rows
                .iter()
                .find(|r| r.symbol == symbol)
                .ok_or("持仓模型证据缺失")?;
            if row
                .close
                .is_none_or(|p| (tick.quote.prev_close - p).abs() > 0.011)
            {
                return Err("持仓昨收仍不一致".into());
            }
            let raw:String=tx.query_row("SELECT f.meta_json FROM sim_orders o JOIN model_follow_orders f ON f.order_id=o.id WHERE o.account_id=?1 AND o.symbol=?2 AND o.side='buy' AND o.status='filled' ORDER BY o.id DESC LIMIT 1",params![binding.account_id,symbol],|r|r.get(0)).map_err(|_|"入场持仓证据缺失")?;
            let entry: FollowOrderMeta = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            if (row.factor / entry.factor - 1.).abs() > 1e-8 {
                return Err("持仓复权已改变，不能由报价恢复权益".into());
            }
            tx.execute("INSERT INTO sim_live_marks(account_id,symbol,price,timestamp,source) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account_id,symbol) DO UPDATE SET price=excluded.price,timestamp=excluded.timestamp,source=excluded.source",params![binding.account_id,symbol,crate::simulation_live::scaled(tick.quote.price)?,tick.quote.timestamp,tick.source]).map_err(|e|e.to_string())?;
        }
        tx.execute(
            "UPDATE model_follow_accounts SET binding_json=?1 WHERE account_id=?2",
            params![
                serde_json::to_string(&recovered).map_err(|e| e.to_string())?,
                binding.account_id
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        *binding = recovered;
        Ok(())
    }
    pub fn follow_mark(&self, account: i64, symbol: &str) -> Result<Option<(i64, i64)>, String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT price,timestamp FROM sim_live_marks WHERE account_id=?1 AND symbol=?2",
                params![account, symbol],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    // Claim each order transition once, including after restart. Delivery uses the existing alert preference.
    pub fn follow_notices(&self) -> Result<Vec<Value>, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let notices = {
            let mut stmt=tx.prepare("SELECT o.id,o.account_id,o.symbol,o.name,o.side,o.quantity,o.status,o.decision_reason,o.reject_reason,o.price,o.filled_at,f.meta_json FROM model_follow_orders f JOIN sim_orders o ON o.id=f.order_id WHERE COALESCE(f.notified_status,'')<>o.status ORDER BY o.id").map_err(|e|e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, Option<i64>>(9)?,
                        r.get::<_, Option<String>>(10)?,
                        r.get::<_, String>(11)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for item in rows {
                let (
                    id,
                    account,
                    symbol,
                    name,
                    side,
                    qty,
                    status,
                    reason,
                    _rejection,
                    price,
                    filled_at,
                    meta,
                ) = item.map_err(|e| e.to_string())?;
                let meta: FollowOrderMeta =
                    serde_json::from_str(&meta).map_err(|e| e.to_string())?;
                let action = if side == "buy" { "买入" } else { "卖出" };
                let notice = if status == "filled" {
                    let filled_price =
                        price.ok_or("已成交跟随委托缺少真实成交价格")? as f64 / 10000.;
                    let filled_at = filled_at.ok_or("已成交跟随委托缺少成交时间")?;
                    let local_time = DateTime::parse_from_rfc3339(&filled_at)
                        .map_err(|_| "已成交跟随委托的成交时间无效")?
                        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string();
                    Some(serde_json::json!({
                        "signal_kind":"research", "signal_tag":"模型跟随交易", "code":symbol,
                        "title":format!("{name} · {action}{qty}股 · 模拟已成交"),
                        "body":format!("账户 #{}；模拟成交价 {:.2}；成交时间 {}（北京时间）；{}", account, filled_price, local_time, reason.unwrap_or_default()),
                        "model_snapshot":{
                            "follow_account_id":account, "order_id":id, "status":status,
                            "context_as_of":meta.context_as_of, "source_sha256":meta.context_sha256,
                            "automatic":meta.automatic_submission, "side":side, "quantity":qty,
                            "filled_price_cny":filled_price, "filled_at":filled_at
                        }
                    }))
                } else {
                    None
                };
                out.push((id, status.clone(), notice));
            }
            out
        };
        for (id, status, _) in &notices {
            tx.execute(
                "UPDATE model_follow_orders SET notified_status=?1 WHERE order_id=?2",
                params![status, id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(notices.into_iter().filter_map(|(_, _, v)| v).collect())
    }
}

#[cfg(test)]
pub(crate) mod delete_tests {
    use super::*;
    use crate::commands::model_follow::{ContextRow, FollowContext};
    use rusqlite::types::Value as SqlValue;
    use serde_json::json;

    const RANK_SOURCE: &str = include_str!("../../../research/research-center-runner/checks/verified-rank-forward-b6cce0d8-167d-4c84-ad84-32f5b6800312.json");
    const INDEX_SOURCE: &str =
        include_str!("../../../research/follow-timing-2026-10-04/checks/index-forward.json");

    pub(crate) fn fixture() -> (Database, FollowBinding) {
        // No user database, filesystem, market feed or StockDB connection is used.
        let db = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate().unwrap();
        db.migrate_simulation().unwrap();
        db.migrate_simulation_live().unwrap();
        db.migrate_strategies().unwrap();
        db.migrate_research_loop().unwrap();
        db.migrate_model_research().unwrap();
        db.migrate_model_follow().unwrap();
        db.migrate_research_jobs().unwrap();
        let binding = add_follow(&db, RANK_SOURCE);
        (db, binding)
    }

    fn add_follow(db: &Database, raw: &str) -> FollowBinding {
        let source_run_id = db.save_model_run(raw, None).unwrap();
        let run = db.model_run(source_run_id).unwrap();
        let ctx = FollowContext {
            schema: "frozen-model-execution-v1".into(),
            as_of: run["as_of"].as_str().unwrap().into(),
            model_id: run["model_id"].as_str().unwrap().into(),
            model_sha256: run["model_sha256"].as_str().unwrap().into(),
            source_run_sha256: run["content_sha256"].as_str().unwrap().into(),
            source_runner_sha256: run["runner_sha256"].as_str().unwrap().into(),
            context_runner_sha256: "isolated-delete-test".into(),
            data_sha256: json!({}),
            session_dates: vec!["2026-09-18".into(), "2026-09-21".into()],
            ranked_symbols: vec!["sh600000".into()],
            market: None,
            rows: vec![ContextRow {
                symbol: "sh600000".into(),
                close: Some(10.),
                adjusted_close: Some(10.),
                atr14: Some(0.2),
                ma10: Some(10.),
                factor: 1.,
                amount: Some(1000000.),
                valid: true,
                is_st: Some(0),
            }],
        };
        let mut binding = FollowBinding {
            account_id: 0,
            source_run_id,
            model_id: ctx.model_id.clone(),
            model_name: "删除接口隔离测试".into(),
            max_positions: 3,
            allocation_policy: "baseline8".into(),
            execution_policy: if ctx.model_id == "index26_h20" { "index_support" } else { "baseline" }.into(),
            enabled: true,
            auto_execute: true,
            initial_cash_cny: 100000.,
            as_of: ctx.as_of.clone(),
            source_sha256: ctx.source_run_sha256.clone(),
            context_json: serde_json::to_string(&ctx).unwrap(),
            state: "waiting_session".into(),
            message: "内存数据库测试".into(),
            allocation_date: None,
            allocation_equity: None,
            valuation_blocked: false,
            valuation_block_reason: None,
            candidate_cursor: 0,
        };
        db.create_follow_account(&mut binding).unwrap();
        db.enable_model_observation(source_run_id, true).unwrap();
        binding
    }

    fn manual_account(db: &Database, name: &str) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO sim_accounts(name,initial_cash,cash,mode,created_at,updated_at) VALUES(?1,1000000000,1000000000,'record','test','test')", [name]).unwrap();
        conn.last_insert_rowid()
    }

    pub(crate) fn seed_ledger(db: &Database, account: i64) -> Vec<i64> {
        let binding = if db.is_follow_account(account).unwrap() {
            Some(db.follow_binding(account).unwrap())
        } else {
            None
        };
        let conn = db.conn.lock().unwrap();
        if binding.is_none() {
            conn.execute(
                "INSERT INTO sim_live_accounts(account_id,message) VALUES(?1,'test')",
                [account],
            )
            .unwrap();
            conn.execute("INSERT INTO sim_live_plans(account_id,symbol,plan_json) VALUES(?1,'sh600000','{}')", [account]).unwrap();
            conn.execute(
                "INSERT INTO sim_targets(account_id,symbol,rule) VALUES(?1,'sh600000','test')",
                [account],
            )
            .unwrap();
        }
        let mut orders = Vec::new();
        for status in ["filled", "pending", "awaiting_confirmation"] {
            let symbol = if status == "filled" {
                "sh600000"
            } else {
                "sz000001"
            };
            conn.execute("INSERT INTO sim_orders(account_id,idempotency_key,symbol,side,quantity,signal_date,source,limit_bps,status,created_at) VALUES(?1,?2,?3,'buy',100,'2026-09-21','rule',1000,?2,'2026-09-21T01:30:00Z')", params![account, status, symbol]).unwrap();
            let order_id = conn.last_insert_rowid();
            conn.execute("INSERT INTO sim_live_orders(order_id,limit_price,submitted_at) VALUES(?1,100000,1790000000)", [order_id]).unwrap();
            if let Some(b) = &binding {
                let meta = FollowOrderMeta {
                    order_id,
                    limit_price: 100000,
                    estimated_fee: 50000,
                    factor: 1.,
                    atr: 0.2,
                    quote_at: 1790000000,
                    valid_until: i64::MAX,
                    context_as_of: b.as_of.clone(),
                    context_sha256: b.source_sha256.clone(),
                    allocation_policy: b.allocation_policy.clone(),
                    execution_policy: b.execution_policy.clone(),
                    automatic_submission: true,
                };
                conn.execute("INSERT INTO model_follow_orders(order_id,meta_json,notified_status) VALUES(?1,?2,?3)", params![order_id, serde_json::to_string(&meta).unwrap(), status]).unwrap();
            }
            if status == "filled" {
                conn.execute("UPDATE sim_orders SET price=100000,gross=10000000,fee=50000,cash_delta=-10050000,cost_basis=10050000,filled_at='2026-09-21T01:30:01Z' WHERE id=?1", [order_id]).unwrap();
            }
            orders.push(order_id);
        }
        conn.execute(
            "UPDATE sim_accounts SET cash=cash-10050000 WHERE id=?1",
            [account],
        )
        .unwrap();
        conn.execute("INSERT INTO sim_lots(account_id,symbol,quantity,unit_cost,cost_basis,acquired_date) VALUES(?1,'sh600000',100,100500,10050000,'2026-09-21')", [account]).unwrap();
        conn.execute("INSERT INTO sim_runs(account_id,run_key,status,created_at) VALUES(?1,'test','complete','test')", [account]).unwrap();
        conn.execute("INSERT INTO sim_capital_adjustments(account_id,old_initial_cash,new_initial_cash,delta,cash_before,cash_after,created_at) VALUES(?1,900000000,1000000000,100000000,890000000,990000000,'test')", [account]).unwrap();
        conn.execute("INSERT INTO sim_equity_daily(account_id,trade_date,equity) VALUES(?1,'2026-09-21',999950000)", [account]).unwrap();
        conn.execute("INSERT INTO sim_live_equity(account_id,minute,equity,low_equity,high_equity) VALUES(?1,'2026-09-21T09:31',999950000,999950000,999950000)", [account]).unwrap();
        conn.execute("INSERT INTO sim_live_marks(account_id,symbol,price,timestamp,source) VALUES(?1,'sh600000',100000,1790000001,'test')", [account]).unwrap();
        conn.execute(
            "INSERT INTO sim_live_risk(account_id,peak,max_drawdown_bps) VALUES(?1,1000000000,1)",
            [account],
        )
        .unwrap();
        conn.execute("INSERT INTO sim_live_consumed(account_id,symbol,timestamp) VALUES(?1,'sh600000',1790000001)", [account]).unwrap();
        orders
    }

    // Discover every table, so preservation and orphan assertions also cover schema additions.
    fn snapshot(db: &Database, account: Option<i64>) -> Vec<(String, Vec<Vec<SqlValue>>)> {
        let conn = db.conn.lock().unwrap();
        let tables = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
            .query_map([], |row| row.get::<_, String>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let mut out = Vec::new();
        for table in tables {
            let columns = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let filter = match account {
                None => String::new(),
                Some(id) if table == "sim_accounts" => format!(" WHERE id={id}"),
                Some(id) if columns.iter().any(|c| c == "account_id") => {
                    format!(" WHERE account_id={id}")
                }
                Some(id) if columns.iter().any(|c| c == "order_id") => {
                    format!(" WHERE order_id IN (SELECT id FROM sim_orders WHERE account_id={id})")
                }
                Some(_) => continue,
            };
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM {table}{filter} ORDER BY rowid"))
                .unwrap();
            let count = stmt.column_count();
            let rows = stmt
                .query_map([], |row| {
                    (0..count)
                        .map(|i| row.get::<_, SqlValue>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            out.push((table, rows));
        }
        out
    }

    pub(crate) fn assert_deleted(db: &Database, account: i64, orders: &[i64]) {
        assert!(snapshot(db, Some(account))
            .iter()
            .all(|(_, rows)| rows.is_empty()));
        assert!(db.get_sim_detail(account).is_err());
        assert!(db.follow_binding(account).is_err());
        assert!(!db
            .follow_bindings()
            .unwrap()
            .iter()
            .any(|b| b.account_id == account));
        assert!(!db.list_auto_sim_account_ids().unwrap().contains(&account));
        assert!(!db.live_account(account).unwrap());
        let conn = db.conn.lock().unwrap();
        for order in orders {
            for table in ["sim_orders", "sim_live_orders", "model_follow_orders"] {
                let column = if table == "sim_orders" {
                    "id"
                } else {
                    "order_id"
                };
                let count: i64 = conn
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {table} WHERE {column}=?1"),
                        [order],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(count, 0, "orphan in {table}");
            }
        }
        let violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
    }

    #[test]
    fn follow_delete_cleans_owned_assets_and_preserves_other_ledgers_with_fk_on_or_off() {
        for foreign_keys in [true, false] {
            let (db, binding) = fixture();
            assert!(db
                .conn
                .lock()
                .unwrap()
                .pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
                .unwrap());
            let other = add_follow(&db, INDEX_SOURCE);
            let manual = manual_account(&db, "manual");
            let research = manual_account(&db, "research source");
            let comparison = manual_account(&db, "research comparison");
            let mut preset = crate::datasource::eastmoney_universe::preset_infos().remove(0);
            preset.id = "follow-delete-research".into();
            let experiment = db
                .register_experiment(
                    &preset,
                    &crate::db::research_loop::ResearchConfig::default(),
                    "isolated-test",
                )
                .unwrap();
            db.link_experiment(experiment, research, "[]").unwrap();
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO research_daily_comparison(experiment_id,account_id) VALUES(?1,?2)",
                    params![experiment, comparison],
                )
                .unwrap();
            let orders = seed_ledger(&db, binding.account_id);
            for id in [other.account_id, manual, research, comparison] {
                seed_ledger(&db, id);
            }
            // Old schemas may contain manual slots before isolation triggers were added.
            db.conn.lock().unwrap().execute_batch("DROP TRIGGER follow_isolate_sim_live_plans_insert; DROP TRIGGER follow_isolate_sim_targets_insert;").unwrap();
            db.conn.lock().unwrap().execute("INSERT INTO sim_live_plans(account_id,symbol,plan_json) VALUES(?1,'sz000001','{}')", [binding.account_id]).unwrap();
            db.conn.lock().unwrap().execute("INSERT INTO sim_targets(account_id,symbol,rule) VALUES(?1,'sz000001','legacy')", [binding.account_id]).unwrap();
            let preserved: Vec<_> = [other.account_id, manual, research, comparison]
                .iter()
                .map(|id| (*id, snapshot(&db, Some(*id))))
                .collect();
            let source_raw = db.model_run_bundle(binding.source_run_id).unwrap();
            let other_source = db.model_run(other.source_run_id).unwrap();
            db.conn
                .lock()
                .unwrap()
                .pragma_update(None, "foreign_keys", foreign_keys)
                .unwrap();
            db.delete_follow_account(binding.account_id).unwrap();
            assert_deleted(&db, binding.account_id, &orders);
            assert!(db.save_follow_binding(&binding).is_err());
            assert_deleted(&db, binding.account_id, &orders);
            for (id, before) in preserved {
                assert_eq!(snapshot(&db, Some(id)), before);
            }
            assert_eq!(
                db.model_run_bundle(binding.source_run_id).unwrap(),
                source_raw
            );
            assert_eq!(
                db.model_run(binding.source_run_id).unwrap()["enabled"],
                false
            );
            assert_eq!(db.model_run(other.source_run_id).unwrap(), other_source);
            assert!(!db
                .enabled_model_runs()
                .unwrap()
                .iter()
                .any(|run| run["id"] == binding.source_run_id));
        }
    }

    #[test]
    fn follow_delete_rejects_missing_manual_and_research_accounts_without_other_writes() {
        let (db, binding) = fixture();
        seed_ledger(&db, binding.account_id);
        let manual = manual_account(&db, "manual");
        seed_ledger(&db, manual);
        let before = snapshot(&db, None);
        for id in [i64::MAX, manual] {
            assert!(db
                .delete_follow_account(id)
                .unwrap_err()
                .contains("专用账户不存在"));
            assert_eq!(snapshot(&db, None), before);
        }
        // Even a pre-isolation source cross-link must be protected with FK enforcement off.
        let mut preset = crate::datasource::eastmoney_universe::preset_infos().remove(0);
        preset.id = "follow-delete-corrupt-source".into();
        let experiment = db
            .register_experiment(
                &preset,
                &crate::db::research_loop::ResearchConfig::default(),
                "isolated-test",
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER follow_isolate_research_experiments_update;")
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE research_experiments SET account_id=?1 WHERE id=?2",
                params![binding.account_id, experiment],
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        let before = snapshot(&db, None);
        assert!(db
            .delete_follow_account(binding.account_id)
            .unwrap_err()
            .contains("研究源账户"));
        assert_eq!(snapshot(&db, None), before);
    }

    #[test]
    fn follow_delete_rolls_back_cancellations_children_and_source_flag_on_failure() {
        for fail_at_source_flag in [false, true] {
            let (db, binding) = fixture();
            seed_ledger(&db, binding.account_id);
            let trigger = if fail_at_source_flag {
                format!("CREATE TRIGGER block_delete BEFORE UPDATE OF enabled ON model_research_runs WHEN OLD.id={} BEGIN SELECT RAISE(ABORT,'source flag failure'); END;", binding.source_run_id)
            } else {
                format!("CREATE TRIGGER block_delete BEFORE DELETE ON sim_accounts WHEN OLD.id={} BEGIN SELECT RAISE(ABORT,'account delete failure'); END;", binding.account_id)
            };
            db.conn.lock().unwrap().execute_batch(&trigger).unwrap();
            let before = snapshot(&db, None);
            assert!(db
                .delete_follow_account(binding.account_id)
                .unwrap_err()
                .contains("failure"));
            assert_eq!(snapshot(&db, None), before);
            db.conn
                .lock()
                .unwrap()
                .execute_batch("DROP TRIGGER block_delete;")
                .unwrap();
            db.delete_follow_account(binding.account_id).unwrap();
        }
    }

    #[test]
    fn follow_delete_keeps_source_for_other_follow_or_enabled_matching_manual_watch() {
        // (remaining follow enabled, watch enabled/model match/hash match, keep maintenance)
        let cases = [
            (None, None, false),
            (Some(true), None, true),
            (Some(false), None, true),
            (None, Some((true, true, true)), true),
            (None, Some((false, true, true)), false),
            (None, Some((true, false, true)), false),
            (None, Some((true, true, false)), false),
        ];
        for (follow_enabled, watch, keep) in cases {
            let (db, binding) = fixture();
            if let Some(enabled) = follow_enabled {
                let mut other = binding.clone();
                other.account_id = 0;
                // Simulate a legacy duplicate already on disk; new setup now refuses it.
                other.account_id = manual_account(&db, "legacy-follow");
                let conn = db.conn.lock().unwrap();
                conn.execute("INSERT INTO model_follow_accounts(account_id,source_run_id,binding_json) VALUES(?1,?2,?3)", params![other.account_id,other.source_run_id,serde_json::to_string(&other).unwrap()]).unwrap();
                drop(conn);
                other.enabled = enabled;
                db.save_follow_binding_for_user(&other).unwrap();
            }
            let source = db.model_run(binding.source_run_id).unwrap();
            if let Some((enabled, same_model, same_sha)) = watch {
                let config = json!({"model_id":if same_model {source["model_id"].clone()} else {json!("index26_h20")},"model_sha256":if same_sha {source["model_sha256"].clone()} else {json!("different")},"runner_sha256":"different-runner-intentionally"});
                let id = db.save_condition_watch("manual-watch", &config).unwrap();
                db.enable_condition_watch(id, enabled).unwrap();
            }
            let watches_before = db.condition_watches().unwrap();
            db.delete_follow_account(binding.account_id).unwrap();
            let mut expected = source;
            expected["enabled"] = json!(keep);
            assert_eq!(db.model_run(binding.source_run_id).unwrap(), expected);
            assert_eq!(db.condition_watches().unwrap(), watches_before);
            assert_eq!(
                db.enabled_model_runs()
                    .unwrap()
                    .iter()
                    .any(|r| r["id"] == binding.source_run_id),
                keep
            );
        }
    }

    #[test]
    fn follow_delete_can_restart_with_new_empty_account_and_old_binding_cannot_return() {
        let (db, binding) = fixture();
        let orders = seed_ledger(&db, binding.account_id);
        db.delete_follow_account(binding.account_id).unwrap();
        assert!(db.save_follow_binding(&binding).is_err());
        let mut fresh = binding.clone();
        fresh.account_id = 0;
        fresh.initial_cash_cny = 120000.;
        fresh.allocation_date = None;
        fresh.allocation_equity = None;
        db.create_follow_account(&mut fresh).unwrap();
        db.enable_model_observation(fresh.source_run_id, true)
            .unwrap();
        assert!(fresh.account_id > binding.account_id);
        let detail = db.get_sim_detail(fresh.account_id).unwrap();
        assert_eq!(detail.account.current_cash, "1200000000");
        assert!(
            detail.positions.is_empty()
                && detail.orders.is_empty()
                && detail.capital_adjustments.is_empty()
        );
        assert_eq!(db.follow_bindings().unwrap().len(), 1);
        assert!(db
            .enabled_model_runs()
            .unwrap()
            .iter()
            .any(|r| r["id"] == fresh.source_run_id));
        assert_deleted(&db, binding.account_id, &orders);
    }
}
