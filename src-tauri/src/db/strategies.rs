use super::Database;
use crate::datasource::eastmoney_universe::{MarketFilter, PresetInfo};
use crate::domain::KLineData;
use crate::quant::playbook::TradeRule;
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

const PRESET_SETTING_KEY: &str = "universe_custom_presets";
const ENGINE_REVISION: &str = "strategy-v1";
const REQUIRED_STAGES: [&str; 6] = [
    "static",
    "causal",
    "historical",
    "admission",
    "forward",
    "probation",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StrategyDefinition {
    label: String,
    description: String,
    filter: MarketFilter,
    rule: String,
}

impl From<&PresetInfo> for StrategyDefinition {
    fn from(value: &PresetInfo) -> Self {
        Self {
            label: value.label.clone(),
            description: value.description.clone(),
            filter: value.filter.clone(),
            rule: value.rule.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyStageView {
    pub stage: String,
    pub status: String,
    pub detail: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyCardView {
    pub id: String,
    pub name: String,
    pub source: String,
    pub current_version: i64,
    pub active_version_id: Option<i64>,
    pub version_id: i64,
    pub status: String,
    pub rule: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub stages: Vec<StrategyStageView>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyCounts {
    pub candidate: usize,
    pub trial: usize,
    pub rejected: usize,
    pub active: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyLibrary {
    pub cards: Vec<StrategyCardView>,
    pub counts: StrategyCounts,
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn sample_bars() -> Vec<KLineData> {
    (0..180)
        .map(|index| {
            let close = 10.0 + index as f64 * 0.03 + (index as f64 * 0.17).sin();
            KLineData {
                date: format!("2025-{:02}-{:02}", index / 28 + 1, index % 28 + 1),
                open: close,
                high: close + 0.2,
                low: close - 0.2,
                close,
                volume: 10_000,
                turnover: 1.0,
            }
        })
        .collect()
}

fn prechecks(definition: &StrategyDefinition) -> (String, Vec<(&'static str, String, String)>) {
    let rule = TradeRule::try_from_id(&definition.rule);
    let static_result = rule
        .as_ref()
        .map(|_| ("passed", "结构化字段与交易规则合法".to_string()))
        .unwrap_or_else(|error| ("failed", error.clone()));
    let causal_result = rule
        .ok()
        .map(|rule| crate::quant::causal::audit(&sample_bars(), rule))
        .map(|audit| {
            if audit.passed {
                ("passed", audit.message)
            } else {
                ("failed", audit.message)
            }
        })
        .unwrap_or(("blocked", "静态校验失败，未运行因果审计".into()));
    let rejected = static_result.0 == "failed" || causal_result.0 == "failed";
    (
        if rejected { "rejected" } else { "candidate" }.into(),
        vec![
            ("static", static_result.0.into(), static_result.1),
            ("causal", causal_result.0.into(), causal_result.1),
            (
                "historical",
                "blocked".into(),
                "排名7尚无可通过的时点全市场组合、完整可交易性和公司行为账本".into(),
            ),
            (
                "admission",
                "pending".into(),
                "等待历史硬门禁全部通过".into(),
            ),
            (
                "forward",
                "pending".into(),
                "只接受策略版本登记后产生的隔离前向证据".into(),
            ),
            (
                "probation",
                "pending".into(),
                "未达到考核期前不给通过结论".into(),
            ),
        ],
    )
}

fn ensure_version(tx: &Transaction<'_>, preset: &PresetInfo) -> Result<(i64, i64, String), String> {
    let source = if preset.builtin { "builtin" } else { "user" };
    let definition = StrategyDefinition::from(preset);
    let definition_json = serde_json::to_string(&definition).map_err(|error| error.to_string())?;
    let existing: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT v.id,v.version,v.definition_json
             FROM strategy_cards c JOIN strategy_versions v ON v.card_id=c.id AND v.version=c.current_version
             WHERE c.id=?1",
            [&preset.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((version_id, version, stored)) = existing {
        if stored == definition_json {
            return Ok((version_id, version, String::new()));
        }
    }

    let timestamp = now();
    tx.execute(
        "INSERT INTO strategy_cards(id,name,source,current_version,created_at,updated_at)
         VALUES(?1,?2,?3,0,?4,?4)
         ON CONFLICT(id) DO UPDATE SET name=excluded.name,source=excluded.source,updated_at=excluded.updated_at",
        params![preset.id, preset.label, source, timestamp],
    )
    .map_err(|error| error.to_string())?;
    let version: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(version),0)+1 FROM strategy_versions WHERE card_id=?1",
            [&preset.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let (status, checks) = prechecks(&definition);
    tx.execute(
        "INSERT INTO strategy_versions(card_id,version,definition_json,engine_revision,status,revision,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,0,?6,?6)",
        params![preset.id, version, definition_json, ENGINE_REVISION, status, timestamp],
    )
    .map_err(|error| error.to_string())?;
    let version_id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE strategy_cards SET current_version=?2,updated_at=?3 WHERE id=?1",
        params![preset.id, version, timestamp],
    )
    .map_err(|error| error.to_string())?;
    for (stage, stage_status, detail) in checks {
        tx.execute(
            "INSERT INTO strategy_stage_runs(version_id,stage,input_fingerprint,status,detail,checked_at)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![version_id, stage, format!("{ENGINE_REVISION}:{version_id}"), stage_status, detail, timestamp],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.execute(
        "INSERT INTO strategy_events(version_id,from_state,to_state,actor,reason,created_at)
         VALUES(?1,NULL,?2,'system','登记不可变策略版本',?3)",
        params![version_id, status, timestamp],
    )
    .map_err(|error| error.to_string())?;
    Ok((version_id, version, status))
}

impl Database {
    pub fn migrate_strategies(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS strategy_cards(
               id TEXT PRIMARY KEY,name TEXT NOT NULL,source TEXT NOT NULL,
               current_version INTEGER NOT NULL DEFAULT 0,active_version_id INTEGER,
               created_at TEXT NOT NULL,updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS strategy_versions(
               id INTEGER PRIMARY KEY AUTOINCREMENT,card_id TEXT NOT NULL,version INTEGER NOT NULL,
               definition_json TEXT NOT NULL,engine_revision TEXT NOT NULL,status TEXT NOT NULL,
               revision INTEGER NOT NULL DEFAULT 0,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,
               UNIQUE(card_id,version),FOREIGN KEY(card_id) REFERENCES strategy_cards(id) ON DELETE RESTRICT
             );
             CREATE TABLE IF NOT EXISTS strategy_stage_runs(
               id INTEGER PRIMARY KEY AUTOINCREMENT,version_id INTEGER NOT NULL,stage TEXT NOT NULL,
               input_fingerprint TEXT NOT NULL,status TEXT NOT NULL,detail TEXT NOT NULL,checked_at TEXT NOT NULL,
               UNIQUE(version_id,stage,input_fingerprint),
               FOREIGN KEY(version_id) REFERENCES strategy_versions(id) ON DELETE RESTRICT
             );
             CREATE TABLE IF NOT EXISTS strategy_events(
               id INTEGER PRIMARY KEY AUTOINCREMENT,version_id INTEGER NOT NULL,from_state TEXT,to_state TEXT NOT NULL,
               actor TEXT NOT NULL,reason TEXT NOT NULL,created_at TEXT NOT NULL,
               FOREIGN KEY(version_id) REFERENCES strategy_versions(id) ON DELETE RESTRICT
             );",
        )
    }

    pub fn sync_strategy_presets(&self, presets: &[PresetInfo]) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        for preset in presets {
            ensure_version(&tx, preset)?;
        }
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn replace_custom_strategy_presets(
        &self,
        presets: &[PresetInfo],
        retired_id: Option<&str>,
    ) -> Result<(), String> {
        let json = serde_json::to_string(presets).map_err(|error| error.to_string())?;
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
            params![PRESET_SETTING_KEY, json],
        )
        .map_err(|error| error.to_string())?;
        for preset in presets {
            ensure_version(&tx, preset)?;
        }
        if let Some(card_id) = retired_id {
            let current: Option<(i64, String)> = tx
                .query_row(
                    "SELECT v.id,v.status FROM strategy_cards c JOIN strategy_versions v ON v.card_id=c.id AND v.version=c.current_version WHERE c.id=?1",
                    [card_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if let Some((version_id, from)) = current {
                let timestamp = now();
                tx.execute(
                    "UPDATE strategy_versions SET status='retired',revision=revision+1,updated_at=?2 WHERE id=?1",
                    params![version_id, timestamp],
                )
                .map_err(|error| error.to_string())?;
                tx.execute(
                    "UPDATE strategy_cards SET active_version_id=NULL,updated_at=?2 WHERE id=?1",
                    params![card_id, timestamp],
                )
                .map_err(|error| error.to_string())?;
                tx.execute(
                    "INSERT INTO strategy_events(version_id,from_state,to_state,actor,reason,created_at) VALUES(?1,?2,'retired','user','从筛选器退役，保留全部历史',?3)",
                    params![version_id, from, timestamp],
                )
                .map_err(|error| error.to_string())?;
            }
        }
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn strategy_library(&self) -> Result<StrategyLibrary, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut stmt = conn
            .prepare(
                "SELECT c.id,c.name,c.source,c.current_version,c.active_version_id,
                        v.id,v.status,v.definition_json,v.revision,v.created_at,v.updated_at
                 FROM strategy_cards c JOIN strategy_versions v ON v.card_id=c.id AND v.version=c.current_version
                 ORDER BY c.created_at,c.id",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(stmt);
        let mut cards = Vec::with_capacity(rows.len());
        for (
            id,
            name,
            source,
            current_version,
            active_version_id,
            version_id,
            status,
            json,
            revision,
            created_at,
            updated_at,
        ) in rows
        {
            let definition: StrategyDefinition =
                serde_json::from_str(&json).map_err(|error| error.to_string())?;
            let mut stages = conn
                .prepare("SELECT stage,status,detail,checked_at FROM strategy_stage_runs WHERE version_id=?1 ORDER BY id")
                .map_err(|error| error.to_string())?
                .query_map([version_id], |row| {
                    Ok(StrategyStageView { stage: row.get(0)?, status: row.get(1)?, detail: row.get(2)?, checked_at: row.get(3)? })
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            stages.sort_by_key(|item| {
                REQUIRED_STAGES
                    .iter()
                    .position(|stage| *stage == item.stage)
                    .unwrap_or(usize::MAX)
            });
            cards.push(StrategyCardView {
                id,
                name,
                source,
                current_version,
                active_version_id,
                version_id,
                status,
                rule: definition.rule,
                revision,
                created_at,
                updated_at,
                stages,
            });
        }
        let mut counts = StrategyCounts::default();
        for card in &cards {
            match card.status.as_str() {
                "candidate" => counts.candidate += 1,
                "trial" | "probation" | "awaiting_confirmation" => counts.trial += 1,
                "active" | "degraded" => counts.active += 1,
                "rejected" | "retired" => counts.rejected += 1,
                _ => {}
            }
        }
        Ok(StrategyLibrary { cards, counts })
    }

    pub fn transition_strategy(
        &self,
        version_id: i64,
        action: &str,
        confirmed: bool,
        expected_revision: i64,
    ) -> Result<StrategyCardView, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let (card_id, from, revision): (String, String, i64) = tx
            .query_row(
                "SELECT card_id,status,revision FROM strategy_versions WHERE id=?1",
                [version_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "策略版本不存在".to_string())?;
        if revision != expected_revision {
            return Err("策略状态已在别处变化，请刷新后重试".into());
        }
        let to = match action {
            "activate" => {
                if !confirmed {
                    return Err("采用策略必须由用户明确确认".into());
                }
                let missing: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM strategy_stage_runs WHERE version_id=?1 AND stage IN ('static','causal','historical','admission','forward','probation') AND status!='passed'",
                        [version_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                if missing > 0 {
                    return Err("仍有验证或考核门禁未通过，不能采用".into());
                }
                "active"
            }
            "degrade" if from == "active" => "degraded",
            "pause" if matches!(from.as_str(), "active" | "degraded") => "paused",
            _ => return Err(format!("不允许从 {from} 执行 {action}")),
        };
        let timestamp = now();
        let changed = tx
            .execute(
                "UPDATE strategy_versions SET status=?2,revision=revision+1,updated_at=?3 WHERE id=?1 AND revision=?4",
                params![version_id, to, timestamp, expected_revision],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("策略状态已在别处变化，请刷新后重试".into());
        }
        if to == "active" {
            tx.execute(
                "UPDATE strategy_versions SET status='superseded',revision=revision+1,updated_at=?3 WHERE card_id=?1 AND id!=?2 AND status IN ('active','degraded')",
                params![card_id, version_id, timestamp],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "UPDATE strategy_cards SET active_version_id=?2,updated_at=?3 WHERE id=?1",
                params![card_id, version_id, timestamp],
            )
            .map_err(|error| error.to_string())?;
        } else if to == "paused" {
            tx.execute(
                "UPDATE strategy_cards SET active_version_id=NULL,updated_at=?2 WHERE id=?1 AND active_version_id=?3",
                params![card_id, timestamp, version_id],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.execute(
            "INSERT INTO strategy_events(version_id,from_state,to_state,actor,reason,created_at) VALUES(?1,?2,?3,'user','显式状态变更',?4)",
            params![version_id, from, to, timestamp],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        self.strategy_library()?
            .cards
            .into_iter()
            .find(|card| card.version_id == version_id)
            .ok_or_else(|| "状态变更后未找到策略版本".into())
    }
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
        database.migrate_strategies().unwrap();
        database
    }

    fn preset(label: &str) -> PresetInfo {
        PresetInfo {
            id: "custom_test".into(),
            label: label.into(),
            description: String::new(),
            filter: MarketFilter::default(),
            rule: "trend_follow".into(),
            builtin: false,
            strategy_version_id: None,
            strategy_version: 0,
            strategy_status: String::new(),
        }
    }

    #[test]
    fn overwrite_creates_version_and_retire_keeps_history() {
        let db = database();
        db.replace_custom_strategy_presets(&[preset("版本一")], None)
            .unwrap();
        db.replace_custom_strategy_presets(&[preset("版本二")], None)
            .unwrap();
        let card = db.strategy_library().unwrap().cards.pop().unwrap();
        assert_eq!(card.current_version, 2);
        db.replace_custom_strategy_presets(&[], Some("custom_test"))
            .unwrap();
        let library = db.strategy_library().unwrap();
        assert_eq!(library.cards[0].status, "retired");
        let conn = db.conn.lock().unwrap();
        let versions: i64 = conn
            .query_row("SELECT COUNT(*) FROM strategy_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(versions, 2);
    }

    #[test]
    fn cannot_activate_before_every_gate_and_confirmation() {
        let db = database();
        db.replace_custom_strategy_presets(&[preset("候选")], None)
            .unwrap();
        let card = db.strategy_library().unwrap().cards.pop().unwrap();
        assert!(db
            .transition_strategy(card.version_id, "activate", false, card.revision)
            .is_err());
        assert!(db
            .transition_strategy(card.version_id, "activate", true, card.revision)
            .is_err());
    }

    #[test]
    fn invalid_rule_is_rejected() {
        let db = database();
        let mut invalid = preset("坏策略");
        invalid.rule = "future_rule".into();
        db.replace_custom_strategy_presets(&[invalid], None)
            .unwrap();
        let card = db.strategy_library().unwrap().cards.pop().unwrap();
        assert_eq!(card.status, "rejected");
        assert_eq!(card.stages[0].status, "failed");
        assert_eq!(card.stages[1].status, "blocked");
    }
}
