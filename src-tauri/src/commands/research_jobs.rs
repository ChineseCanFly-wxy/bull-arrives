//! Small, trusted research jobs. No AI generated code is executed by this coordinator.
use super::research::{self, ModelRunnerConfig};
use crate::db::Database;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Read, path::Path, sync::Arc};
use tauri::State;

// These frozen models consume completed daily bars; intraday signal models need their own admission and scheduler.
pub const DAILY_MODEL_IDS: [&str; 5] = [
    "breadth22_h20",
    "index26_h20",
    "breadth22_excess_csi20",
    "breadth22_rank20",
    "breadth22_open_downside20",
];
pub const MODEL_IDS: [&str; 5] = DAILY_MODEL_IDS;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub kind: String,
    pub models: Vec<String>,
    #[serde(default = "default_comparisons")]
    pub comparisons: Vec<String>,
    #[serde(default)]
    pub continuation: Option<i64>,
    #[serde(default)]
    pub enable_observation: bool,
}
fn default_comparisons() -> Vec<String> {
    vec!["baseline".into()]
}
impl JobRequest {
    fn steps(&self) -> Result<Vec<(String, String, i64)>, String> {
        if self.kind=="explore" {
            if self.models!=vec![super::research_search::SEARCH_ID.to_string()] || self.comparisons!=default_comparisons() || self.continuation.is_some() || self.enable_observation {return Err("嵌套探索仅支持受信固定技术候选集，不能接管原模型账户".into());}
            return Ok(super::research_search::YEARS.iter().map(|year|(super::research_search::SEARCH_ID.to_string(),"nested".into(),*year)).collect());
        }
        if !matches!(
            self.kind.as_str(),
            "scan" | "replay" | "forward" | "compare"
        ) || self.models.is_empty()
            || self.models.len() > 5
            || self.comparisons.is_empty()
            || self.comparisons.len() > 5
        {
            return Err("请选择研究类型和一至五个冻结模型".into());
        }
        if self.models.iter().any(|m| !MODEL_IDS.contains(&m.as_str()))
            || self.models.iter().collect::<HashSet<_>>().len() != self.models.len()
            || self.comparisons.iter().collect::<HashSet<_>>().len() != self.comparisons.len()
        {
            return Err("模型身份未知或重复；探索形态不混入冻结模型".into());
        }
        if self.kind == "scan"
            && (self.comparisons != default_comparisons() || self.continuation.is_some())
        {
            return Err("选股只计算冻结基准信号，不创建模拟账户".into());
        }
        if self.kind == "compare"
            && self
                .comparisons
                .iter()
                .any(|s| !matches!(s.as_str(), "baseline" | "holding15" | "cost_double"))
        {
            return Err("批量比较限定基准、15日、双费；不进行任意参数搜索".into());
        }
        if self.continuation.is_some()
            && (self.kind != "forward" || self.models.len() != 1 || self.comparisons.len() != 1)
        {
            return Err("延续只允许原模型、期限和单个纸上账户".into());
        }
        if self.enable_observation
            && (self.kind != "forward"
                || self.continuation.is_some()
                || self.comparisons != default_comparisons())
        {
            return Err("自动观察只用于新建冻结基准纸上账户".into());
        }
        let mut result = Vec::new();
        for model in &self.models {
            for comparison in &self.comparisons {
                let hold = if comparison == "holding15" { 15 } else { 20 };
                crate::db::model_research::validate_task(
                    &json!({"model_id":model,"holding_days":hold,"comparison":comparison}),
                )?;
                result.push((model.clone(), comparison.clone(), hold));
            }
        }
        if result.len() > 20 {
            return Err("单作业最多20个受控配置".into());
        }
        Ok(result)
    }
}
fn file_hash(path: &Path) -> Result<String, String> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|_| format!("研究文件缺失：{}", path.display()))?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("研究输入必须是实际普通文件".into());
    }
    let mut input = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(hex::encode(digest.finalize()))
}
fn input_identity(config: &ModelRunnerConfig) -> Result<(String, Value), String> {
    research::trusted_runner(config)?;
    let root = Path::new(&config.research_root);
    let mut source = serde_json::Map::new();
    for (name, path) in [
        (
            "runner",
            root.join("research/research-center-runner/model_runner.py"),
        ),
        (
            "registry",
            root.join("research/research-center-runner/registry.json"),
        ),
        (
            "refresh",
            root.join("research/research-center-runner/refresh_market.py"),
        ),
        ("snapshot", Path::new(&config.snapshot).join("matrices.npz")),
        (
            "metadata",
            Path::new(&config.snapshot).join("matrices-metadata.json"),
        ),
        ("index", Path::new(&config.index).to_path_buf()),
    ] {
        source.insert(name.into(), json!({"path":path,"sha256":file_hash(&path)?}));
    }
    let source = Value::Object(source);
    let fingerprint = hex::encode(Sha256::digest(source.to_string().as_bytes()));
    Ok((fingerprint, source))
}
fn ensure_not_cancelled(db: &Database, id: i64) -> Result<(), String> {
    if db.research_job_cancelled(id) {
        Err("研究作业已取消；已完成步骤保留".into())
    } else {
        Ok(())
    }
}
fn group_from_run(row: &Value, job_id: i64, fingerprint: &str) -> Result<Value, String> {
    let model = row["model_id"].as_str().ok_or("选股结果缺模型")?;
    if !MODEL_IDS.contains(&model)
        || row["production_admission"] != false
        || row["training_refitted"] != false
    {
        return Err("选股必须使用未改训的冻结模型".into());
    }
    let scores = row["current_scores"].as_array().ok_or("选股缺全池评分")?;
    let watch = row["signal_watch"].as_array().ok_or("选股缺命中集合")?;
    let threshold = row["signal_threshold"].as_f64().ok_or("选股缺阈值")?;
    if !threshold.is_finite() {
        return Err("模型阈值无效".into());
    }
    let mut names = HashSet::new();
    for candidate in watch {
        let symbol = candidate["symbol"].as_str().ok_or("候选缺股票身份")?;
        let ordinary = symbol.len() == 8
            && (symbol.starts_with("sh6")
                || symbol.starts_with("sz0")
                || symbol.starts_with("sz3"))
            && symbol[2..].bytes().all(|b| b.is_ascii_digit());
        if !ordinary
            || candidate["as_of"] != row["as_of"]
            || candidate["signal_eligible"] != true
            || candidate["score"]
                .as_f64()
                .is_none_or(|v| !v.is_finite() || v <= threshold)
            || !names.insert(symbol)
        {
            return Err("候选日期、沪深身份、阈值或信号条件不符".into());
        }
    }
    Ok(
        json!({"model_id":model,"name":row["model_name"],"holding_days":row["holding_days"],"score_semantic":row["score_semantic"],"threshold":threshold,"as_of":row["as_of"],"scored_stocks":scores.len(),"hit_count":watch.len(),"candidates":watch,"current_scores":scores,"model_sha256":row["model_sha256"],"score_cache_sha256":row["score_cache_sha256"],"runner_sha256":row["runner_sha256"],"data_sha256":row["data_sha256"],"source_fingerprint":fingerprint,"job_id":job_id,"production_admission":false}),
    )
}
fn local_cutoff_ready(config: &ModelRunnerConfig, day: &str) -> bool {
    let requested = day.replace('-', "").parse::<i64>().unwrap_or(-1);
    let Ok(text) =
        std::fs::read_to_string(Path::new(&config.snapshot).join("matrices-metadata.json"))
    else {
        return false;
    };
    let Ok(meta) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    if meta["quality"]["last_date"].as_i64() != Some(requested) {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&config.index) else {
        return false;
    };
    let Some(line) = text.lines().rev().find(|line| !line.trim().is_empty()) else {
        return false;
    };
    let Ok(index) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    index["date"].as_i64() == Some(requested)
        || index["date"]
            .as_str()
            .is_some_and(|date| date.replace('-', "") == requested.to_string())
}
fn run_worker(db: &Database, id: i64) -> Result<(), String> {
    if db.research_job(id)?["kind"]=="explore" {return super::research_search::run_worker(db,id); }
    let job = db.research_job(id)?;
    let request: JobRequest =
        serde_json::from_value(job["request"].clone()).map_err(|_| "研究请求损坏")?;
    let steps = request.steps()?;
    ensure_not_cancelled(db, id)?;
    db.research_job_phase(
        id,
        "checking",
        "检查受信程序与已完成行情；不会重新训练或执行AI代码",
    )?;
    let mut config = if job["context"].is_null() {
        research::model_config(db)?
    } else {
        serde_json::from_value(job["context"]["config"].clone()).map_err(|_| "作业环境断点损坏")?
    };
    let day = research::model_completed_day(chrono::Utc::now())?.to_string();
    let previous = if let Some(run) = request.continuation {
        let previous = db.model_run(run)?;
        if previous["model_id"] != request.models[0]
            || previous["comparison"] != request.comparisons[0]
            || previous["mode"] != "forward"
        {
            return Err("延续请求不匹配原账户".into());
        }
        Some(previous)
    } else {
        None
    };
    let mut data_status = json!({"mode":"frozen_input","message":"使用固定输入文件"});
    if job["context"].is_null() && matches!(request.kind.as_str(), "scan" | "forward") {
        let endpoint = db
            .get_setting("local_history_url")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "http://127.0.0.1:7899".into());
        let refreshed = {
            db.research_job_phase(
                id,
                "updating",
                "优先核对StockDB；近期缺口从新浪/腾讯收盘行情补齐，日期和覆盖合格才采用",
            )?;
            research::refresh_model_config_controlled(
                config.clone(),
                endpoint,
                day.clone(),
                true,
                Some((db, id)),
            )
        };
        match refreshed {
            Ok(updated) => {
                config = updated;
                data_status=research::market_data_status(db,&config);
                data_status["message"]=json!("StockDB优先，近期收盘备用源通过日期、覆盖和历史衔接检查；原模型和历史前缀保留");
                db.set_setting("model_market_data_status",&data_status.to_string()).map_err(|e|e.to_string())?;
            }
            Err(error) => {
                ensure_not_cancelled(db, id)?;
                if !local_cutoff_ready(&config, &day) {
                    return Err(format!(
                        "行情未到最近完成日{day}，不能用旧快照替代。{error}"
                    ));
                }
                data_status = json!({"mode":"saved_snapshot","message":"服务未核对，使用截止日期齐备的已保存快照；仍需原始程序核对实际矩阵日期","refresh_error":error});
                db.research_job_phase(
                    id,
                    "checking",
                    "在线核对不可用；本地快照和指数已到最近完成日，按已保存快照计算",
                )?;
            }
        }
    }
    ensure_not_cancelled(db, id)?;
    let (fingerprint, sources) = input_identity(&config)?;
    let context = if job["context"].is_null() {
        json!({"config":config,"sources":sources,"requested_day":day,"data_status":data_status,"continuation_sha256":previous.as_ref().map(|v|v["content_sha256"].clone())})
    } else {
        job["context"].clone()
    };
    db.prepare_research_job(id, &context, &fingerprint)?;
    let completed = job["results"].as_array().ok_or("作业断点列表损坏")?;
    for (index, (model, comparison, hold)) in steps.iter().enumerate() {
        ensure_not_cancelled(db, id)?;
        let key = format!("{model}:{comparison}:{hold}");
        if let Some(saved) = completed.iter().find(|v| v["key"].as_str() == Some(&key)) {
            if saved["input_identity"] != fingerprint {
                return Err("已完成步骤身份与当前作业不同".into());
            }
            continue;
        }
        if let Some(old) = &previous {
            if context["continuation_sha256"] != old["content_sha256"] {
                return Err("原账户已在其他操作中更新，请新建延续作业".into());
            }
        }
        db.research_job_phase(
            id,
            "calculating",
            &format!(
                "第{}/{}项：{} / {}；实际计算{}",
                index + 1,
                steps.len(),
                model,
                comparison,
                if request.kind == "scan" {
                    "最近完成日模型信号"
                } else {
                    "原始价格现金与交易账本"
                }
            ),
        )?;
        let mode = if matches!(request.kind.as_str(), "scan" | "forward") {
            "forward"
        } else {
            "replay"
        };
        let start = previous.as_ref().and_then(|v| v["forward_start"].as_i64());
        let raw = research::execute_model_controlled(
            config.clone(),
            model.clone(),
            *hold,
            comparison.clone(),
            mode.into(),
            start,
            Some((db, id)),
        )?;
        ensure_not_cancelled(db, id)?;
        db.research_job_phase(
            id,
            "verifying",
            "核对输入身份、完整信号、现金守恒与真实账本",
        )?;
        let content = crate::db::model_research::decode_run(&raw)?;
        if content["model_id"] != *model
            || content["holding_days"] != *hold
            || content["comparison"] != *comparison
            || content["mode"] != mode
        {
            return Err("程序输出配置身份与作业不同".into());
        }
        if input_identity(&config)?.0 != fingerprint {
            return Err("计算期间行情或程序改变，本步骤不导入；请新建任务".into());
        }
        if matches!(request.kind.as_str(), "scan" | "forward")
            && content["as_of"] != context["requested_day"]
        {
            return Err("模型结果没有达到该作业指定完成日，不能展示为当前候选".into());
        }
        let mut saved = json!({"key":key,"model_id":model,"comparison":comparison,"holding_days":hold,"input_identity":fingerprint});
        if request.kind == "scan" {
            saved["group"] = group_from_run(&content, id, &fingerprint)?;
            saved["group"]["data_status"] = context["data_status"].clone();
            db.checkpoint_research_job(id, &saved)?;
        } else {
            saved["metrics"] = content["ledger"]["metrics"].clone();
            saved["state"] = content["state"].clone();
            saved["as_of"] = content["as_of"].clone();
            db.save_job_model_step(
                id,
                &raw,
                request.continuation,
                context["continuation_sha256"].as_str(),
                request.enable_observation,
                &saved,
            )?;
        }
    }
    ensure_not_cancelled(db, id)?;
    db.set_setting(
        "model_runner_config",
        &serde_json::to_string(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    db.set_setting("model_research_last_error", "")
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn dispatch(db: Arc<Database>, id: i64, permit: tokio::sync::SemaphorePermit<'static>) {
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let work = db.clone();
        let result = tokio::task::spawn_blocking(move || run_worker(&work, id)).await;
        match result {
            Ok(Ok(())) => {
                let (state, message) = if db.research_job_cancelled(id) {
                    ("cancelled", "作业已取消；已完成步骤与证据保留")
                } else {
                    (
                        "complete",
                        "实际计算与证据核验完成；结果保留，模型仍为研究观察",
                    )
                };
                let _ = db.finish_research_job(id, state, message);
            }
            Ok(Err(error)) => {
                let state = if db.research_job_cancelled(id) {
                    "cancelled"
                } else {
                    "failed"
                };
                let _ = db.finish_research_job(id, state, &error);
                let _ = db.set_setting("model_research_last_error", &error);
            }
            Err(error) => {
                let _ = db.finish_research_job(
                    id,
                    "failed",
                    &format!("研究进程异常：{error}；已完成步骤保留"),
                );
            }
        }
    });
}
#[tauri::command]
pub fn research_job_start(
    db: State<'_, Arc<Database>>,
    request: JobRequest,
) -> Result<Value, String> {
    let steps = request.steps()?;
    let permit = research::research_gate()
        .try_acquire()
        .map_err(|_| "另一项研究正在运行，可在研究中心查看进度")?;
    let id = db.create_research_job(
        &request.kind,
        &serde_json::to_value(&request).map_err(|e| e.to_string())?,
        steps.len(),
    )?;
    db.activate_research_job(id)?;
    let response = db.research_job(id)?;
    dispatch(db.inner().clone(), id, permit);
    Ok(response)
}
#[tauri::command]
pub fn research_job_resume(db: State<'_, Arc<Database>>, id: i64) -> Result<Value, String> {
    let job = db.research_job(id)?;
    let request: JobRequest =
        serde_json::from_value(job["request"].clone()).map_err(|_| "研究请求损坏")?;
    request.steps()?;
    let permit = research::research_gate()
        .try_acquire()
        .map_err(|_| "另一项研究正在运行")?;
    db.activate_research_job(id)?;
    let response = db.research_job(id)?;
    dispatch(db.inner().clone(), id, permit);
    Ok(response)
}
#[tauri::command]
pub fn research_job_cancel(db: State<'_, Arc<Database>>, id: i64) -> Result<(), String> {
    db.cancel_research_job(id)
}
#[tauri::command]
pub fn research_job_list(db: State<'_, Arc<Database>>) -> Result<Vec<Value>, String> {
    db.research_jobs()
}
#[tauri::command]
pub fn research_job_get(db: State<'_, Arc<Database>>, id: i64) -> Result<Value, String> {
    db.research_job(id)
}

pub fn candidate_view(db: &Database) -> Result<Value, String> {
    let day = super::mainline::completed_day(chrono::Utc::now())?.to_string();
    let job = db.latest_model_scan_steps()?;
    let Some(job) = job else {
        return Ok(
            json!({"schema":"research-model-candidates-v1","current_as_of":day,"as_of":null,"fresh":false,"groups":[],"job_id":null,"production_admission":false,"message":"选择模型并点击筛选，会更新真实行情后计算；进入页面不会自动执行"}),
        );
    };
    let groups = job["results"]
        .as_array()
        .ok_or("选股步骤损坏")?
        .iter()
        .map(|s| {
            let mut group = s["group"].clone();
            if let Some(fields) = group.as_object_mut() {
                fields.remove("current_scores");
            }
            group
        })
        .collect::<Vec<_>>();
    let as_of = groups
        .first()
        .and_then(|v| v["as_of"].as_str())
        .ok_or("选股日期缺失")?;
    if groups
        .iter()
        .any(|g| g["as_of"] != as_of || g["production_admission"] != false)
    {
        return Err("选股各模型日期或状态不一致".into());
    }
    let fresh = as_of == day;
    let unique = groups
        .iter()
        .flat_map(|g| g["candidates"].as_array().into_iter().flatten())
        .filter_map(|v| v["symbol"].as_str())
        .collect::<HashSet<_>>()
        .len();
    Ok(
        json!({"schema":"research-model-candidates-v1","current_as_of":day,"as_of":as_of,"fresh":fresh,"groups":if fresh{groups}else{vec![]},"historical_hit_count":unique,"data_status":job["context"]["data_status"],"job_id":job["id"],"source_fingerprint":job["fingerprint"],"production_admission":false,"message":if fresh{"满足冻结模型条件的研究候选；分数按各模型独立排序，不是胜率或下一开盘可买入许可"}else{"上次选股日期已经过期；历史结果保留，请点击筛选更新，不把旧名单当今日候选"}}),
    )
}
#[tauri::command]
pub fn get_model_candidates(db: State<'_, Arc<Database>>) -> Result<Value, String> {
    candidate_view(&db)
}

/// Names are auxiliary quote labels, never a replacement for frozen date/score evidence.
#[tauri::command]
pub async fn get_model_candidate_labels(
    symbols: Vec<String>,
    manager: State<'_, Arc<crate::datasource::DataSourceManager>>,
) -> Result<Value, String> {
    if symbols.len() > 100
        || symbols.iter().any(|s| {
            s.len() != 8
                || !(s.starts_with("sh6") || s.starts_with("sz0") || s.starts_with("sz3"))
                || !s[2..].bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err("每页最多100只沪深股票名称查询".into());
    }
    let mut labels = serde_json::Map::new();
    let mut sources = manager.all_sources();
    let active = manager.active_name();
    sources.sort_by_key(|(id, _)| *id != active);
    for (source_id, source) in sources {
        let missing = symbols
            .iter()
            .filter(|s| !labels.contains_key(*s))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            break;
        }
        if let Ok(Ok(rows)) = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            source.fetch_realtime(&missing, "CN"),
        )
        .await
        {
            for quote in rows {
                if missing.contains(&quote.code)
                    && quote.market == "CN"
                    && !quote.name.trim().is_empty()
                {
                    labels.insert(quote.code.clone(),json!({"name":quote.name,"excluded":crate::market_rules::is_st(&quote.name)||crate::market_rules::is_delisting(&quote.name)||crate::market_rules::is_new_listing(&quote.name),"source":source_id,"quote_time":quote.timestamp}));
                }
            }
        }
    }
    Ok(Value::Object(labels))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "explicit live StockDB read-only job smoke; writes only isolated research output"]
    fn live_frozen_scan_job_roundtrip() {
        assert_eq!(std::env::var("BULL_MODEL_JOB_SMOKE").as_deref(), Ok("1"));
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let out = project.join("research/tsp-integration-2026-10-03");
        std::fs::create_dir_all(&out).unwrap();
        let db_root = out.join(format!("runtime-db-{}", uuid::Uuid::new_v4()));
        let db = Database::open(db_root.clone()).unwrap();
        db.set_setting("local_history_enabled", "1").unwrap();
        db.set_setting("local_history_url", "http://127.0.0.1:7899")
            .unwrap();
        let config = research::model_config(&db).unwrap();
        let original = file_hash(&Path::new(&config.snapshot).join("matrices.npz")).unwrap();
        let request = JobRequest {
            kind: "scan".into(),
            models: MODEL_IDS.iter().map(|s| s.to_string()).collect(),
            comparisons: default_comparisons(),
            continuation: None,
            enable_observation: false,
        };
        let id = db
            .create_research_job(
                "scan",
                &serde_json::to_value(&request).unwrap(),
                request.steps().unwrap().len(),
            )
            .unwrap();
        db.activate_research_job(id).unwrap();
        run_worker(&db, id).unwrap();
        assert_eq!(db.research_job(id).unwrap()["completed"], 5);
        db.recover_research_jobs().unwrap();
        db.activate_research_job(id).unwrap();
        run_worker(&db, id).unwrap();
        db.finish_research_job(id, "complete", "live verified and recovered")
            .unwrap();
        let view = candidate_view(&db).unwrap();
        assert_eq!(view["fresh"], true);
        assert_eq!(view["groups"].as_array().unwrap().len(), 5);
        assert_eq!(db.model_runs().unwrap(), json!([]));
        let groups = view["groups"].as_array().unwrap();
        let report = json!({"schema":"model-job-live-verification-v1","tested_at":chrono::Utc::now(),"as_of":view["as_of"],"data_status":view["data_status"],"job_id":id,"fingerprint":view["source_fingerprint"],"models":groups.iter().map(|g|json!({"model_id":g["model_id"],"scored_stocks":g["scored_stocks"],"hit_count":g["hit_count"],"threshold":g["threshold"],"model_sha256":g["model_sha256"],"top5":g["candidates"].as_array().unwrap().iter().take(5).collect::<Vec<_>>() })).collect::<Vec<_>>(),"scan_created_accounts":0,"original_snapshot_unchanged":file_hash(&Path::new(&config.snapshot).join("matrices.npz")).unwrap()==original,"database":db_root});
        assert_eq!(report["original_snapshot_unchanged"], true);
        std::fs::write(
            out.join("live-scan-verification.json"),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("{}", serde_json::to_string(&report).unwrap());
    }
    #[test]
    #[ignore = "explicit multi-year three-account compare using unchanged local frozen artifacts"]
    fn real_finite_compare_saves_accounts_and_checkpoints_atomically() {
        assert_eq!(std::env::var("BULL_MODEL_JOB_SMOKE").as_deref(), Ok("1"));
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let out = project.join("research/tsp-integration-2026-10-03");
        std::fs::create_dir_all(&out).unwrap();
        let db_root = out.join(format!("compare-db-{}", uuid::Uuid::new_v4()));
        let db = Database::open(db_root.clone()).unwrap();
        let request = JobRequest {
            kind: "compare".into(),
            models: vec!["breadth22_h20".into()],
            comparisons: vec!["baseline".into(), "holding15".into(), "cost_double".into()],
            continuation: None,
            enable_observation: false,
        };
        let id = db
            .create_research_job("compare", &serde_json::to_value(&request).unwrap(), 3)
            .unwrap();
        db.activate_research_job(id).unwrap();
        run_worker(&db, id).unwrap();
        db.recover_research_jobs().unwrap();
        db.activate_research_job(id).unwrap();
        run_worker(&db, id).unwrap();
        db.finish_research_job(id, "complete", "compare verified")
            .unwrap();
        let job = db.research_job(id).unwrap();
        let runs = db.model_runs().unwrap();
        assert_eq!(job["completed"], 3);
        assert_eq!(runs.as_array().unwrap().len(), 3);
        for step in job["results"].as_array().unwrap() {
            let run_id = step["run_id"].as_i64().unwrap();
            assert_eq!(
                db.model_run(run_id).unwrap()["ledger"]["metrics"],
                step["metrics"]
            );
        }
        // A cancelled transaction must not register or overwrite a real ledger, even with valid output.
        let cancelled = db.create_research_job("replay", &json!({}), 1).unwrap();
        db.activate_research_job(cancelled).unwrap();
        db.cancel_research_job(cancelled).unwrap();
        let first = &job["results"][0];
        let raw = db
            .model_run_bundle(first["run_id"].as_i64().unwrap())
            .unwrap();
        assert!(db
            .save_job_model_step(cancelled, &raw, None, None, false, first)
            .is_err());
        assert_eq!(db.model_runs().unwrap().as_array().unwrap().len(), 3);
        assert_eq!(db.research_job(cancelled).unwrap()["completed"], 0);
        let report = json!({"schema":"finite-compare-live-verification-v1","tested_at":chrono::Utc::now(),"database":db_root,"job_id":id,"fingerprint":job["fingerprint"],"completed":3,"resume_skipped_verified_steps":true,"cancelled_account_save_rejected":true,"results":job["results"]});
        std::fs::write(
            out.join("finite-compare-verification.json"),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("{}", report);
    }
    #[tokio::test]
    #[ignore = "explicit opt-in paper/watch smoke in the isolated real-scan database only"]
    async fn real_scan_to_paper_observation_does_not_backfill_trades() {
        assert_eq!(std::env::var("BULL_MODEL_JOB_SMOKE").as_deref(), Ok("1"));
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let out = project.join("research/tsp-integration-2026-10-03");
        let report: Value = serde_json::from_str(
            &std::fs::read_to_string(out.join("live-scan-verification.json")).unwrap(),
        )
        .unwrap();
        let db_root = std::path::PathBuf::from(report["database"].as_str().unwrap());
        assert!(db_root
            .canonicalize()
            .unwrap()
            .starts_with(out.canonicalize().unwrap()));
        let db = Database::open(db_root).unwrap();
        let day = super::super::mainline::completed_day(chrono::Utc::now())
            .unwrap()
            .to_string();
        assert_eq!(report["as_of"], day);
        let symbol = report["models"][0]["top5"][0]["symbol"].as_str().unwrap();
        let watch = crate::model_conditions::WatchRequest {
            job_id: report["job_id"].as_i64().unwrap(),
            model_id: "breadth22_h20".into(),
            symbol: symbol.into(),
            preset: "model_confirm".into(),
            condition_tree: None,
        };
        let watch_id = crate::model_conditions::register_watch(&db, &watch).unwrap();
        let request = JobRequest {
            kind: "forward".into(),
            models: vec![watch.model_id.clone()],
            comparisons: default_comparisons(),
            continuation: None,
            enable_observation: true,
        };
        let id = db
            .create_research_job("forward", &serde_json::to_value(&request).unwrap(), 1)
            .unwrap();
        db.activate_research_job(id).unwrap();
        run_worker(&db, id).unwrap();
        db.finish_research_job(id, "complete", "paper observation verified")
            .unwrap();
        let run_id = db.research_job(id).unwrap()["results"][0]["run_id"]
            .as_i64()
            .unwrap();
        let run = db.model_run(run_id).unwrap();
        assert_eq!(run["mode"], "forward");
        assert_eq!(run["enabled"], true);
        assert_eq!(run["ledger"]["metrics"]["completed_holding_cycles"], 0);
        assert_eq!(run["ledger"]["orders"], json!([]));
        assert_eq!(run["ledger"]["curve"][0]["cash"], 100000.);
        assert_eq!(run["signal_watch"].as_array().unwrap().len(), 7);
        let watches = db.condition_watches().unwrap();
        let config = &watches.iter().find(|w| w["id"] == watch_id).unwrap()["config"];
        assert!(crate::model_conditions::latest_group(&db, config, &day)
            .unwrap()
            .is_some());
        let context =
            super::super::research_evidence::stock_research_context(&db, symbol, &day).unwrap();
        assert_eq!(
            context["latest_model_scan"]["models"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert_eq!(
            context["latest_model_scan"]["models"][0]["signal_status"],
            "positive_record"
        );
        let notices = crate::model_conditions::tick(
            &db,
            &crate::datasource::DataSourceManager::new(),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        assert!(notices.is_empty());
        let state = db
            .condition_watches()
            .unwrap()
            .into_iter()
            .find(|w| w["id"] == watch_id)
            .unwrap();
        assert_eq!(state["observation"]["state"], "waiting_data");
        let saved = json!({"schema":"scan-to-paper-watch-verification-v1","tested_at":chrono::Utc::now(),"as_of":day,"watch_id":watch_id,"run_id":run_id,"enabled":true,"new_account_cash":100000,"backfilled_orders":0,"completed_cycles":0,"daily_model_hits":7,"same_day_ai_models":5,"weekend_condition_state":state["observation"]["state"],"weekend_notifications":0});
        std::fs::write(
            out.join("scan-to-observation-verification.json"),
            serde_json::to_string_pretty(&saved).unwrap(),
        )
        .unwrap();
        println!("{}", saved);
    }
    #[test]
    fn finite_requests_reject_unknown_duplicate_and_mixed_continuation() {
        let mut r = JobRequest {
            kind: "scan".into(),
            models: MODEL_IDS.iter().map(|s| s.to_string()).collect(),
            comparisons: default_comparisons(),
            continuation: None,
            enable_observation: false,
        };
        assert_eq!(r.steps().unwrap().len(), 5);
        r.models.push(r.models[0].clone());
        assert!(r.steps().is_err());
        r.models = vec!["panic_second_test".into()];
        assert!(r.steps().is_err());
        r.models = vec![MODEL_IDS[0].into()];
        r.continuation = Some(1);
        assert!(r.steps().is_err());
        r.continuation = None;
        r.kind = "compare".into();
        r.comparisons = vec!["baseline".into(), "holding15".into(), "cost_double".into()];
        assert_eq!(r.steps().unwrap()[1].2, 15);
    }
    #[test]
    fn screen_requires_actual_positive_score_identity_and_ordinary_stock() {
        let mut v = json!({"model_id":"breadth22_h20","production_admission":false,"training_refitted":false,"as_of":"2026-09-30","current_scores":[{}],"signal_threshold":0.,"signal_watch":[{"symbol":"sz000001","as_of":"2026-09-30","score":0.02,"signal_eligible":true}]});
        assert_eq!(
            group_from_run(&v, 1, "fingerprint").unwrap()["hit_count"],
            1
        );
        v["signal_watch"][0]["score"] = json!(0.);
        assert!(group_from_run(&v, 1, "f").is_err());
        v["signal_watch"][0]["score"] = json!(0.02);
        v["signal_watch"][0]["symbol"] = json!("bj920001");
        assert!(group_from_run(&v, 1, "f").is_err());
        v["signal_watch"] = json!([]);
        v["training_refitted"] = json!(true);
        assert!(group_from_run(&v, 1, "f").is_err());
    }
    #[test]
    fn empty_candidate_view_does_not_start_python_or_create_account() {
        let db = Database::open(
            std::env::temp_dir().join(format!("candidate-empty-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let view = candidate_view(&db).unwrap();
        assert_eq!(view["groups"], json!([]));
        assert!(db.research_jobs().unwrap().is_empty());
        assert_eq!(db.model_runs().unwrap(), json!([]));
    }
}
