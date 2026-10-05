//! Explicit opt-in compound observations for real frozen model hits.
use crate::{
    datasource::{
        self,
        DataSourceManager,
    },
    db::Database,
};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};
use tauri::State;
const LIMIT: usize = 20;
fn cst() -> FixedOffset {
    FixedOffset::east_opt(28800).unwrap()
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchRequest {
    pub job_id: i64,
    pub model_id: String,
    pub symbol: String,
    pub preset: String,
    #[serde(default)]
    pub condition_tree: Option<crate::condition_logic::ConditionTree>,
}
fn presets() -> Value {
    json!([
        {"id":"model_hit","name":"模型新命中 / 失效","description":"下一次完成日重新筛选后，模型条件新成立或失效时提醒"}
    ])
}
const RETIRED_MINUTE: &str = "分钟走势确认已停用；请从筛选结果重新添加模型命中或可用自定义提醒，旧记录仍保留";
fn active_condition(config:&Value)->Result<Option<crate::condition_logic::ConditionTree>,String>{
    match config["preset"].as_str(){
        Some("model_hit")=>Ok(None),
        Some("custom")=>{let node:crate::condition_logic::ConditionTree=serde_json::from_value(config["condition_tree"].clone()).map_err(|_|"自定义条件已损坏")?;node.validate()?;if node.needs_intraday(){Err(RETIRED_MINUTE.into())}else{Ok(Some(node))}},
        _=>Err(RETIRED_MINUTE.into()),
    }
}
fn scan_hit(db: &Database, request: &WatchRequest) -> Result<(Value, Value), String> {
    if !crate::commands::research_jobs::MODEL_IDS.contains(&request.model_id.as_str()) {return Err("请选择冻结模型和常用观察条件".into());}
    active_condition(&json!({"preset":request.preset,"condition_tree":request.condition_tree}))?;
    if request.preset!="custom" && request.condition_tree.is_some(){return Err("预设条件不接受额外自定义表达式".into());}
    let job = db.research_job(request.job_id)?;
    let day = crate::commands::mainline::completed_day(Utc::now())?.to_string();
    if job["kind"] != "scan" || job["state"] != "complete" {
        return Err("先完成模型筛选，再选择真实命中的股票".into());
    }
    let group = job["results"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["model_id"] == request.model_id)
        .map(|s| s["group"].clone())
        .ok_or("该作业未计算所选模型")?;
    if group["as_of"] != day || group["production_admission"] != false {
        return Err("候选日期已过期或身份不同，请重新筛选".into());
    }
    let hit = group["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["symbol"] == request.symbol && c["signal_eligible"] == true)
        .cloned()
        .ok_or("股票不在该模型真实命中集合")?;
    Ok((group, hit))
}
#[tauri::command]
pub fn model_condition_watch(
    db: State<'_, Arc<Database>>,
    request: WatchRequest,
) -> Result<i64, String> {
    register_watch(&db, &request)
}
pub(crate) fn register_watch(db: &Database, request: &WatchRequest) -> Result<i64, String> {
    let (group, hit) = scan_hit(db, request)?;
    let config = json!({"schema":"frozen-model-condition-v1","symbol":request.symbol,"model_id":request.model_id,"model_name":group["name"],"preset":request.preset,"condition_tree":request.condition_tree,"model_sha256":group["model_sha256"],"runner_sha256":group["runner_sha256"],"score_cache_sha256":group["score_cache_sha256"],"initial_hit":hit,"source_job_id":request.job_id,"source_fingerprint":group["source_fingerprint"],"production_admission":false});
    let mut identity_fields=json!({"symbol":config["symbol"],"model":config["model_id"],"preset":config["preset"],"model_sha":config["model_sha256"],"runner_sha":config["runner_sha256"]});
    if request.preset=="custom" {identity_fields["condition_tree"]=config["condition_tree"].clone();}
    let identity=hex::encode(Sha256::digest(identity_fields.to_string().as_bytes()));
    let id = db.save_condition_watch(&identity, &config)?;
    // A new or explicitly re-added watch acknowledges the current daily hit silently.
    let previous = db
        .condition_watches()?
        .into_iter()
        .find(|v| v["id"] == id)
        .map(|v| v["observation"].clone())
        .unwrap_or_else(|| json!({}));
    if previous.as_object().is_some_and(|s| s.is_empty())
        || request.preset == "model_hit"
        || previous["last_known_model_hit"] == false
    {
        let mut state = previous;
        state["state"] = json!("watching");
        state["last_known_model_hit"] = json!(true);
        state["last_known_combined"] = json!(request.preset == "model_hit");
        state["last_scan_day"] = group["as_of"].clone();
        state["checked_at"] = json!(Utc::now().to_rfc3339());
        state["message"] = json!("已静默登记当前命中；等待新条件确认或失效，不在加入时刷通知");
        db.commit_condition_observation(id, &state, None)?;
    }
    Ok(id)
}
#[tauri::command]
pub fn model_condition_auto_update(
    db: State<'_, Arc<Database>>,
    watch_id: i64,
) -> Result<Value, String> {
    let watch = db
        .condition_watches()?
        .into_iter()
        .find(|w| w["id"] == watch_id && w["enabled"] == true)
        .ok_or("先开启该股票条件观察")?;
    if db.get_setting("local_history_enabled").map_err(|e|e.to_string())?.as_deref()!=Some("1"){return Err("自动更新需在数据源启用StockDB历史；本次股票观察已保存，手动筛选仍可读取齐备快照".into());}
    let config = &watch["config"];
    let model = config["model_id"].as_str().ok_or("观察缺模型身份")?;
    for run in db.enabled_model_runs()? {
        if run["mode"] == "forward"
            && run["model_id"] == model
            && run["comparison"] == "baseline"
            && run["model_sha256"] == config["model_sha256"]
            && run["runner_sha256"] == config["runner_sha256"]
        {
            return Ok(
                json!({"run_id":run["id"],"job_id":null,"message":"已复用原模型日线维护记录；不使用交易账户"}),
            );
        }
    }
    for job in db.research_jobs()? {
        if matches!(job["state"].as_str(), Some("running" | "queued"))
            && job["kind"] == "forward"
            && job["request"]["enable_observation"] == true
            && job["request"]["models"]
                .as_array()
                .is_some_and(|v| v.iter().any(|m| m == model))
        {
            return Ok(
                json!({"run_id":null,"job_id":job["id"],"message":"原模型日线维护正在准备；完成后只用于更新观察证据"}),
            );
        }
    }
    let request = crate::commands::research_jobs::JobRequest {
        kind: "forward".into(),
        models: vec![model.to_owned()],
        comparisons: vec!["baseline".into()],
        continuation: None,
        enable_observation: true,
    };
    let job = crate::commands::research_jobs::research_job_start(db, request)?;
    Ok(
        json!({"run_id":null,"job_id":job["id"],"message":"后台准备原模型日线维护记录，核验后更新观察证据；不创建买卖账户、不下交易委托"}),
    )
}
#[tauri::command]
pub fn model_condition_watches(db: State<'_, Arc<Database>>) -> Result<Value, String> {
    Ok(
        json!({"presets":presets(),"watches":db.condition_watches()?,"limit_per_tick":LIMIT,"status":db.get_setting("model_condition_watch_status").map_err(|e|e.to_string())?.and_then(|s|serde_json::from_str::<Value>(&s).ok())}),
    )
}
#[tauri::command]
pub fn model_condition_enable(
    db: State<'_, Arc<Database>>,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    db.enable_condition_watch(id, enabled)
}
#[tauri::command]
pub fn model_condition_delete(
    db: State<'_, Arc<Database>>,
    id: i64,
) -> Result<(), String> {
    db.delete_condition_watch(id)
}
#[tauri::command]
pub fn get_model_condition_event(
    db: State<'_, Arc<Database>>,
    event_key: String,
) -> Result<Value, String> {
    db.condition_event(&event_key)
}
pub(crate) fn latest_group(
    db: &Database,
    config: &Value,
    day: &str,
) -> Result<Option<Value>, String> {
    // Find the latest completed scan for this model, including a non-hit result (empty candidate list).
    for summary in db
        .research_jobs()?
        .iter()
        .filter(|v| v["kind"] == "scan" && v["state"] == "complete")
    {
        let job = db.research_job(summary["id"].as_i64().unwrap())?;
        if let Some(group) = job["results"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| &s["group"])
            .find(|g| g["model_id"] == config["model_id"] && g["as_of"] == day)
        {
            if group["model_sha256"] != config["model_sha256"]
                || group["runner_sha256"] != config["runner_sha256"]
            {
                return Err("模型程序版本改变，请重新登记观察条件".into());
            }
            return Ok(Some(group.clone()));
        }
    }
    for run in db.enabled_model_runs()? {
        if run["as_of"] == day
            && run["model_id"] == config["model_id"]
            && run["model_sha256"] == config["model_sha256"]
            && run["runner_sha256"] == config["runner_sha256"]
        {
            return Ok(Some(
                json!({"model_id":run["model_id"],"model_sha256":run["model_sha256"],"runner_sha256":run["runner_sha256"],"as_of":day,"candidates":run["signal_watch"],"source_fingerprint":run["content_sha256"]}),
            ));
        }
    }
    Ok(None)
}
fn mainline_for_symbol(
    db: &Database,
    symbol: &str,
    day: &str,
    bound: Option<&str>,
) -> Result<Option<Value>, String> {
    let mut rows = Vec::new();
    for (key, text) in db.get_all_settings().map_err(|e| e.to_string())? {
        if !(key.starts_with("mainline_snapshot_BK") || key.starts_with("mainline_snapshot_SW")) {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if row["as_of"] == day
            && row["complete"] == true
            && crate::commands::mainline::verify_snapshot(&row).is_ok()
        {
            rows.push(row);
        }
    }
    let leader = |row: &Value| {
        row["leaders"]
            .as_array()
            .is_some_and(|leaders| leaders.iter().any(|stock| stock["symbol"] == symbol))
    };
    let selected = bound
        .and_then(|code| rows.iter().find(|row| row["sector_code"] == code))
        .or_else(|| rows.iter().find(|row| row["strong"] == true && leader(row)))
        .or_else(|| rows.iter().find(|row| leader(row)));
    Ok(selected.cloned())
}
/// Missing data cannot erase the last known state or fabricate a new edge.
fn edge(
    previous: &Value,
    model_hit: Option<bool>,
    combined: Option<bool>,
    shape: Option<&str>,
) -> (Option<&'static str>, Value) {
    let mut next = previous.clone();
    let mut event = None;
    if let Some(hit) = model_hit {
        let old = previous["last_known_model_hit"].as_bool();
        if old == Some(true) && !hit {
            event = Some("model_invalidated");
        }
        next["last_known_model_hit"] = json!(hit);
    }
    if let Some(value) = combined {
        let old = previous["last_known_combined"].as_bool();
        if value && old == Some(false) {
            event = Some("conditions_confirmed");
        }
        if !value && old == Some(true) && event.is_none() {
            event = Some("conditions_invalidated");
        }
        next["last_known_combined"] = json!(value);
    }
    if matches!(shape, Some("invalidated" | "weakened"))
        && previous["last_shape"].as_str() != shape
        && previous["last_shape"].as_str() == Some("confirmed")
    {
        event = Some("shape_invalidated");
    }
    if let Some(shape) = shape {
        next["last_shape"] = json!(shape);
    }
    (event, next)
}
pub async fn tick(
    db: &Database,
    manager: &DataSourceManager,
    now: DateTime<Utc>,
) -> Result<Vec<Value>, String> {
    let mut watches = db
        .condition_watches()?
        .into_iter()
        .filter(|w| w["enabled"] == true)
        .collect::<Vec<_>>();
    if watches.is_empty() {
        return Ok(vec![]);
    }
    let all = watches.len();
    let start = db
        .get_setting("model_condition_cursor")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        % all;
    watches.rotate_left(start);
    watches.truncate(LIMIT);
    db.set_setting(
        "model_condition_cursor",
        &((start + watches.len()) % all).to_string(),
    )
    .map_err(|e| e.to_string())?;
    let day = crate::commands::mainline::completed_day(now)?.to_string();
    let continuous = datasource::a_share_calendar::continuous(now).is_ok();
    let symbols = watches
        .iter()
        .filter(|w| active_condition(&w["config"]).ok().flatten().is_some())
        .filter_map(|w| w["config"]["symbol"].as_str().map(str::to_owned))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut quotes = BTreeMap::new();
    if continuous && !symbols.is_empty() {
        let mut sources = manager.all_sources();
        let active = manager.active_name();
        sources.sort_by_key(|(id, _)| *id != active);
        for (name, source) in sources {
            let missing = symbols
                .iter()
                .filter(|s| !quotes.contains_key(*s))
                .cloned()
                .collect::<Vec<_>>();
            if missing.is_empty() {
                break;
            }
            if let Ok(rows) = source.fetch_realtime(&missing, "CN").await {
                for q in rows {
                    if missing.contains(&q.code)
                        && q.market == "CN"
                        && crate::alerts::is_fresh_trading_quote(&q, now.with_timezone(&cst()))
                    {
                        quotes.insert(q.code.clone(), (q, name.to_owned()));
                    }
                }
            }
        }
    }
    let mut notices = BTreeMap::<String, Value>::new();
    for watch in &watches {
        let id = watch["id"].as_i64().ok_or("观察身份缺失")?;
        let config = &watch["config"];
        let symbol = config["symbol"].as_str().ok_or("观察代码缺失")?;
        let observed=async {
            let custom=active_condition(config)?;
            let group=latest_group(db,config,&day)?.ok_or("缺最近完成日该模型计算；点筛选更新，或在研究中心开启原模型自动观察")?;
            let candidate=group["candidates"].as_array().ok_or("模型候选集合损坏")?.iter().find(|c|c["symbol"]==symbol).cloned();
            let mut group=group; if let Some(fields)=group.as_object_mut(){fields.remove("candidates");fields.remove("current_scores");}
            if candidate.is_none(){return Ok::<_,String>((Some(false),Some(false),None,json!({"model_group":group,"message":"最新模型已计算，该股票不再满足冻结阈值与资格条件"})));}
            let candidate=candidate.unwrap();
            if config["preset"]=="model_hit"{return Ok((Some(true),Some(true),None,json!({"candidate":candidate,"model_group":group,"message":"最近完成日满足该冻结模型条件；下一开盘可成交性未知"})));}
            if !continuous{return Err("等待交易时段和新鲜报价；休市不发送当前条件确认".into());}
            let (quote,source)=quotes.get(symbol).ok_or("缺当日新鲜报价，暂停盘中条件")?;
            if quote.name.is_empty()||crate::market_rules::is_st(&quote.name)||crate::market_rules::is_delisting(&quote.name)||crate::market_rules::is_new_listing(&quote.name){return Err("当前股票名称未知或ST/退市/新股，停止观察".into());}
            if !quote.price.is_finite()||quote.price<=0.||quote.volume==0||!quote.turnover.is_finite()||quote.turnover<=0.{return Err("当前价格或量额无效".into());}
            if !quote.prev_close.is_finite()||quote.prev_close<=0.||(quote.prev_close-candidate["close"].as_f64().ok_or("缺冻结价格")?).abs()>0.011{return Err("实时报价昨收与模型版本不同，重新核验后再观察".into());}
            let node=custom.ok_or("自定义条件缺失")?;
            let checked=Utc::now();datasource::a_share_calendar::continuous(checked)?;if !crate::alerts::is_fresh_trading_quote(quote,checked.with_timezone(&cst())){return Err("组合核验后报价已过期，状态未知".into());}
            let (sector,mainline_error)=if node.needs_mainline(){match mainline_for_symbol(db,symbol,&day,watch["observation"]["last_mainline_code"].as_str()){Ok(value)=>(value,None),Err(e)=>(None,Some(e))}}else{(None,None)};
            let facts=crate::condition_logic::Facts{intraday:None,mainline:sector.as_ref().map(|s|s["strong"]==true&&s["leaders"].as_array().is_some_and(|rows|rows.iter().any(|r|r["symbol"]==symbol))),change_pct:Some((quote.price/quote.prev_close-1.)*100.)};
            let combined=node.evaluate(&facts);let message=match combined{Some(true)=>"模型命中且自定义组合成立",Some(false)=>"模型命中，自定义组合尚未成立或已失效",None=>"模型命中，自定义组合缺少必要证据，状态未知"};
            Ok((Some(true),combined,None::<String>,json!({"candidate":candidate,"model_group":group,"condition_tree":node,"condition_scope":"single_stock_with_frozen_model_gate","condition_facts":{"mainline":facts.mainline,"change_pct":facts.change_pct},"mainline_snapshot":sector,"missing_evidence":{"mainline":mainline_error},"quote":{"price":quote.price,"prev_close":quote.prev_close,"timestamp":quote.timestamp,"source":source},"message":message})))
        }.await;
        let (hit, combined, shape, evidence) = match observed {
            Ok(v) => v,
            Err(message) => (None, None, None, json!({"message":message})),
        };
        let previous = &watch["observation"];
        let (transition, mut next) = edge(previous, hit, combined, shape.as_deref());
        next["checked_at"] = json!(now.to_rfc3339());
        next["last_scan_day"] = json!(day);
        next["message"] = evidence["message"].clone();
        next["state"] = json!(if hit == Some(false) {
            "invalidated"
        } else if hit.is_none() || combined.is_none() {
            "waiting_data"
        } else if combined == Some(true) {
            "confirmed"
        } else {
            "watching"
        });
        next["latest_evidence"] = evidence.clone();
        if evidence["mainline_snapshot"]["sector_code"].is_string() {
            next["last_mainline_code"] = evidence["mainline_snapshot"]["sector_code"].clone();
        }
        let event=transition.map(|kind|{
            let key=hex::encode(Sha256::digest(json!({"watch":id,"day":day,"today":now.with_timezone(&cst()).date_naive().to_string(),"event":kind,"model_sha":config["model_sha256"]}).to_string().as_bytes()));
            let saved=json!({"schema":"model-condition-event-v1","event_key":key,"watch_id":id,"event":kind,"symbol":symbol,"model_name":config["model_name"],"model_id":config["model_id"],"preset":config["preset"],"as_of":day,"checked_at":now.to_rfc3339(),"production_admission":false,"message":evidence["message"],"evidence":evidence,"config":config});(key,saved)
        });
        let emitted = db.commit_condition_observation(
            id,
            &next,
            event.as_ref().map(|(key, value)| (key.as_str(), value)),
        )?;
        if emitted {
            let (key, event) = event.unwrap();
            let kind = event["event"].as_str().unwrap_or("observation");
            let title = if kind == "conditions_confirmed" {
                "条件新成立"
            } else {
                "观察条件失效"
            };
            let row=notices.entry(symbol.to_owned()).or_insert_with(||json!({"signal_kind":"research","signal_tag":"模型条件观察","title":format!("{symbol} · {title}"),"body":"","code":symbol,"condition_event":event,"condition_events":[],"production_admission":false}));
            row["condition_events"].as_array_mut().unwrap().push(event);
            row["body"] = json!(format!(
                "{}项模型观察条件变化。{}；仍未准入，查看当时证据。",
                row["condition_events"].as_array().unwrap().len(),
                next["message"].as_str().unwrap_or("查看证据")
            ));
            row["event_key"] = json!(key);
        }
    }
    db.set_setting("model_condition_watch_status",&json!({"checked_at":now.to_rfc3339(),"enabled":all,"checked":watches.len(),"limit":LIMIT,"message":"仅运行已加入的观察条件，轮转检查；未知不当满足，加入时静默。模型日线更新需点筛选或启用原模型自动观察"}).to_string()).map_err(|e|e.to_string())?;
    Ok(notices.into_values().collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn save_scan_fixture(db: &Database, day: &str, hit: bool) -> i64 {
        let id = db
            .create_research_job(
                "scan",
                &json!({"kind":"scan","models":["breadth22_h20"],"comparisons":["baseline"]}),
                1,
            )
            .unwrap();
        db.activate_research_job(id).unwrap();
        let candidates = if hit {
            json!([{"symbol":"sz000001","score":0.02,"close":10.,"as_of":day,"signal_eligible":true,"threshold":0.}])
        } else {
            json!([])
        };
        db.checkpoint_research_job(id,&json!({"key":"breadth22_h20:baseline:20","model_id":"breadth22_h20","group":{"model_id":"breadth22_h20","name":"test frozen model","as_of":day,"production_admission":false,"model_sha256":"model","runner_sha256":"runner","source_fingerprint":id.to_string(),"candidates":candidates,"current_scores":[]}})).unwrap();
        db.finish_research_job(id, "complete", "fixture").unwrap();
        id
    }
    #[tokio::test]
    async fn custom_group_identity_keeps_model_gate_and_unknown_evidence(){
        let root=std::env::temp_dir().join(format!("custom-condition-{}",uuid::Uuid::new_v4()));let db=Database::open(root.clone()).unwrap();
        let now=Utc::now();let day=crate::commands::mainline::completed_day(now).unwrap().to_string();let job=save_scan_fixture(&db,&day,true);
        let mut request=WatchRequest{job_id:job,model_id:"breadth22_h20".into(),symbol:"sz000001".into(),preset:"custom".into(),condition_tree:Some(crate::condition_logic::ConditionTree::Or{children:vec![crate::condition_logic::ConditionTree::Mainline{},crate::condition_logic::ConditionTree::ChangeAbove{value:-30.0}]})};
        let id=register_watch(&db,&request).unwrap();assert_eq!(register_watch(&db,&request).unwrap(),id);
        request.condition_tree=Some(crate::condition_logic::ConditionTree::And{children:vec![crate::condition_logic::ConditionTree::Mainline{},crate::condition_logic::ConditionTree::ChangeAbove{value:-30.0}]});
        assert_ne!(register_watch(&db,&request).unwrap(),id,"AND and OR retain different saved identities");
        let manager=DataSourceManager::new();assert!(tick(&db,&manager,now).await.unwrap().is_empty());
        assert!(db.condition_watches().unwrap().iter().all(|w|w["observation"]["state"]=="waiting_data"),"missing live quote cannot satisfy even a -30% condition");
        save_scan_fixture(&db,&day,false);let notices=tick(&db,&manager,now).await.unwrap();assert_eq!(notices.len(),1);assert_eq!(notices[0]["condition_events"].as_array().unwrap().len(),2);
        assert!(notices[0]["condition_events"].as_array().unwrap().iter().all(|v|v["event"]=="model_invalidated"),"custom OR cannot bypass the frozen-model gate");
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn daily_watch_is_quiet_on_add_deduplicates_loss_and_saves_evidence_when_muted() {
        let db_root = std::env::temp_dir().join(format!("condition-tick-{}", uuid::Uuid::new_v4()));
        let db = Database::open(db_root.clone()).unwrap();
        let manager = DataSourceManager::new();
        let now = Utc::now();
        let day = crate::commands::mainline::completed_day(now)
            .unwrap()
            .to_string();
        let job = save_scan_fixture(&db, &day, true);
        let request = WatchRequest {
            job_id: job,
            model_id: "breadth22_h20".into(),
            symbol: "sz000001".into(),
            preset: "model_hit".into(),
            condition_tree: None,
        };
        let id = register_watch(&db, &request).unwrap();
        assert_eq!(register_watch(&db, &request).unwrap(), id);
        assert!(tick(&db, &manager, now).await.unwrap().is_empty());
        assert!(tick(&db, &manager, now).await.unwrap().is_empty());
        save_scan_fixture(&db, &day, false);
        let notices = tick(&db, &manager, now).await.unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["condition_event"]["event"], "model_invalidated");
        let key = notices[0]["condition_event"]["event_key"].as_str().unwrap();
        let saved = db.condition_event(key).unwrap();
        assert!(saved["evidence"]["model_group"].get("candidates").is_none());
        assert!(saved["evidence"]["model_group"]
            .get("current_scores")
            .is_none());
        assert!(tick(&db, &manager, now).await.unwrap().is_empty());
        db.set_setting("alerts_enabled", "0").unwrap();
        save_scan_fixture(&db, &day, true);
        let muted = tick(&db, &manager, now).await.unwrap();
        assert_eq!(muted.len(), 1);
        assert_eq!(muted[0]["condition_event"]["event"], "conditions_confirmed"); // Delivery policy saves history but suppresses the popup.
        let check = rusqlite::Connection::open_with_flags(
            db_root.join("bull-arrives.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let count: i64 = check
            .query_row("SELECT COUNT(*) FROM model_condition_events", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(db.condition_event(key).unwrap(), saved);
        db.enable_condition_watch(id, false).unwrap();
        save_scan_fixture(&db, &day, false);
        assert!(tick(&db, &manager, now).await.unwrap().is_empty());
    }
    #[test]
    fn model_hit_remains_quiet_across_days_until_real_loss_or_reentry() {
        let previous = json!({"last_known_model_hit":true,"last_known_combined":true});
        let (event, same) = edge(&previous, Some(true), Some(true), None);
        assert!(event.is_none());
        let (event, lost) = edge(&same, Some(false), Some(false), None);
        assert_eq!(event, Some("model_invalidated"));
        assert_eq!(
            edge(&lost, Some(true), Some(true), None).0,
            Some("conditions_confirmed")
        );
    }
    #[test]
    fn mainline_bound_observation_distinguishes_verified_weak_from_stale_or_incomplete() {
        let db = Database::open(
            std::env::temp_dir().join(format!("condition-mainline-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let save = |strong: bool, complete: bool, day: &str, has_leader: bool| {
            let mut row = json!({"sector_code":"SW801080","sector_name":"电子","kind":"industry","as_of":day,"strong":strong,"complete":complete,"leaders":if has_leader{json!([{"symbol":"sz000001"}])}else{json!([])}});
            let hash = hex::encode(Sha256::digest(serde_json::to_vec(&row).unwrap()));
            row["fingerprint"] = json!(hash);
            db.set_setting("mainline_snapshot_SW801080", &row.to_string())
                .unwrap();
        };
        save(true, true, "2026-09-30", true);
        assert_eq!(
            mainline_for_symbol(&db, "sz000001", "2026-09-30", None)
                .unwrap()
                .unwrap()["strong"],
            true
        );
        save(false, true, "2026-09-30", false);
        assert_eq!(
            mainline_for_symbol(&db, "sz000001", "2026-09-30", Some("SW801080"))
                .unwrap()
                .unwrap()["strong"],
            false
        );
        save(true, false, "2026-09-30", true);
        assert!(
            mainline_for_symbol(&db, "sz000001", "2026-09-30", Some("SW801080"))
                .unwrap()
                .is_none()
        );
        save(true, true, "2026-09-29", true);
        assert!(
            mainline_for_symbol(&db, "sz000001", "2026-09-30", Some("SW801080"))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn known_combination_loss_notifies_but_missing_data_stays_unknown() {
        let old = json!({"last_known_model_hit":true,"last_known_combined":true,"last_shape":"confirmed"});
        assert!(edge(&old, Some(true), None, Some("confirmed")).0.is_none());
        let (event, lost) = edge(&old, Some(true), Some(false), Some("confirmed"));
        assert_eq!(event, Some("conditions_invalidated"));
        assert!(edge(&lost, Some(true), Some(false), Some("confirmed"))
            .0
            .is_none());
    }
    #[test]
    fn unknown_is_neither_confirmation_nor_loss_of_last_known_state() {
        let previous = json!({"last_known_model_hit":true,"last_known_combined":false,"last_shape":"waiting_retest"});
        let (event, next) = edge(&previous, None, None, None);
        assert!(event.is_none());
        assert_eq!(next["last_known_model_hit"], true);
        let (event, next) = edge(&next, Some(true), Some(true), Some("confirmed"));
        assert_eq!(event, Some("conditions_confirmed"));
        assert!(edge(&next, Some(true), Some(true), Some("confirmed"))
            .0
            .is_none());
        assert_eq!(
            edge(&next, Some(true), Some(false), Some("invalidated")).0,
            Some("shape_invalidated")
        );
        assert_eq!(
            edge(&next, Some(false), Some(false), None).0,
            Some("model_invalidated")
        );
    }
    #[test]
    fn retired_minute_conditions_are_rejected_without_changing_model_or_custom_gates(){
        assert!(active_condition(&json!({"preset":"model_hit"})).unwrap().is_none());
        for preset in ["model_confirm","model_mainline_confirm"] {assert!(active_condition(&json!({"preset":preset})).unwrap_err().contains("已停用"));}
        assert!(active_condition(&json!({"preset":"custom","condition_tree":{"op":"or","children":[{"op":"intraday"},{"op":"mainline"}]}})).unwrap_err().contains("已停用"));
        assert!(active_condition(&json!({"preset":"custom","condition_tree":{"op":"and","children":[{"op":"mainline"},{"op":"change_above","value":1}]}})).unwrap().is_some());
        assert_eq!(presets().as_array().unwrap().len(),1);
    }
    #[test]
    fn creating_watch_rejects_legacy_or_unverified_sources() {
        let db = Database::open(
            std::env::temp_dir().join(format!("watch-source-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let request = WatchRequest {
            job_id: 1,
            model_id: "panic_second_test".into(),
            symbol: "sz000001".into(),
            preset: "model_confirm".into(),
            condition_tree: None,
        };
        assert!(scan_hit(&db, &request).is_err());
    }
}
