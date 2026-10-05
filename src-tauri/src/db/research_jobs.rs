//! Persistent research work; independent from brokerage orders and legacy experiments.
use super::Database;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

fn read_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let request: String = row.get(3)?;
    let context: String = row.get(4)?;
    let results: String = row.get(5)?;
    Ok(
        json!({"id":row.get::<_,i64>(0)?,"kind":row.get::<_,String>(1)?,"state":row.get::<_,String>(2)?,
        "request":serde_json::from_str::<Value>(&request).unwrap_or(Value::Null),
        "context":serde_json::from_str::<Value>(&context).unwrap_or(Value::Null),
        "results":serde_json::from_str::<Value>(&results).unwrap_or(Value::Null),
        "fingerprint":row.get::<_,Option<String>>(6)?,"phase":row.get::<_,String>(7)?,
        "completed":row.get::<_,i64>(8)?,"total":row.get::<_,i64>(9)?,"message":row.get::<_,String>(10)?,
        "cancel_requested":row.get::<_,bool>(11)?,"created_at":row.get::<_,String>(12)?,"updated_at":row.get::<_,String>(13)?}),
    )
}
const SELECT_JOB: &str = "SELECT id,kind,state,request_json,context_json,results_json,fingerprint,phase,completed,total,message,cancel_requested,created_at,updated_at FROM research_jobs";

impl Database {
    pub fn migrate_research_jobs(&self) -> rusqlite::Result<()> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute_batch(
            "CREATE TABLE IF NOT EXISTS research_jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,kind TEXT NOT NULL,state TEXT NOT NULL,
                request_json TEXT NOT NULL,context_json TEXT NOT NULL DEFAULT 'null',
                results_json TEXT NOT NULL DEFAULT '[]',fingerprint TEXT,
                phase TEXT NOT NULL DEFAULT 'queued',completed INTEGER NOT NULL DEFAULT 0,
                total INTEGER NOT NULL,message TEXT NOT NULL DEFAULT '',cancel_requested INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
             CREATE TABLE IF NOT EXISTS model_condition_watches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,identity TEXT NOT NULL UNIQUE,config_json TEXT NOT NULL,
                state_json TEXT NOT NULL DEFAULT '{}',enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
             CREATE TABLE IF NOT EXISTS model_condition_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,event_key TEXT NOT NULL UNIQUE,watch_id INTEGER NOT NULL,
                evidence_json TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));"
        )
    }
    pub fn recover_research_jobs(&self) -> Result<usize, String> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute(
            "UPDATE research_jobs SET state='interrupted',phase='interrupted',message='应用已退出；已完成步骤保留，可在相同数据和程序版本下继续',cancel_requested=0,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state IN ('queued','running')",[]).map_err(|e|e.to_string())
    }
    pub fn create_research_job(
        &self,
        kind: &str,
        request: &Value,
        total: usize,
    ) -> Result<i64, String> {
        if !matches!(kind, "scan" | "replay" | "forward" | "compare" | "explore")
            || total == 0
            || total > 20
            || request.is_null()
        {
            return Err("研究作业参数无效".into());
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO research_jobs(kind,state,request_json,total) VALUES(?1,'queued',?2,?3)",
            params![kind, request.to_string(), total as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }
    pub fn research_job(&self, id: i64) -> Result<Value, String> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(&format!("{SELECT_JOB} WHERE id=?1"), [id], read_job)
            .map_err(|_| "研究作业不存在".into())
    }
    pub fn research_jobs(&self) -> Result<Vec<Value>, String> {
        // Poll only small summaries. Whole-universe scores stay in the full job evidence.
        let summary=SELECT_JOB.replace("context_json,results_json,", "'null',(SELECT json_group_array(json_object('key',json_extract(step.value,'$.key'),'model_id',json_extract(step.value,'$.model_id'),'run_id',json_extract(step.value,'$.run_id'),'comparison',json_extract(step.value,'$.comparison'),'holding_days',json_extract(step.value,'$.holding_days'),'metrics',json_extract(step.value,'$.metrics'),'state',json_extract(step.value,'$.state'),'as_of',json_extract(step.value,'$.as_of'),'year',json_extract(step.value,'$.year'),'evaluations',json_extract(step.value,'$.evaluations'),'accounts',json_extract(step.value,'$.accounts'),'bank_size',json_extract(step.value,'$.bank_size'))) FROM json_each(research_jobs.results_json) AS step),");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare(&format!("{summary} ORDER BY id DESC LIMIT 30"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], read_job).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn activate_research_job(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count=conn.execute("UPDATE research_jobs SET state='running',cancel_requested=0,message='',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND state IN ('queued','interrupted','cancelled','failed')",[id]).map_err(|e|e.to_string())?;
        if count != 1 {
            return Err("该研究作业正在运行或已完成".into());
        }
        Ok(())
    }
    pub fn prepare_research_job(
        &self,
        id: i64,
        context: &Value,
        fingerprint: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let old: Option<String> = conn
            .query_row(
                "SELECT fingerprint FROM research_jobs WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "作业不存在")?;
        if old.as_deref().is_some_and(|v| v != fingerprint) {
            return Err("行情或程序身份已改变，保留原作业；请新建任务，不能跨版本续算".into());
        }
        conn.execute("UPDATE research_jobs SET context_json=?2,fingerprint=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND state='running'",params![id,context.to_string(),fingerprint]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn research_job_phase(&self, id: i64, phase: &str, message: &str) -> Result<(), String> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute("UPDATE research_jobs SET phase=?2,message=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND state='running'",params![id,phase,message]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn research_job_cancelled(&self, id: i64) -> bool {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT cancel_requested FROM research_jobs WHERE id=?1",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(true)
    }
    pub fn cancel_research_job(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let changed=conn.execute("UPDATE research_jobs SET cancel_requested=1,message='正在停止该研究进程，已完成步骤保留',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND state IN ('queued','running')",[id]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("作业已结束，不能取消".into());
        }
        Ok(())
    }
    pub fn checkpoint_research_job(&self, id: i64, result: &Value) -> Result<(), String> {
        if result["key"].as_str().is_none() {
            return Err("研究步骤缺身份".into());
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let text: String = conn
            .query_row(
                "SELECT results_json FROM research_jobs WHERE id=?1 AND state='running' AND cancel_requested=0",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "作业不在运行状态")?;
        let mut rows: Vec<Value> = serde_json::from_str(&text).map_err(|_| "作业断点损坏")?;
        if rows.iter().any(|v| v["key"] == result["key"]) {
            return Err("研究步骤已完成，拒绝重复写入".into());
        }
        rows.push(result.clone());
        let count = rows.len();
        conn.execute("UPDATE research_jobs SET results_json=?2,completed=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",params![id,serde_json::to_string(&rows).map_err(|e|e.to_string())?,count as i64]).map_err(|e|e.to_string())?;
        Ok(())
    }
    /// Commit the account and its job checkpoint together. A crash cannot strand a continuation.
    pub fn save_job_model_step(
        &self,
        id: i64,
        raw: &str,
        continuation: Option<i64>,
        expected_sha: Option<&str>,
        enable_observation: bool,
        result: &Value,
    ) -> Result<i64, String> {
        let content = super::model_research::decode_run(raw)?;
        if result["key"].as_str().is_none()
            || result["model_id"] != content["model_id"]
            || result["comparison"] != content["comparison"]
            || result["holding_days"] != content["holding_days"]
        {
            return Err("作业步骤与模型账本身份不同".into());
        }
        if enable_observation && content["mode"] != "forward" {
            return Err("只有前向账户能开启自动观察".into());
        }
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let text:String=tx.query_row("SELECT results_json FROM research_jobs WHERE id=?1 AND state='running' AND cancel_requested=0",[id],|r|r.get(0)).map_err(|_|"作业已取消或不在运行状态")?;
        let mut steps: Vec<Value> = serde_json::from_str(&text).map_err(|_| "作业断点损坏")?;
        if steps.iter().any(|s| s["key"] == result["key"]) {
            return Err("该模型步骤已保存，不重复执行".into());
        }
        let run_id =
            super::model_research::save_model_run_checked(&tx, raw, continuation, expected_sha)?;
        if enable_observation {
            tx.execute(
                "UPDATE model_research_runs SET enabled=1 WHERE id=?1",
                [run_id],
            )
            .map_err(|e| e.to_string())?;
        }
        let mut step = result.clone();
        step["run_id"] = json!(run_id);
        steps.push(step);
        tx.execute("UPDATE research_jobs SET results_json=?2,completed=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",params![id,serde_json::to_string(&steps).map_err(|e|e.to_string())?,steps.len() as i64]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(run_id)
    }
    pub fn finish_research_job(&self, id: i64, state: &str, message: &str) -> Result<(), String> {
        if !matches!(state, "complete" | "cancelled" | "failed") {
            return Err("作业终态无效".into());
        }
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute("UPDATE research_jobs SET state=?2,phase=?2,message=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND state IN ('queued','running')",params![id,state,message]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn latest_model_scan_steps(&self) -> Result<Option<Value>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let value: Option<Value> = conn
            .query_row(
                &format!(
                    "{SELECT_JOB} WHERE kind='scan' AND state='complete' ORDER BY id DESC LIMIT 1"
                ),
                [],
                read_job,
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(value)
    }
    pub fn save_condition_watch(&self, identity: &str, config: &Value) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM model_condition_watches WHERE identity=?1)",
                [identity],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM model_condition_watches", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if !exists && count >= 200 {
            return Err("手动提醒已达200条；请删除不需要的提醒后再添加".into());
        }
        conn.execute("INSERT INTO model_condition_watches(identity,config_json) VALUES(?1,?2) ON CONFLICT(identity) DO UPDATE SET enabled=1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",params![identity,config.to_string()]).map_err(|e|e.to_string())?;
        conn.query_row(
            "SELECT id FROM model_condition_watches WHERE identity=?1",
            [identity],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
    }
    pub fn condition_watches(&self) -> Result<Vec<Value>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt=conn.prepare("SELECT id,config_json,state_json,enabled FROM model_condition_watches ORDER BY id DESC LIMIT 200").map_err(|e|e.to_string())?;
        let rows=stmt.query_map([],|r|{let config:String=r.get(1)?;let state:String=r.get(2)?;Ok(json!({"id":r.get::<_,i64>(0)?,"config":serde_json::from_str::<Value>(&config).unwrap_or(Value::Null),"observation":serde_json::from_str::<Value>(&state).unwrap_or(Value::Null),"enabled":r.get::<_,bool>(3)?}))}).map_err(|e|e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn enable_condition_watch(&self, id: i64, enabled: bool) -> Result<(), String> {
        let changed=self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute("UPDATE model_condition_watches SET enabled=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",params![id,enabled]).map_err(|e|e.to_string())?;
        if changed == 0 {
            return Err("观察条件不存在".into());
        }
        Ok(())
    }
    /// Removing a manual watch never changes shared model sources or trading accounts.
    /// Past immutable notification evidence remains available from the reminder history.
    pub fn delete_condition_watch(&self, id: i64) -> Result<(), String> {
        if id <= 0 { return Err("提醒编号无效".into()); }
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
            .execute("DELETE FROM model_condition_watches WHERE id=?1", [id])
            .map(|_| ()).map_err(|e| e.to_string())
    }
    /// Observation and event identity are committed together; a crash cannot create duplicate notifications.
    pub fn commit_condition_observation(
        &self,
        id: i64,
        state: &Value,
        event: Option<(&str, &Value)>,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let enabled: Option<bool> = tx
            .query_row(
                "SELECT enabled FROM model_condition_watches WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional().map_err(|e| e.to_string())?;
        // A watch can be deleted while its data is being fetched. Skip this stale
        // result without aborting the rest of the observation tick or recreating it.
        if enabled != Some(true) {
            return Ok(false);
        }
        tx.execute("UPDATE model_condition_watches SET state_json=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",params![id,state.to_string()]).map_err(|e|e.to_string())?;
        let inserted = if let Some((key, evidence)) = event {
            tx.execute("INSERT OR IGNORE INTO model_condition_events(event_key,watch_id,evidence_json) VALUES(?1,?2,?3)",params![key,id,evidence.to_string()]).map_err(|e|e.to_string())?>0
        } else {
            false
        };
        tx.commit().map_err(|e| e.to_string())?;
        Ok(inserted)
    }
    pub fn condition_event(&self, key: &str) -> Result<Value, String> {
        let text: String = self
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT evidence_json FROM model_condition_events WHERE event_key=?1",
                [key],
                |r| r.get(0),
            )
            .map_err(|_| "该次条件证据不存在；不以现在的结果代替")?;
        serde_json::from_str(&text).map_err(|_| "条件证据损坏".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jobs_survive_reopening_and_resume_only_matching_identity() {
        let path = std::env::temp_dir().join(format!("research-jobs-{}", uuid::Uuid::new_v4()));
        let db = Database::open(path.clone()).unwrap();
        let id = db
            .create_research_job("scan", &json!({"models":["breadth22_h20"]}), 2)
            .unwrap();
        db.activate_research_job(id).unwrap();
        db.prepare_research_job(id, &json!({"snapshot":"x"}), "abc")
            .unwrap();
        db.checkpoint_research_job(id, &json!({"key":"model-1","model_id":"breadth22_h20"}))
            .unwrap();
        db.cancel_research_job(id).unwrap();
        assert!(db.research_job_cancelled(id));
        drop(db);
        let db = Database::open(path).unwrap();
        assert_eq!(db.recover_research_jobs().unwrap(), 1);
        assert_eq!(db.research_job(id).unwrap()["completed"], 1);
        db.activate_research_job(id).unwrap();
        assert!(!db.research_job_cancelled(id));
        assert!(db.prepare_research_job(id, &json!({}), "changed").is_err());
        db.prepare_research_job(id, &json!({}), "abc").unwrap();
        assert!(db
            .checkpoint_research_job(id, &json!({"key":"model-1"}))
            .is_err());
        db.checkpoint_research_job(id, &json!({"key":"model-2"}))
            .unwrap();
        db.finish_research_job(id, "complete", "done").unwrap();
        assert!(db.activate_research_job(id).is_err());
        assert_eq!(
            db.latest_model_scan_steps().unwrap().unwrap()["completed"],
            2
        );
    }
    #[test]
    fn watches_deduplicate_and_event_evidence_is_atomic_and_immutable() {
        let path = std::env::temp_dir().join(format!("condition-watch-{}", uuid::Uuid::new_v4()));
        let db = Database::open(path).unwrap();
        let id = db
            .save_condition_watch("fixed", &json!({"symbol":"sz000001"}))
            .unwrap();
        assert_eq!(
            db.save_condition_watch("fixed", &json!({"symbol":"sz000001"}))
                .unwrap(),
            id
        );
        assert!(db
            .commit_condition_observation(
                id,
                &json!({"state":"confirmed"}),
                Some(("event1", &json!({"price":10})))
            )
            .unwrap());
        assert!(!db
            .commit_condition_observation(
                id,
                &json!({"state":"confirmed"}),
                Some(("event1", &json!({"price":999})))
            )
            .unwrap());
        assert_eq!(db.condition_event("event1").unwrap()["price"], 10);
        db.enable_condition_watch(id, false).unwrap();
        assert!(!db
            .commit_condition_observation(id, &json!({}), Some(("event2", &json!({}))))
            .unwrap());
        assert!(db.condition_event("event2").is_err());
    }
    #[test]
    fn deleting_a_manual_watch_persists_and_keeps_past_evidence() {
        let root = std::env::temp_dir().join(format!("condition-delete-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let id = db.save_condition_watch("remove", &json!({"symbol":"sz000001"})).unwrap();
        let other = db.save_condition_watch("keep", &json!({"symbol":"sh600000"})).unwrap();
        assert!(db.commit_condition_observation(id, &json!({"state":"confirmed"}), Some(("past-notice", &json!({"price":10})))).unwrap());
        db.enable_condition_watch(id, false).unwrap();
        db.delete_condition_watch(id).unwrap();
        db.delete_condition_watch(id).unwrap(); // A repeated click/request is harmless.
        assert_eq!(db.condition_watches().unwrap().len(), 1);
        assert_eq!(db.condition_watches().unwrap()[0]["id"], other);
        assert!(db.enable_condition_watch(id, true).is_err());
        assert_eq!(db.condition_event("past-notice").unwrap()["price"], 10);
        drop(db);
        let reopened = Database::open(root.clone()).unwrap();
        assert!(reopened.condition_watches().unwrap().iter().all(|watch|watch["id"] != id));
        let replacement = reopened.save_condition_watch("remove", &json!({"symbol":"sz000001"})).unwrap();
        assert!(replacement > id, "Re-adding explicitly creates a fresh watch, not the deleted monitoring state");
        assert_eq!(reopened.condition_event("past-notice").unwrap()["price"], 10);
        drop(reopened); std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn an_in_flight_result_cannot_recreate_a_deleted_watch_or_abort_other_watches() {
        let root = std::env::temp_dir().join(format!("condition-delete-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let stale = db.save_condition_watch("stale", &json!({"symbol":"sz000001"})).unwrap();
        let next = db.save_condition_watch("next", &json!({"symbol":"sh600000"})).unwrap();
        let listed = db.condition_watches().unwrap();
        assert!(listed.iter().any(|watch|watch["id"]==stale));
        db.delete_condition_watch(stale).unwrap(); // Simulate deletion during a network fetch.
        assert!(!db.commit_condition_observation(stale, &json!({"state":"confirmed"}), Some(("late-notice", &json!({})))).unwrap());
        assert!(db.condition_event("late-notice").is_err());
        assert!(db.commit_condition_observation(next, &json!({"state":"confirmed"}), Some(("next-notice", &json!({"symbol":"sh600000"})))).unwrap());
        assert_eq!(db.condition_watches().unwrap().len(),1);
        assert_eq!(db.condition_event("next-notice").unwrap()["symbol"],"sh600000");
        drop(db); std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn deleting_manual_watches_releases_capacity_without_removing_shared_research() {
        let root = std::env::temp_dir().join(format!("condition-delete-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let job = db.create_research_job("scan", &json!({"models":["breadth22_h20"]}), 1).unwrap();
        let before = db.research_job(job).unwrap();
        let mut first = 0;
        for n in 0..200 {
            let id = db.save_condition_watch(&format!("watch-{n}"), &json!({"symbol":"sz000001"})).unwrap();
            if n==0 { first=id; }
        }
        assert!(db.save_condition_watch("over-capacity", &json!({})).is_err());
        db.enable_condition_watch(first,false).unwrap();
        assert!(db.save_condition_watch("over-capacity", &json!({})).is_err(), "Pausing alone does not free a list slot");
        assert!(db.delete_condition_watch(0).is_err());
        db.delete_condition_watch(first).unwrap();
        db.save_condition_watch("over-capacity", &json!({})).unwrap();
        assert_eq!(db.condition_watches().unwrap().len(),200);
        assert_eq!(db.research_job(job).unwrap(),before);
        drop(db); std::fs::remove_dir_all(root).unwrap();
    }

}
