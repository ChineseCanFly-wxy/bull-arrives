use crate::{
    datasource::eastmoney_universe::{
        self as market, Board, FilterCapabilities,
    },
    db::{
        research_loop::{assessment, ExperimentView, ResearchConfig},
        simulation::{AccountInput, Target},
        Database,
    },
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use tauri::State;
use serde_json::{json, Value};
#[cfg(test)]
use crate::datasource::eastmoney_universe::{MarketFilter,PresetInfo};
static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRunnerConfig {pub research_root:String,pub python:String,pub snapshot:String,pub index:String}
pub(crate) fn model_config(db:&Database)->Result<ModelRunnerConfig,String>{
    if let Some(raw)=db.get_setting("model_runner_config").map_err(|e|e.to_string())? {return serde_json::from_str(&raw).map_err(|_|"模型研究路径配置损坏".into());}
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("缺项目目录")?;
    let mut candidates=vec![root.join(".venv/Scripts/python.exe")];
    if let Some(home)=dirs::home_dir(){for path in ["AppData/Local/Programs/Python/Python312/python.exe","AppData/Local/Programs/Python/Python313/python.exe",".cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe"]{candidates.push(home.join(path));}}
    let python=candidates.into_iter().find(|p|p.is_file()).map(|p|p.to_string_lossy().into_owned()).unwrap_or_else(||"python".into());
    Ok(ModelRunnerConfig{research_root:root.to_string_lossy().into_owned(),python,snapshot:root.join("research/ashare-open-2026-10-01/stockdb-live/exports").to_string_lossy().into_owned(),index:root.join("research/ashare-open-2026-10-01/online/index-sh-000300.ndjson").to_string_lossy().into_owned()})
}
#[tauri::command]
pub fn research_model_config(db:State<'_,Arc<Database>>,config:Option<ModelRunnerConfig>)->Result<ModelRunnerConfig,String>{
    if let Some(c)=config {if c.python.trim().is_empty()||c.research_root.trim().is_empty()||c.snapshot.trim().is_empty()||c.index.trim().is_empty(){return Err("请填写Python、项目、快照与CSI300文件路径".into());}db.set_setting("model_runner_config",&serde_json::to_string(&c).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;}
    model_config(&db)
}
pub(crate) fn trusted_runner(c:&ModelRunnerConfig)->Result<std::path::PathBuf,String>{
    let root=std::path::Path::new(&c.research_root);let runner=root.join("research/research-center-runner/model_runner.py");
    use sha2::{Digest,Sha256};
    for (path,expected) in [(&runner,include_bytes!("../../../research/research-center-runner/model_runner.py").as_slice()),(&root.join("research/research-center-runner/registry.json"),include_bytes!("../../../research/research-center-runner/registry.json").as_slice())] {
        let meta=std::fs::symlink_metadata(path).map_err(|_|format!("受信研究文件缺失：{}；请选择包含快照及模型缓存的项目目录",path.display()))?;
        if !meta.is_file()||meta.file_type().is_symlink()||Sha256::digest(std::fs::read(path).map_err(|e|e.to_string())?)!=Sha256::digest(expected){return Err("可信runner/registry与此应用源码版本不符，禁止执行任意研究代码".into());}
    }
    Ok(runner)
}
fn trusted_execution_runner(c:&ModelRunnerConfig,model:&str)->Result<std::path::PathBuf,String>{
    if model!="fundamental37_h20"{return trusted_runner(c);}
    use sha2::{Digest,Sha256};let root=std::path::Path::new(&c.research_root);
    let runner=root.join("research/ashare-fundamental-2026-10-01/replay_adapter.py");
    for (path,expected) in [(&runner,include_bytes!("../../../research/ashare-fundamental-2026-10-01/replay_adapter.py").as_slice()),(&root.join("research/ashare-fundamental-2026-10-01/replay-sources.json"),include_bytes!("../../../research/ashare-fundamental-2026-10-01/replay-sources.json").as_slice())]{
        let meta=std::fs::symlink_metadata(path).map_err(|_|"缺少受信财务回放程序或来源清单")?;
        if !meta.is_file()||meta.file_type().is_symlink()||Sha256::digest(std::fs::read(path).map_err(|e|e.to_string())?)!=Sha256::digest(expected){return Err("财务回放程序/来源版本不同，拒绝执行".into());}
    }
    Ok(runner)
}
fn execution_config(mut c:ModelRunnerConfig,model:&str)->Result<ModelRunnerConfig,String>{
    if model=="fundamental37_h20"{
        let source:Value=serde_json::from_str(include_str!("../../../research/ashare-fundamental-2026-10-01/replay-sources.json")).map_err(|_|"财务冻结路径清单损坏")?;
        let root=std::path::Path::new(&c.research_root);
        let snapshot=root.join(source["files"]["snapshot"]["path"].as_str().ok_or("缺财务冻结矩阵路径")?);
        c.snapshot=snapshot.parent().ok_or("财务冻结矩阵目录无效")?.to_string_lossy().into_owned();
        c.index=root.join(source["files"]["index"]["path"].as_str().ok_or("缺财务冻结指数路径")?).to_string_lossy().into_owned();
    }
    Ok(c)
}
pub(crate) fn check_bound_run(raw:&str)->Result<Value,String>{
    let value=crate::db::model_research::decode_run(raw)?;
    if value["model_id"]=="fundamental37_h20"{
        use sha2::{Digest,Sha256};
        let source:Value=serde_json::from_str(include_str!("../../../research/ashare-fundamental-2026-10-01/replay-sources.json")).map_err(|e|e.to_string())?;
        for (key,row) in source["files"].as_object().ok_or("财务来源清单损坏")?{if value["input_sha256"][key]!=row["sha256"]{return Err("财务账本来源集合不同".into());}}
        let manifest:Value=serde_json::from_str(include_str!("../../../research/ashare-fundamental-2026-10-01/adapter-checks/manifest.json")).map_err(|e|e.to_string())?;
        if value["runner_sha256"].as_str()!=Some(&hex::encode(Sha256::digest(include_bytes!("../../../research/ashare-fundamental-2026-10-01/replay_adapter.py"))))
            ||value["input_sha256"]["replay_sources"].as_str()!=Some(&hex::encode(Sha256::digest(include_bytes!("../../../research/ashare-fundamental-2026-10-01/replay-sources.json"))))
            ||value["model_sha256"]!=source["files"]["fundamental37_h20_model2026"]["sha256"]||value["score_cache_sha256"]!=source["files"]["fundamental37_h20_score"]["sha256"]
            ||value["mode"]!="replay"||value["forward_supported"]!=false||value["financial_history_version_certified"]!=false||value["signal_threshold"].as_f64()!=Some(0.)||value["position_policy"]!="full8pct target"{return Err("财务模型、阈值、程序或回放边界不同".into());}
        for key in ["model_name","source_run_id","score_semantic"]{if value[key]!=manifest[key]{return Err("财务模型身份或标签语义不同".into());}}
        return Ok(value);
    }
    let registry:Value=serde_json::from_str(include_str!("../../../research/research-center-runner/registry.json")).map_err(|e|e.to_string())?;
    for (key,row) in registry["files"].as_object().ok_or("受信registry损坏")? {if value["input_sha256"][key]!=row["sha256"] {return Err("导入账本不属于此冻结输入集合".into());}}
    use sha2::{Digest,Sha256};
    if value["runner_sha256"].as_str()!=Some(&hex::encode(Sha256::digest(include_bytes!("../../../research/research-center-runner/model_runner.py")))) {return Err("账本runner版本未核验".into());}
    let id=value["model_id"].as_str().unwrap();
    if value["model_sha256"]!=registry["files"][format!("{id}_model2026")]["sha256"]||value["score_cache_sha256"]!=registry["files"][format!("{id}_score")]["sha256"] {return Err("账本模型身份不符".into());}
    let spec=&registry["models"][id];
    if value["source_run_id"]!=spec["source_run_id"]||value["model_name"]!=spec["name"]||value["signal_threshold"]!=spec["signal_threshold"]||value["score_semantic"]!=spec["score_semantic"] {return Err("模型名称、标签语义、固定阈值或研究来源不符".into());}
    let policy=if value["comparison"]=="staged"{"40/30/30 initial/add, weak reduce/profit protection"}else{"full8pct target"};
    if value["position_policy"]!=policy {return Err("固定仓位执行方式不符".into());}
    Ok(value)
}
pub(crate) fn model_completed_day(now:chrono::DateTime<chrono::Utc>)->Result<chrono::NaiveDate,String>{
    use chrono::Timelike;
    let local=now.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
    let mut day=local.date_naive();
    if local.hour()<16{day-=chrono::Duration::days(1);}
    for _ in 0..35{if crate::datasource::trading_calendar::is_trading_day_at(now,day)?{return Ok(day);}day-=chrono::Duration::days(1);}
    Err("无法确认模型最近完成交易日".into())
}
fn previous_model_session(day:&str)->Result<String,String>{
    let mut date=chrono::NaiveDate::parse_from_str(day,"%Y-%m-%d").map_err(|_|"模型日期无效")?-chrono::Duration::days(1);
    for _ in 0..35{if crate::datasource::trading_calendar::is_trading_day_at(chrono::Utc::now(),date)?{return Ok(date.to_string());}date-=chrono::Duration::days(1);}
    Err("无法核对模型前一交易日，不能拼接近期行情".into())
}
pub(crate) fn market_data_status(db:&Database,c:&ModelRunnerConfig)->Value{
    let snapshot=std::path::Path::new(&c.snapshot);
    let parent=snapshot.parent().unwrap_or(snapshot);
    std::fs::read_to_string(parent.join("refresh-audit.json")).ok().and_then(|raw|serde_json::from_str::<Value>(&raw).ok()).map(|audit|json!({"mode":audit["mode"].as_str().unwrap_or("stockdb"),"as_of":audit["as_of"],"stockdb_error":audit["stockdb_error"],"coverage":audit["checks"]["valid_coverage"],"required_coverage":audit["checks"]["required_coverage"],"providers":audit["checks"]["providers"],"checked_at":chrono::Utc::now().to_rfc3339()})).unwrap_or_else(||{let _=db;json!({"mode":"saved_snapshot","message":"原模型已保存快照，近期来源尚未重新核对"})})
}
pub(crate) fn research_gate() -> &'static tokio::sync::Semaphore {
    GATE.get_or_init(||tokio::sync::Semaphore::new(1))
}

pub(crate) fn own_research_process(child:&mut std::process::Child)->Result<crate::agent::ProcessJob,String>{
    match crate::agent::ProcessJob::assign(child){Ok(job)=>Ok(job),Err(error)=>{let _=child.kill();let _=child.wait();Err(format!("研究进程隔离失败，已停止本次计算：{error}"))}}
}

fn cancel_owned_process(child:&mut std::process::Child,control:Option<(&Database,i64)>) -> Result<(),String> {
    if control.is_some_and(|(db,id)|db.research_job_cancelled(id)) {
        let _=child.kill(); let _=child.wait();
        return Err("研究作业已取消；已完成步骤保留".into());
    }
    Ok(())
}

fn execute_model(c:ModelRunnerConfig,model:String,hold:i64,comparison:String,mode:String,start:Option<i64>)->Result<String,String>{
    execute_model_controlled(c,model,hold,comparison,mode,start,None)
}

pub(crate) fn execute_model_controlled(c:ModelRunnerConfig,model:String,hold:i64,comparison:String,mode:String,start:Option<i64>,control:Option<(&Database,i64)>)->Result<String,String>{
    crate::db::model_research::validate_task(&json!({"model_id":model,"holding_days":hold,"comparison":comparison}))?;
    if !matches!(mode.as_str(),"replay"|"forward"){return Err("运行模式无效".into());}
    if model=="fundamental37_h20"&&mode!="replay"{return Err("财务37仅支持冻结历史回放，尚未核验财务实时更新，不能开前向账户".into());}
    let c=execution_config(c,&model)?;
    let runner=trusted_execution_runner(&c,&model)?;let work=workspace()?.join("model-runs");std::fs::create_dir_all(&work).map_err(|e|e.to_string())?;
    let key=uuid::Uuid::new_v4().to_string();let out=work.join(format!("{key}.json"));let stdout=work.join(format!("{key}.log"));let stderr=work.join(format!("{key}.error.log"));
    let mut command=std::process::Command::new(&c.python);
    command.arg(&runner).args(["--model-id",&model,"--holding-days",&hold.to_string(),"--comparison",&comparison,"--mode",&mode,"--snapshot",&c.snapshot,"--index",&c.index,"--output"]).arg(&out);
    if let Some(day)=start {command.args(["--forward-start",&day.to_string()]);}
    command.env("PYTHONIOENCODING","utf-8").current_dir(&c.research_root).stdin(std::process::Stdio::null()).stdout(std::fs::File::create(&stdout).map_err(|e|e.to_string())?).stderr(std::fs::File::create(&stderr).map_err(|e|e.to_string())?);
    #[cfg(windows)]{use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
    let mut child=command.spawn().map_err(|e|format!("Python无法运行：{e}；选择真实Python可执行文件，需要numpy与现有模型scikit-learn环境，无需Claude"))?;
    let _process_job=own_research_process(&mut child)?;
    let began=std::time::Instant::now();
    loop {
        cancel_owned_process(&mut child,control)?;
        if let Some(status)=child.try_wait().map_err(|e|e.to_string())? {if !status.success(){let err=std::fs::read_to_string(&stderr).unwrap_or_default();return Err(format!("模型研究停止：{}；日志 {}",err.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>(),stderr.display()));}break;}
        if began.elapsed()>std::time::Duration::from_secs(300){let _=child.kill();let _=child.wait();return Err("模型研究超过300秒已终止，本次未导入账本；检查依赖/数据规模后手动重试".into());}
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    let meta=std::fs::symlink_metadata(&out).map_err(|_|"runner没有输出实际账本")?;
    if !meta.is_file()||meta.file_type().is_symlink()||meta.len()>32*1024*1024 {return Err("runner输出文件无效".into());}
    let raw=std::fs::read_to_string(out).map_err(|e|e.to_string())?;check_bound_run(&raw)?;Ok(raw)
}
fn refresh_model_config(c:ModelRunnerConfig,endpoint:String,day:String,require_current:bool)->Result<ModelRunnerConfig,String>{
    refresh_model_config_controlled(c,endpoint,day,require_current,None)
}

fn market_refresh_failure(raw:&str)->String{let last=raw.lines().rev().find(|line|!line.trim().is_empty()).unwrap_or("未返回错误说明，请检查网络与历史数据");last.strip_prefix("ValueError: ").unwrap_or(last).chars().take(1200).collect()}

pub(crate) fn refresh_model_config_controlled(mut c:ModelRunnerConfig,endpoint:String,day:String,require_current:bool,control:Option<(&Database,i64)>)->Result<ModelRunnerConfig,String>{
    trusted_runner(&c)?;
    let script=std::path::Path::new(&c.research_root).join("research/research-center-runner/refresh_market.py");
    use sha2::{Digest,Sha256};
    let meta=std::fs::symlink_metadata(&script).map_err(|_|"本机缺受信增量刷新程序")?;
    if !meta.is_file()||meta.file_type().is_symlink()||Sha256::digest(std::fs::read(&script).map_err(|e|e.to_string())?)!=Sha256::digest(include_bytes!("../../../research/research-center-runner/refresh_market.py")){return Err("增量刷新程序版本不同，停止自动读取".into());}
    let helper=std::path::Path::new(&c.research_root).join("research/research-center-runner/recent_market_fallback.py");
    if std::fs::read(&helper).map(|bytes|Sha256::digest(bytes)!=Sha256::digest(include_bytes!("../../../research/research-center-runner/recent_market_fallback.py"))).unwrap_or(true) || std::fs::symlink_metadata(&helper).map(|m|!m.is_file()||m.file_type().is_symlink()).unwrap_or(true){return Err("近期行情备用程序缺失或版本不同，停止自动读取".into());}
    let previous=previous_model_session(&day)?;
    let key=uuid::Uuid::new_v4().to_string();let work=workspace()?.join("snapshots");std::fs::create_dir_all(&work).map_err(|e|e.to_string())?;
    let out=work.join(&key);let stdout=work.join(format!("{key}.log"));let stderr=work.join(format!("{key}.error.log"));
    let mut command=std::process::Command::new(&c.python);
    command.arg(&script).args(["--source",&c.snapshot,"--index",&c.index,"--endpoint",&endpoint,"--as-of",&day,"--previous-as-of",&previous,"--output"]).arg(&out).current_dir(&c.research_root).env("PYTHONIOENCODING","utf-8").stdin(std::process::Stdio::null()).stdout(std::fs::File::create(&stdout).map_err(|e|e.to_string())?).stderr(std::fs::File::create(&stderr).map_err(|e|e.to_string())?);
    #[cfg(windows)]{use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
    let mut child=command.spawn().map_err(|e|format!("Python增量刷新启动失败：{e}"))?;let _process_job=own_research_process(&mut child)?;let began=std::time::Instant::now();
    loop {cancel_owned_process(&mut child,control)?;if let Some(status)=child.try_wait().map_err(|e|e.to_string())? {if !status.success(){return Err(format!("只读行情刷新失败：{}",market_refresh_failure(&std::fs::read_to_string(&stderr).unwrap_or_default())));}break;}if began.elapsed()>std::time::Duration::from_secs(300){let _=child.kill();let _=child.wait();return Err("StockDB及近期行情备用刷新超时，原账本保留".into());}std::thread::sleep(std::time::Duration::from_millis(150));}
    let text=std::fs::read_to_string(&stdout).map_err(|e|e.to_string())?;
    let result:Value=serde_json::from_str(text.lines().rev().find(|l|l.starts_with('{')).ok_or("增量刷新没有结果")?).map_err(|_|"增量刷新结果无效")?;
    if !matches!(result["state"].as_str(),Some("updated"|"up_to_date"|"waiting_market_data")){return Err("未知增量数据状态".into());}
    if require_current && result["as_of"].as_str()!=Some(day.as_str()){return Err(format!("最近完成交易日{day}的行情尚未齐备，StockDB和近期行情备用均未取得齐备日线；等待数据恢复，不从旧日期倒填前向收益"));}
    c.snapshot=result["snapshot"].as_str().ok_or("刷新缺快照路径")?.into();c.index=result["index"].as_str().ok_or("刷新缺指数路径")?.into();
    if result["state"]=="updated" {
        for path in [&c.snapshot,&c.index] {if !std::path::Path::new(path).canonicalize().map_err(|e|e.to_string())?.starts_with(out.canonicalize().map_err(|e|e.to_string())?){return Err("新增研究导出离开独立输出目录".into());}}
    }
    Ok(c)
}
pub(crate) async fn run_model(db:&Database,model:String,hold:i64,comparison:String,mode:String,continuation:Option<i64>)->Result<i64,String>{
    if model=="fundamental37_h20"&&(mode!="replay"||continuation.is_some()){return Err("财务37的更新与前向执行尚未核验，只能运行冻结历史回放".into());}
    let financial=model=="fundamental37_h20";let mut config=model_config(db)?;
    let start=if let Some(id)=continuation {let row=db.model_run(id)?;if row["model_id"]!=model||row["holding_days"]!=hold||row["comparison"]!=comparison||mode!="forward" {return Err("延续必须保持原模型、期限与执行对照".into());}Some(row["forward_start"].as_i64().ok_or("账户不是前向模式")?)}else{None};
    if mode=="forward" {
        let endpoint=db.get_setting("local_history_url").map_err(|e|e.to_string())?.unwrap_or_else(||"http://127.0.0.1:7899".into());
        let day=model_completed_day(chrono::Utc::now())?.to_string();
        let require_current=true;
        config=tokio::task::spawn_blocking(move||refresh_model_config(config,endpoint,day,require_current)).await.map_err(|e|e.to_string())??;
    }
    if mode=="forward"{let status=market_data_status(db,&config);db.set_setting("model_market_data_status",&status.to_string()).map_err(|e|e.to_string())?;}
    let saved_config=config.clone();
    let raw=tokio::task::spawn_blocking(move||execute_model(config,model,hold,comparison,mode,start)).await.map_err(|e|e.to_string())??;
    let id=db.save_model_run(&raw,continuation)?;
    if !financial{db.set_setting("model_runner_config",&serde_json::to_string(&saved_config).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;}
    db.set_setting("model_research_last_error","").map_err(|e|e.to_string())?;Ok(id)
}
#[tauri::command]
pub async fn research_model_run(db:State<'_,Arc<Database>>,model_id:String,holding_days:i64,comparison:String,mode:String,continuation:Option<i64>)->Result<i64,String>{
    let _permit=GATE.get_or_init(||tokio::sync::Semaphore::new(1)).try_acquire().map_err(|_|"研究任务正在运行")?;
    let result=run_model(&db,model_id,holding_days,comparison,mode,continuation).await;
    if let Err(e)=&result {let _=db.set_setting("model_research_last_error",e);}result
}
#[tauri::command]
pub fn research_market_data_status(db:State<'_,Arc<Database>>)->Result<Value,String>{crate::model_data_health::status(&db,chrono::Utc::now())}
#[tauri::command]
pub fn research_model_runs(db:State<'_,Arc<Database>>)->Result<Value,String>{Ok(json!({"runs":db.model_runs()?,"tasks":db.model_tasks()?,"config":model_config(&db)?,"last_error":db.get_setting("model_research_last_error").map_err(|e|e.to_string())?}))}
#[tauri::command]
pub async fn research_import_model_run(db:State<'_,Arc<Database>>,path:String)->Result<i64,String>{
    let _permit=GATE.get_or_init(||tokio::sync::Semaphore::new(1)).try_acquire().map_err(|_|"另一研究任务正在运行")?;
    let meta=std::fs::symlink_metadata(&path).map_err(|e|e.to_string())?;
    if !meta.is_file()||meta.file_type().is_symlink()||meta.len()>32*1024*1024{return Err("导入必须是32MiB以内普通JSON账本".into());}
    let raw=std::fs::read_to_string(path).map_err(|e|e.to_string())?;
    import_model_raw(&db,&raw).await
}
async fn import_model_raw(db:&Database,raw:&str)->Result<i64,String>{
    let offered=check_bound_run(raw)?;
    let config=execution_config(model_config(db)?,offered["model_id"].as_str().unwrap())?;
    use sha2::{Digest,Sha256};
    for (key,path) in [("snapshot",std::path::Path::new(&config.snapshot).join("matrices.npz")),("metadata",std::path::Path::new(&config.snapshot).join("matrices-metadata.json")),("index",std::path::PathBuf::from(&config.index))] {
        // A whole-file SHA binds the exact replay inputs, not merely a same-size export.
        let bytes=std::fs::read(path).map_err(|_|"请在本机路径中选择导入账本对应的兼容快照和指数")?;
        if offered["data_sha256"][key].as_str()!=Some(&hex::encode(Sha256::digest(bytes))) {return Err("导入账本的数据指纹与本机快照不同，请选择对应snapshot/CSI300后重放核验".into());}
    }
    let model=offered["model_id"].as_str().unwrap().to_owned();let hold=offered["holding_days"].as_i64().unwrap();let comparison=offered["comparison"].as_str().unwrap().to_owned();let mode=offered["mode"].as_str().unwrap().to_owned();let start=offered["forward_start"].as_i64();
    let actual_raw=tokio::task::spawn_blocking(move||execute_model(config,model,hold,comparison,mode,start)).await.map_err(|e|e.to_string())??;
    let actual=check_bound_run(&actual_raw)?;
    for key in ["ledger","signal_watch","current_scores","accounting","action_coverage","data_sha256","input_sha256","model_sha256","score_cache_sha256","runner_sha256","source_run_id","score_semantic","signal_threshold","position_policy","as_of","state"] {
        if offered[key]!=actual[key] {return Err(format!("外部模型账本 {key} 与可信程序实际重算不符，拒绝导入"));}
    }
    db.save_model_run(&actual_raw,None)
}
#[tauri::command]
pub fn research_model_observe(db:State<'_,Arc<Database>>,id:i64,enabled:bool)->Result<(),String>{db.enable_model_observation(id,enabled)}

/// Same decision is used for live continuation and dated historical chain checks.
pub(crate) fn model_observation(previous:&Value,current:&Value,completed_day:&str)->Option<Value>{
    let as_of=current["as_of"].as_str()?;
    if previous["as_of"]==current["as_of"]&&previous["signal_watch"]==current["signal_watch"] {return None;}
    let before=previous["ledger"]["orders"].as_array().map_or(0,Vec::len);
    let after=current["ledger"]["orders"].as_array().map_or(0,Vec::len);
    let delta=current["ledger"]["orders"].as_array().map(|v|&v[before.min(v.len())..]);
    Some(json!({"kind":"model_research_observation","run_id":current["id"],"model_name":current["model_name"],"as_of":as_of,
        "previous_as_of":previous["as_of"],"historical_catchup":as_of!=completed_day,
        "signals":current["signal_watch"],"new_orders":after.saturating_sub(before),
        "new_buys":delta.map_or(0,|rows|rows.iter().filter(|v|v["side"]=="buy").count()),
        "new_sells":delta.map_or(0,|rows|rows.iter().filter(|v|v["side"]=="sell").count()),
        "ledger_sha256":current["content_sha256"],
        "message":"模型纸上观察已更新；订单仅为模拟，未准入。历史补齐不称实时，分数不等于胜率。"}))
}

/// Caller may surface returned events as paper-observation notifications, never buy orders.
pub async fn scheduled_model_tick(db:&Database)->Vec<Value>{
    let mut notices=Vec::new();let Ok(day)=model_completed_day(chrono::Utc::now()) else{return notices};
    if GATE.get().is_some_and(|g|g.available_permits()==0){return notices;}
    let Ok(rows)=db.enabled_model_runs() else{return notices};
    for row in rows.iter().filter(|v|v["mode"]=="forward") {
        if row["as_of"].as_str().is_some_and(|d|d>=day.to_string().as_str()){continue;}
        let marker=format!("model_tick_{}_{}",row["id"],day);
        let now=chrono::Utc::now().timestamp();
        if db.get_setting(&marker).ok().flatten().and_then(|v|v.parse::<i64>().ok()).is_some_and(|last|now-last<300){continue;}
        let _permit=match GATE.get_or_init(||tokio::sync::Semaphore::new(1)).try_acquire(){Ok(v)=>v,Err(_)=>return notices};
        let _=db.set_setting(&marker,&now.to_string());
        let result=run_model(db,row["model_id"].as_str().unwrap().into(),row["holding_days"].as_i64().unwrap(),row["comparison"].as_str().unwrap().into(),"forward".into(),row["id"].as_i64()).await;
        match result {Ok(id)=>{let _=db.set_setting(&format!("model_source_data_error_{}",row["id"]),"");if let Ok(new)=db.model_run(id){if let Some(notice)=model_observation(row,&new,&day.to_string()){notices.push(notice);}}},Err(e)=>{let _=db.set_setting("model_research_last_error",&e);let _=db.set_setting(&format!("model_source_data_error_{}",row["id"]),&json!({"as_of":day.to_string(),"error":e,"checked_at":chrono::Utc::now().to_rfc3339()}).to_string());}}
    }
    notices
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn data_failure_message_keeps_the_actual_source_reason_without_python_stack_frames(){
        let raw="Traceback (most recent call last):\n  File \"refresh_market.py\", line 360\nValueError: StockDB及近期行情备用均未取得合格日线；StockDB：未更新；行情备用：指数日期不符\n";
        assert_eq!(market_refresh_failure(raw),"StockDB及近期行情备用均未取得合格日线；StockDB：未更新；行情备用：指数日期不符");
        assert!(!market_refresh_failure("").is_empty());
    }
    #[test]
    fn completed_model_data_waits_until_1600_and_holidays_keep_the_last_session(){
        use chrono::TimeZone;
        assert_eq!(model_completed_day(chrono::Utc.with_ymd_and_hms(2026,9,28,7,59,0).unwrap()).unwrap().to_string(),"2026-09-24");
        assert_eq!(model_completed_day(chrono::Utc.with_ymd_and_hms(2026,9,28,8,0,0).unwrap()).unwrap().to_string(),"2026-09-28");
        assert_eq!(model_completed_day(chrono::Utc.with_ymd_and_hms(2026,10,4,10,0,0).unwrap()).unwrap().to_string(),"2026-09-30");
    }
    #[test]
    fn continuation_notice_is_silent_for_same_day_and_explicit_for_catchup(){
        let previous=json!({"id":1,"as_of":"2026-09-28","signal_watch":[],"ledger":{"orders":[]}});
        assert!(model_observation(&previous,&previous,"2026-09-30").is_none());
        let current=json!({"id":1,"as_of":"2026-09-29","signal_watch":[{"symbol":"sz301068"}],"ledger":{"orders":[{"side":"buy"},{"side":"sell"}]}});
        let notice=model_observation(&previous,&current,"2026-09-30").unwrap();
        assert_eq!(notice["historical_catchup"],true);assert_eq!(notice["new_orders"],2);assert_eq!(notice["new_buys"],1);assert_eq!(notice["new_sells"],1);
        assert!(model_observation(&current,&current,"2026-09-30").is_none());
    }
    #[test]fn financial_replay_contract_cannot_become_an_automatic_account(){
        let raw=include_str!("../../../research/ashare-fundamental-2026-10-01/adapter-checks/fundamental37-continuous.json");
        let mut value=check_bound_run(raw).unwrap();assert_eq!(value["ledger"]["metrics"]["completed_holding_cycles"],689);
        assert_eq!(value["current_scores"].as_array().unwrap().len(),4454);assert_eq!(value["unknown_current_scores"].as_array().unwrap().len(),1);
        value.as_object_mut().unwrap().remove("content_sha256");value["mode"]=json!("forward");let body=value.to_string();use sha2::{Digest,Sha256};
        let forged=json!({"schema":"model-run-bundle-v1","content":body,"content_sha256":hex::encode(Sha256::digest(body.as_bytes()))}).to_string();assert!(check_bound_run(&forged).is_err());
    }
    #[test]fn financial_execution_paths_do_not_follow_shared_forward_snapshot(){
        let config=ModelRunnerConfig{research_root:"C:/research-project".into(),python:"python.exe".into(),snapshot:"C:/advanced/2026-10-08".into(),index:"C:/advanced/2026-10-08/index.ndjson".into()};
        let fixed=execution_config(config.clone(),"fundamental37_h20").unwrap();
        assert!(fixed.snapshot.replace('\\',"/").ends_with("research/ashare-open-2026-10-01/stockdb-live/exports"));
        assert!(fixed.index.replace('\\',"/").ends_with("research/ashare-open-2026-10-01/online/index-sh-000300.ndjson"));
        let normal=execution_config(config.clone(),"breadth22_h20").unwrap();assert_eq!(normal.snapshot,config.snapshot);assert_eq!(normal.index,config.index);
        assert_eq!(config.snapshot,"C:/advanced/2026-10-08");assert_eq!(fixed.python,config.python);
    }
    #[tokio::test]
    #[ignore="actual frozen financial program to isolated SQLite; requires numpy/model environment"]
    async fn real_financial_replay_to_independent_account(){
        let root=std::env::temp_dir().join(format!("real-financial-replay-{}",uuid::Uuid::new_v4()));let db=Database::open(root.clone()).unwrap();
        let mut advanced=model_config(&db).unwrap();advanced.snapshot=root.join("advanced-forward-snapshot").to_string_lossy().into_owned();advanced.index=root.join("advanced-forward-index.ndjson").to_string_lossy().into_owned();
        let shared=serde_json::to_string(&advanced).unwrap();db.set_setting("model_runner_config",&shared).unwrap();
        assert!(run_model(&db,"fundamental37_h20".into(),20,"baseline".into(),"forward".into(),None).await.is_err());
        let id=run_model(&db,"fundamental37_h20".into(),20,"baseline".into(),"replay".into(),None).await.unwrap();
        let actual=db.model_run(id).unwrap();assert_eq!(actual["ledger"]["metrics"]["completed_holding_cycles"],689);assert_eq!(actual["ledger"]["metrics"]["net_return_pct"],57.551);
        assert_eq!(db.get_setting("model_runner_config").unwrap().as_deref(),Some(shared.as_str()));
        assert_eq!(actual["signal_watch"].as_array().unwrap().len(),4021);assert!(db.enable_model_observation(id,true).is_err());
        let offered=check_bound_run(include_str!("../../../research/ashare-fundamental-2026-10-01/adapter-checks/fundamental37-continuous.json")).unwrap();
        assert_eq!(actual["ledger"],offered["ledger"]);assert_eq!(actual["current_scores"],offered["current_scores"]);
        assert_eq!(import_model_raw(&db,include_str!("../../../research/ashare-fundamental-2026-10-01/adapter-checks/fundamental37-continuous.json")).await.unwrap(),id);
        assert_eq!(db.get_setting("model_runner_config").unwrap().as_deref(),Some(shared.as_str()));
        let out=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("research/ashare-fundamental-2026-10-01/adapter-checks").join(format!("verified-sqlite-{}.json",uuid::Uuid::new_v4()));std::fs::write(&out,db.model_run_bundle(id).unwrap()).unwrap();println!("financial program → SQLite verified: {}",out.display());
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn real_past_chain_records_one_notice_per_new_completed_day(){
        let fixture:Value=serde_json::from_str(include_str!("../../../research/forward-chain-2026-10-01/real-history-v3/actual-notification-fixtures.json")).unwrap();
        let rows=fixture["rows"].as_array().unwrap();assert_eq!(rows.len(),3);let mut notices=vec![];
        let root=std::env::temp_dir().join(format!("historical-model-notice-{}",uuid::Uuid::new_v4()));let db=Database::open(root.clone()).unwrap();
        for pair in rows.windows(2){
            let notice=model_observation(&pair[0],&pair[1],"2026-09-30").unwrap();
            assert_eq!(notice["historical_catchup"],pair[1]["as_of"]!="2026-09-30");
            assert!(model_observation(&pair[1],&pair[1],"2026-09-30").is_none());
            let payload=json!({"id":format!("test-{}",notices.len()),"signal_kind":"research","title":"真实历史补齐验收","body":"非当前实盘提醒","received_at":chrono::Utc::now().timestamp_millis()+notices.len() as i64,"model_snapshot":notice});
            db.archive_brief_alert(&payload).unwrap();notices.push(payload);
        }
        assert_eq!(notices.len(),2);assert!(rows.iter().any(|r|r["ledger"]["orders"].as_array().unwrap().iter().any(|o|o["side"]=="sell")));
        let archived=db.brief_alerts_between("2026-01-01","2027-01-01").unwrap();assert_eq!(archived.len(),2);assert_eq!(archived[0]["model_snapshot"]["as_of"],"2026-09-30");
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore = "requires logged-in Claude Code and existing debug launcher; no market network"]
    async fn claude_proposal_registers_once_with_hard_exclusions() {
        let root = std::env::temp_dir().join(format!("bull-research-smoke-{}", std::process::id()));
        let db = Database::open(root.clone()).unwrap();
        let value=crate::agent::discover_strategy(&db,serde_json::json!({"market":{"advancers":2000,"decliners":2000},"source":"synthetic smoke fixture","existing":[]})).await.unwrap();
        let id = register_proposal(&db, value.clone(), &ResearchConfig::default(), "test").unwrap();
        let rows = db.research_experiments().unwrap();
        let e = rows.iter().find(|e| e.id == id).unwrap();
        assert!(
            e.filter.exclude_st
                && e.filter.exclude_delisting
                && e.filter.exclude_suspended
                && e.filter.exclude_limit_locked
        );
        assert_eq!(e.filter.boards, vec![Board::ShMain, Board::SzMain]);
        assert!(e.account_id.is_none());
        assert!(register_proposal(&db, value, &ResearchConfig::default(), "test").is_err());
        assert!(!db.research_config().unwrap().auto_research);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn proposal_rejects_unknown_rules_and_inverted_ranges() {
        let valid = serde_json::json!({"name":"趋势候选","hypothesis":"基于主板流动性与趋势回踩构造假设，破位或量能不足即失效，尚未验证盈利能力。","rule":"trend_follow","price_min":3.0,"price_max":100.0,"turnover_min":1.0,"turnover_max":15.0,"amount_min_wan":5000.0});
        let mut p: Proposal = serde_json::from_value(valid.clone()).unwrap();
        assert!(p.validate().is_ok());
        p.price_min = 101.0;
        assert!(p.validate().is_err());
        p.rule = "execute_python".into();
        assert!(p.validate().is_err());
        let mut bad = valid;
        bad["code"] = serde_json::json!("arbitrary code");
        assert!(serde_json::from_value::<Proposal>(bad).is_err());
    }
    #[test]
    fn trusted_runner_rejects_modified_code_before_spawn(){
        let root=std::env::temp_dir().join(format!("trusted-model-{}",uuid::Uuid::new_v4()));let dir=root.join("research/research-center-runner");std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model_runner.py"),include_bytes!("../../../research/research-center-runner/model_runner.py")).unwrap();std::fs::write(dir.join("registry.json"),include_bytes!("../../../research/research-center-runner/registry.json")).unwrap();
        let c=ModelRunnerConfig{research_root:root.to_string_lossy().into_owned(),python:"will-not-run".into(),snapshot:"unused".into(),index:"unused".into()};assert!(trusted_runner(&c).is_ok());std::fs::write(dir.join("model_runner.py"),"raise RuntimeError('untrusted')").unwrap();assert!(trusted_runner(&c).is_err());std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore="real StockDB 2026-09-30 fixture + frozen models, no AI/broker; ~100s, not a future-date automatic test"]
    async fn real_frozen_model_process_to_independent_sqlite(){
        let root=std::env::temp_dir().join(format!("real-model-lab-{}",uuid::Uuid::new_v4()));let db=Database::open(root.clone()).unwrap();
        let id=run_model(&db,"breadth22_rank20".into(),20,"baseline".into(),"forward".into(),None).await.unwrap();
        let runs=db.model_runs().unwrap();let run=runs.as_array().unwrap().iter().find(|v|v["id"]==id).unwrap();assert_eq!(run["state"],"waiting_new_data");assert_eq!(run["as_of"],"2026-09-30");assert_eq!(run["ledger"]["metrics"]["completed_holding_cycles"],0);assert_eq!(run["ledger"]["curve"][0]["cash"],100000.);assert_eq!(run["signal_watch"].as_array().unwrap().len(),0);assert_eq!(run["enabled"],false);db.enable_model_observation(id,true).unwrap();db.enable_model_observation(id,false).unwrap();
        assert_eq!(id,run_model(&db,"breadth22_rank20".into(),20,"baseline".into(),"forward".into(),Some(id)).await.unwrap());
        let forward_raw=db.model_run_bundle(id).unwrap();let mut bad=check_bound_run(&forward_raw).unwrap();bad.as_object_mut().unwrap().remove("content_sha256");bad["signal_threshold"]=json!(0.0);let text=bad.to_string();use sha2::{Digest,Sha256};let forged=json!({"schema":"model-run-bundle-v1","content_sha256":hex::encode(Sha256::digest(text.as_bytes())),"content":text}).to_string();assert!(check_bound_run(&forged).is_err());
        let id=run_model(&db,"breadth22_h20".into(),20,"staged".into(),"replay".into(),None).await.unwrap();
        let runs=db.model_runs().unwrap();let run=runs.as_array().unwrap().iter().find(|v|v["id"]==id).unwrap();assert!(run["ledger"]["orders"].as_array().unwrap().iter().any(|v|v["reason"]=="strength_confirm_add"));assert!(run["ledger"]["orders"].as_array().unwrap().iter().any(|v|v["reason"]=="weakness_reduce"));assert_eq!(run["checks"]["ST_BJ_buys"],0);
        let actual=db.model_run_bundle(id).unwrap();assert_eq!(id,import_model_raw(&db,&actual).await.unwrap());
        let mut tampered=check_bound_run(&actual).unwrap();tampered.as_object_mut().unwrap().remove("content_sha256");tampered["ledger"]["orders"]=json!([]);let text=tampered.to_string();let forged=json!({"schema":"model-run-bundle-v1","content_sha256":hex::encode(Sha256::digest(text.as_bytes())),"content":text}).to_string();assert!(import_model_raw(&db,&forged).await.unwrap_err().contains("ledger"));
        let checks=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("research/research-center-runner/checks");std::fs::create_dir_all(&checks).unwrap();let key=uuid::Uuid::new_v4();std::fs::write(checks.join(format!("verified-rank-forward-{key}.json")),forward_raw).unwrap();std::fs::write(checks.join(format!("verified-staged-replay-{key}.json")),&actual).unwrap();
        println!("real model research: rankforward100k/0cycles/0watch, continuationprefix/idempotent, defaultobservefalse, stagedactualorders, externalimporttrustedreplay, deletedorders/thresholdtamperrejected; staged metrics {}; checks {}",run["ledger"]["metrics"],checks.display());drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}

#[derive(Serialize)]
pub struct ResearchDashboard {
    pub config: ResearchConfig,
    pub experiments: Vec<ExperimentView>,
    pub local_ready: bool,
    pub agent_installed: bool,
    pub last_auto_message: String,
    pub busy: bool,
}

#[derive(Serialize)]
pub struct ModeResearchReport {
    pub symbol: String,
    pub source: String,
    pub stale: bool,
    pub signals: Vec<crate::quant::modes::ModeSignal>,
}

#[tauri::command]
pub async fn research_mode_report(
    db: State<'_, Arc<Database>>,
    symbol: String,
) -> Result<ModeResearchReport, String> {
    if db.get_setting("local_history_enabled").ok().flatten().as_deref() != Some("1") {
        return Err("请先在设置中启用本地历史数据".into());
    }
    let url = db.get_setting("local_history_url").map_err(|e| e.to_string())?
        .unwrap_or_else(|| "http://127.0.0.1:7899".into());
    let config = crate::datasource::history::LocalHistoryConfig::new(url);
    let daily = crate::datasource::history::fetch_daily(&config, &symbol, None, None).await?;
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .format("%Y-%m-%d").to_string();
    let complete: Vec<_> = daily.klines.into_iter().filter(|bar| bar.date < today).collect();
    let day = complete.last().ok_or("本地数据库缺少已完成日 K")?.date.clone();
    let stale = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
        .and_then(|now| chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
            .map(|last| (now - last).num_days() > 7))
        .map_err(|e| e.to_string())?;
    let minute = crate::datasource::history::fetch_minute_day(&config, &symbol, &day).await?;
    Ok(ModeResearchReport {
        symbol,
        source: "stockdb 本地日 K + 历史 1 分钟 K".into(),
        stale,
        signals: vec![
            crate::quant::modes::swing(&complete),
            crate::quant::modes::intraday(&minute, &day),
            crate::quant::modes::long_term(&complete),
        ],
    })
}
#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    name: String,
    hypothesis: String,
    rule: String,
    price_min: f64,
    price_max: f64,
    turnover_min: f64,
    turnover_max: f64,
    amount_min_wan: f64,
}
#[cfg(test)]
impl Proposal {
    fn validate(&self) -> Result<(), String> {
        crate::quant::playbook::TradeRule::try_from_id(&self.rule)?;
        if self.name.trim().is_empty()
            || self.name.chars().count() > 40
            || !(20..=800).contains(&self.hypothesis.chars().count())
            || [
                self.price_min,
                self.price_max,
                self.turnover_min,
                self.turnover_max,
                self.amount_min_wan,
            ]
            .iter()
            .any(|v| !v.is_finite())
            || self.price_min < 1.0
            || self.price_max > 1000.0
            || self.price_min > self.price_max
            || self.turnover_min < 0.0
            || self.turnover_max > 30.0
            || self.turnover_min > self.turnover_max
            || !(1000.0..=1_000_000.0).contains(&self.amount_min_wan)
        {
            return Err("AI 候选未通过名称、逻辑、规则或参数校验".into());
        }
        Ok(())
    }
}

#[tauri::command]
pub fn research_dashboard(db: State<'_, Arc<Database>>) -> Result<ResearchDashboard, String> {
    dashboard(&db)
}
pub fn dashboard(db: &Database) -> Result<ResearchDashboard, String> {
    Ok(ResearchDashboard {
        config: db.research_config()?,
        experiments: db.experiment_views()?,
        local_ready: db
            .get_setting("local_history_enabled")
            .ok()
            .flatten()
            .as_deref()
            == Some("1"),
        agent_installed: crate::agent::status(db).installed,
        last_auto_message: db
            .get_setting("research_auto_message")
            .ok()
            .flatten()
            .unwrap_or_default(),
        busy: GATE.get().is_some_and(|g| g.available_permits() == 0),
    })
}
#[tauri::command]
pub fn save_research_config(
    db: State<'_, Arc<Database>>,
    config: ResearchConfig,
) -> Result<(), String> {
    config.validate()?;
    db.set_setting(
        "research_loop_config",
        &serde_json::to_string(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn research_discover(db: State<'_, Arc<Database>>) -> Result<i64, String> {
    discover(&db).await
}
pub async fn discover(db: &Database) -> Result<i64, String> {
    let _permit = GATE.get_or_init(|| tokio::sync::Semaphore::new(1)).try_acquire().map_err(|_| "研究任务正在运行，请稍后")?;
    let source:Value=serde_json::from_str(include_str!("../../../research/research-center-runner/registry.json")).map_err(|e|e.to_string())?;
    let evidence:Value=serde_json::from_str(include_str!("../../research-evidence.json")).map_err(|e|e.to_string())?;
    let longitudinal:Value=serde_json::from_str(include_str!("../../../research/ashare-mainline-2026-10-01/longitudinal-v2/decision.json")).map_err(|e|e.to_string())?;
    let tasks=db.model_tasks()?;
    let extension=super::research_extensions::research_extension_report()?;
    let context=json!({"frozen_registry":source,"multiyear_evidence_bundle":evidence,"longitudinal_ablation":longitudinal,"extension_evidence":extension,"existing_tasks":tasks,"existing_model_runs":db.model_runs()?.as_array().unwrap().iter().map(|v|json!({"id":v["id"],"model_id":v["model_id"],"holding_days":v["holding_days"],"comparison":v["comparison"],"mode":v["mode"],"as_of":v["as_of"],"metrics":v["ledger"]["metrics"]})).collect::<Vec<_>>()});
    // Fixed finite mechanism set; new learning algorithms stay in the research workspace.
    let current:std::collections::HashSet<String>=tasks.as_array().unwrap().iter().filter(|t|t["frozen_source"]==source["files"]).map(|t|format!("{}:{}:{}",t["model_id"],t["holding_days"],t["comparison"])).collect();
    if current.len()>=25 {return Err("当前源五个冻结模型的五个固定对照任务书已登记；直接运行真实账本，新算法请走研究工作目录".into());}
    let mut value=crate::agent::discover_model_research(db,context).await?;
    value["frozen_source"]=source["files"].clone();
    db.register_model_task(&value)
}

#[cfg(test)]
fn register_proposal(
    db: &Database,
    value: serde_json::Value,
    config: &ResearchConfig,
    source: &str,
) -> Result<i64, String> {
    let existing = db.research_experiments()?;
    if existing.len() >= 100 {
        return Err("实验记录已达到 100 条，本版暂停新增，保留现有实验与统计".into());
    }
    if existing
        .iter()
        .filter(|e| {
            matches!(
                e.state.as_str(),
                "candidate" | "observing" | "extended" | "adopted"
            )
        })
        .count()
        >= config.max_active
    {
        return Err("已达到候选及验证数量上限".into());
    }
    let proposal: Proposal =
        serde_json::from_value(value).map_err(|_| "AI 候选字段不完整或有未知字段")?;
    proposal.validate()?;
    let filter = MarketFilter {
        boards: vec![Board::ShMain, Board::SzMain],
        price_min: Some(proposal.price_min),
        price_max: Some(proposal.price_max),
        turnover_min: Some(proposal.turnover_min),
        turnover_max: Some(proposal.turnover_max),
        amount_min_wan: Some(proposal.amount_min_wan),
        ..Default::default()
    };
    let definition =
        serde_json::to_string(&serde_json::json!({"rule":proposal.rule,"filter":filter}))
            .map_err(|e| e.to_string())?;
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(definition.as_bytes()));
    if existing.iter().any(|e| {
        e.rule == proposal.rule
            && serde_json::to_string(&e.filter).ok() == serde_json::to_string(&filter).ok()
    }) {
        return Err("AI 提出了已存在的同一规则与参数，本次没有重复登记".into());
    }
    let preset = PresetInfo {
        id: format!("research_{}", &hash[..20]),
        label: proposal.name,
        description: proposal.hypothesis,
        filter,
        rule: proposal.rule,
        builtin: false,
        strategy_version_id: None,
        strategy_version: 0,
        strategy_status: String::new(),
    };
    let id = db.register_experiment(&preset, config, source)?;
    Ok(id)
}

#[tauri::command]
pub async fn research_start(
    db: State<'_, Arc<Database>>,
    experiment_id: i64,
) -> Result<(), String> {
    start(&db, experiment_id).await
}
pub async fn start(db: &Database, id: i64) -> Result<(), String> {
    db.require_current_research_version(id)?;
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "研究任务正在运行，请稍后")?;
    let experiment = db
        .research_experiments()?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or("实验不存在")?;
    if experiment.account_id.is_some() || experiment.state != "candidate" {
        return Err("该实验已有冻结账户，不能重复启动".into());
    }
    let card = db
        .strategy_library()?
        .cards
        .into_iter()
        .find(|c| c.version_id == experiment.version_id)
        .ok_or("冻结策略版本不存在")?;
    for stage in ["static", "causal"] {
        if !card
            .stages
            .iter()
            .any(|s| s.stage == stage && s.status == "passed")
        {
            return Err(format!("前置检查 {stage} 未通过，不能启动研究实验"));
        }
    }
    if db
        .get_setting("local_history_enabled")
        .ok()
        .flatten()
        .as_deref()
        != Some("1")
    {
        return Err("先在设置中启用本地历史数据并更新未复权日线，再启动模拟验证".into());
    }
    let snapshot = market::market_snapshot_with_fallback(std::time::Duration::from_secs(60), None)
        .await
        .map_err(|e| e.to_string())?;
    if snapshot.stale {
        return Err("市场快照陈旧，暂停选股".into());
    }
    let caps = FilterCapabilities::for_source(snapshot.source);
    let skipped = experiment.filter.skipped_conditions(caps);
    if !skipped.is_empty() {
        return Err(format!("当前数据源缺少策略字段：{}", skipped.join("、")));
    }
    let mut rows = experiment.filter.apply_with(&snapshot.rows, caps);
    rows.sort_by(|a, b| {
        b.amount
            .total_cmp(&a.amount)
            .then_with(|| a.code.cmp(&b.code))
    });
    rows.truncate(experiment.config.stock_count);
    if rows.is_empty() {
        return Err("当前无符合冻结条件的标的，候选保留，等待下次市场快照".into());
    }
    let targets: Vec<_> = rows
        .iter()
        .map(|r| Target {
            id: 0,
            account_id: 0,
            symbol: format!(
                "{}{}",
                if r.board == Board::ShMain { "sh" } else { "sz" },
                r.code
            ),
            name: r.name.clone(),
            rule: experiment.rule.clone(),
            limit_bps: 1000,
        })
        .collect();
    let url = db
        .get_setting("local_history_url")
        .ok()
        .flatten()
        .unwrap_or("http://127.0.0.1:7899".into());
    let local = crate::datasource::history::LocalHistoryConfig::new(url);
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .date_naive();
    let mut live_plans = Vec::new();
    for target in &targets {
        let data =
            crate::datasource::history::fetch_daily(&local, &target.symbol, None, None).await?;
        let date = data
            .raw_klines
            .last()
            .and_then(|b| chrono::NaiveDate::parse_from_str(&b.date, "%Y-%m-%d").ok())
            .ok_or("标的缺少未复权日线")?;
        if data.raw_klines.len() < 60
            || data.klines.len() < 60
            || (today - date).num_days() > 7
            || date > today
        {
            return Err(format!(
                "{} 历史样本不足或数据日期不合适，请更新本地历史",
                target.symbol
            ));
        }
        let end = data
            .klines
            .iter()
            .rposition(|b| b.date < today.to_string())
            .ok_or("缺少已完成日线")?;
        let plan = crate::quant::playbook::plan(
            &data.klines[..=end],
            crate::quant::playbook::TradeRule::try_from_id(&target.rule)?,
        )
        .ok_or("无法建立可执行买卖区间")?;
        let factor = data.raw_klines[end].close / data.klines[end].close;
        let price = |v: f64| crate::simulation_live::scaled(v * factor);
        live_plans.push(crate::db::simulation_live::LivePlan {
            symbol: target.symbol.clone(),
            buy_low: price(plan.buy_low)?,
            buy_high: price(plan.buy_high)?,
            stop: price(plan.stop_loss)?,
            take: price(plan.take_profit)?,
            limit_bps: target.limit_bps,
            basis_date: data.klines[end].date.clone(),
            reference_close: price(data.klines[end].close)?,
            position_pct: plan.position_pct,
        });
    }
    let c = &experiment.config;
    let account_input = AccountInput {
        id: None,
        name: format!("研究 {} · {}", id, experiment.name),
        initial_cash: c.initial_cash.clone(),
        mode: "auto".into(),
        auto_enabled: false,
        manual_source_enabled: false,
        rule_source_enabled: true,
        ai_source_enabled: false,
        commission_bps: c.commission_bps,
        min_commission: c.min_commission.clone(),
        stamp_tax_bps: c.stamp_tax_bps,
        transfer_fee_bps: c.transfer_fee_bps,
        slippage_bps: c.slippage_bps,
        targets,
    };
    let account = db.save_sim_account(&account_input)?;
    if c.execution_mode != "daily" {
        if let Err(error) = db.enable_live_account(account.id, &live_plans) {
            let _ = db.delete_sim_account(account.id);
            return Err(error);
        }
    }
    let selection=serde_json::to_string(&serde_json::json!({"retrieved_at":chrono::Utc::now().to_rfc3339(),"source":snapshot.source,"selection":"冻结快照条件筛选后按成交额排序","rows":rows})).map_err(|e|e.to_string())?;
    if let Err(error) = db.link_experiment(id, account.id, &selection) {
        let _ = db.delete_sim_account(account.id);
        return Err(error);
    }
    if c.execution_mode == "parallel" {
        let mut other = account_input;
        other.name = format!("日线对照 {}", id);
        let comparison = match db.save_sim_account(&other) {
            Ok(a) => a,
            Err(error) => {
                db.set_experiment_state(id, "paused", "日线对照创建失败，实时账户一起暂停")?;
                return Err(error);
            }
        };
        if let Err(e) = db.link_comparison(id, comparison.id) {
            let _ = db.delete_sim_account(comparison.id);
            db.set_experiment_state(id, "paused", "对照账户绑定失败，请检查后重试")?;
            return Err(e);
        }
    }
    Ok(())
}

pub fn evaluate(db: &Database, id: i64) -> Result<(), String> {
    let view = db
        .experiment_views()?
        .into_iter()
        .find(|v| v.experiment.id == id)
        .ok_or("实验不存在")?;
    if !matches!(
        view.experiment.state.as_str(),
        "observing" | "extended" | "adopted"
    ) {
        return Ok(());
    }
    let Some(d) = view.detail else { return Ok(()) };
    let (state, message) = assessment(
        &view.experiment.config,
        view.elapsed_days,
        d.metrics.sample_count,
        d.metrics.total_return_bps,
        d.metrics.max_drawdown_bps,
        d.metrics.benchmark_return_bps,
        view.curve.len(),
    );
    let state = if view.experiment.state == "adopted" && state == "qualified" {
        "adopted"
    } else {
        state
    };
    if state != view.experiment.state || message != view.experiment.last_message {
        db.set_experiment_state_if(id, state, &message, Some(&view.experiment.state))?;
    }
    Ok(())
}
#[tauri::command]
pub async fn research_action(
    db: State<'_, Arc<Database>>,
    manager: State<'_, Arc<crate::datasource::DataSourceManager>>,
    experiment_id: i64,
    action: String,
) -> Result<(), String> {
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "研究任务正在运行，请稍后再操作")?;
    let e = db
        .research_experiments()?
        .into_iter()
        .find(|e| e.id == experiment_id)
        .ok_or("实验不存在")?;
    match action.as_str() {
        "pause" if matches!(e.state.as_str(), "observing" | "extended" | "adopted") => {
            db.set_experiment_state(e.id, "paused", "用户暂停；保留净值、持仓及待成交指令")
        }
        "resume" if e.state == "paused" => {
            db.set_experiment_state(e.id, "observing", "用户恢复原版本验证")
        }
        "reject" => db.set_experiment_state(e.id, "rejected", "用户淘汰候选，全部历史保留"),
        "adopt" if e.state == "qualified" => {
            let v = db
                .experiment_views()?
                .into_iter()
                .find(|v| v.experiment.id == e.id)
                .ok_or("实验不存在")?;
            if v.verdict != "qualified" {
                return Err("当前数据不满足考核门槛".into());
            }
            db.set_experiment_state(
                e.id,
                "adopted",
                "用户采纳为研究策略，继续模拟跟踪；未获得实盘准入",
            )
        }
        "run" => {
            db.require_current_research_version(e.id)?;
            let account = e.account_id.ok_or("请先启动验证")?;
            let result = if db.live_account(account)? {
                super::simulation_live::run(&db, &manager, account).await
            } else {
                super::simulation::run_account(&db, account).await
            };
            if let Err(error) = result {
                account_failed(&db, account, &error);
                return Err(error);
            }
            if let Some(comparison) = db.comparison_account(e.id)? {
                super::simulation::run_account(&db, comparison).await?;
            }
            evaluate(&db, e.id)
        }
        _ => Err("当前状态不允许该操作".into()),
    }
}

pub async fn scheduled_tick(db: &Database) {
    if GATE.get().is_some_and(|g|g.available_permits()==0){return;}
    let Ok(config)=db.research_config() else{return;};
    if !config.auto_research || db.get_setting("ai_enabled").ok().flatten().as_deref()==Some("0"){return;}
    let local=chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
    use chrono::Timelike;
    if !(16..22).contains(&local.hour()){return;}
    let Ok(day)=super::mainline::completed_day(chrono::Utc::now()) else{return;};
    if day!=local.date_naive(){return;}
    let day=day.to_string();
    if db.get_setting("research_auto_day").ok().flatten().as_deref()==Some(day.as_str()){return;}
    let _=db.set_setting("research_auto_day",&day);
    let result=async {
        let tasks=db.model_tasks()?;
        let registry:Value=serde_json::from_str(include_str!("../../../research/research-center-runner/registry.json")).map_err(|e|e.to_string())?;
        let input=Value::Object(registry["files"].as_object().ok_or("受信来源不完整")?.iter().map(|(key,row)|(key.clone(),row["sha256"].clone())).collect());
        use sha2::{Digest,Sha256};let runner_sha=hex::encode(Sha256::digest(include_bytes!("../../../research/research-center-runner/model_runner.py")));
        let mut pending=None;
        for task in tasks.as_array().unwrap().iter().filter(|task|task["frozen_source"]==registry["files"]){if !db.has_model_replay(task,&input,&runner_sha)?{pending=task["id"].as_i64();break;}}
        let id=if let Some(id)=pending{id}else{discover(db).await?};
        let tasks=db.model_tasks()?;let task=tasks.as_array().unwrap().iter().find(|v|v["id"]==id).ok_or("研究任务不存在")?;
        if db.has_model_replay(task,&input,&runner_sha)?{return Ok(format!("{day} 任务 #{id} 的同冻结模型/期限/成本对照已经完成；机制去重，不重复回放"));}
        let _permit=GATE.get_or_init(||tokio::sync::Semaphore::new(1)).try_acquire().map_err(|_|"另一研究任务正在运行，待下一轮")?;
        let run=run_model(db,task["model_id"].as_str().unwrap().into(),task["holding_days"].as_i64().unwrap(),task["comparison"].as_str().unwrap().into(),"replay".into(),None).await?;
        Ok::<String,String>(format!("{day} 研究任务 #{id} 已用受信模型实际跑完历史对照，账本 #{run}；非准入、不执行AI生成脚本"))
    }.await;
    let message=match result {Ok(v)=>v,Err(e)=>{let _=db.set_setting("model_research_last_error",&e);format!("{day} 项目内研究未完成：{e}；当日不自动重复调用，错误保留，可手动模型运行")}};
    let _=db.set_setting("research_auto_message",&message);
}

pub fn account_failed(db: &Database, account: i64, error: &str) {
    if error.contains("正在运行") {
        return;
    }
    if let Ok(items) = db.research_experiments() {
        for e in items {
            if db.comparison_account(e.id).ok().flatten() == Some(account) {
                let _ = db.set_experiment_state_if(
                    e.id,
                    "paused",
                    &format!("日线对照失败，整组暂停：{error}"),
                    Some(&e.state),
                );
                return;
            }
        }
    }
    if let Ok(items) = db.research_experiments() {
        if let Some(e) = items.into_iter().find(|e| {
            e.account_id == Some(account)
                && matches!(e.state.as_str(), "observing" | "extended" | "adopted")
        }) {
            let _ =
                db.set_experiment_state(e.id, "paused", &format!("模拟运行失败，已暂停：{error}"));
        }
    }
}

fn workspace() -> Result<std::path::PathBuf, String> {
    let root = dirs::data_local_dir()
        .ok_or("无法定位用户数据目录")?
        .join("bull-arrives")
        .join("research-workspace");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root)
}

#[derive(Serialize)]
pub struct WorkspaceInfo {
    pub path: String,
    pub skills: Vec<String>,
    pub prompt: String,
}
fn research_brief() -> String {
    include_str!("../../../research/研究中心任务书.md").into()
}
#[tauri::command]
pub fn research_workspace(db: State<'_, Arc<Database>>) -> Result<WorkspaceInfo, String> {
    prepare_workspace(&db, None)
}

fn prepare_workspace(db: &Database, topic: Option<&str>) -> Result<WorkspaceInfo, String> {
    let catalog: serde_json::Value = serde_json::from_str(include_str!("../../research-catalog.json"))
        .map_err(|e| e.to_string())?;
    let chosen = topic.map(|id| {
        catalog["strategies"].as_array().and_then(|rows| rows.iter().find(|row| row["id"] == id))
            .ok_or("未知研究方向")
    }).transpose()?;
    let engine = db.get_setting("local_history_engine_path").ok().flatten();
    let engine_dir = engine.as_deref().and_then(|path| std::path::Path::new(path).parent());
    let endpoint = db.get_setting("local_history_url").ok().flatten()
        .unwrap_or_else(|| "http://127.0.0.1:7899".into());
    let prompt = format!("{}\n\n## 本轮 A 股研究资料\n\n读取当前目录 RESEARCH_CATALOG.json，其中含一手来源、研究假设与缺失数据。选择方向：{}。\n\nstockdb 历史地址：{endpoint}；发行目录：{}。先读取发行目录下 `调用方式/python/AI策略python开发接口文档.md`，缺失时读取资料库中的官网文档；网站文档只作接口参考，Bull Arrives 保留自身 SQLite 账本，不遵循外部文档要求改存储。stockdb 仅提供历史；当前报价与盘口由应用既有通道提供。\n\n## 可复核成果\n\n1. 先核对原始论文的市场、样本期、策略定义与成本，多空学术组合不能冒充 A 股多头可实现收益。\n2. 在研究目录写 `research_report.md`：列出买入/退出原因、证据来源、数据截止日、冻结参数、训练与样本外分段、费用、基准、交易数、失败情况、无法取得的组合回撤。未经实际执行不能填收益。\n3. 需要 Python 检验时，先生成可读的 `study.py` 并解释执行内容，在 Claude 默认交互权限下运行；只读本机历史，不调用公共远端 StockDB 测试节点，不写 data/mydb 或模拟账本。保留执行命令和结果，用户可复现。\n4. 财报用公告可用时间，资讯用首次观测时间；用价格调整总收益或明确报告公司行动遗漏；避免未来函数、存续偏差与重叠交易伪组合。\n5. 任意新算法只保存研究报告与脚本，不能把价值/残差反转/事件策略硬改名为现有 trend_follow 或 mean_reversion。输出 taskbook.json 只登记受限冻结模型假说，不执行AI代码。\n", research_brief(),
        chosen.map(|row| row["name"].as_str().unwrap_or("")).unwrap_or("从资料库选择"),
        engine_dir.map(|path| path.to_string_lossy().into_owned()).unwrap_or_else(|| "尚未配置，请用户定位历史 SDK".into()));
    let root = workspace()?;
    std::fs::write(root.join("RESEARCH_CATALOG.json"), include_str!("../../research-catalog.json"))
        .map_err(|e| e.to_string())?;
    let brief = root.join("BULL_RESEARCH.md");
    std::fs::write(&brief, &prompt).map_err(|e| e.to_string())?;
    let mut skills = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if let Ok(entries) = std::fs::read_dir(home.join(".claude/skills")) {
            for entry in entries.flatten() {
                if entry.path().join("SKILL.md").is_file() {
                    skills.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    skills.sort();
    Ok(WorkspaceInfo {
        path: root.to_string_lossy().into_owned(),
        skills,
        prompt,
    })
}
#[tauri::command]
pub fn open_research_claude(db: State<'_, Arc<Database>>, topic: Option<String>) -> Result<WorkspaceInfo, String> {
    let info = prepare_workspace(&db, topic.as_deref())?;
    let status = crate::agent::status(&db);
    let path = status
        .path
        .filter(|_| status.installed)
        .ok_or(status.message)?;
    #[cfg(windows)]
    {
        // Windows Terminal owns the interactive console; a GUI parent has no usable stdin.
        let terminal = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("Microsoft/WindowsApps/wt.exe"))
            .filter(|p| p.is_file())
            .ok_or_else(|| {
                format!(
                    "未检测到 Windows Terminal，请在终端进入 {} 后运行 claude",
                    info.path
                )
            })?;
        let mut command = std::process::Command::new(terminal);
        command.args(["-w","new","new-tab","--title","Bull Arrives 研究","-d"]).arg(&info.path).arg(path).args(["--permission-mode","default","请先读取当前目录 BULL_RESEARCH.md 和 RESEARCH_CATALOG.json，读取已有研究与失败结果，去重并在独立目录预登记新的A股假设，真实运行多股多年研究；用户已授权开始，不必再次等待“开始”。保持原源和账本只读，新机制输出报告/SHA供审核，不伪装成既有冻结模型。"]);
        command
            .spawn()
            .map_err(|e| format!("无法打开 Claude 交互窗口：{e}"))?;
        Ok(info)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(format!(
            "请在终端进入 {} 后运行 claude；当前自动开窗仅支持 Windows",
            info.path
        ))
    }
}
#[cfg(test)]
mod catalog_tests {
    #[test]
    fn catalog_has_unique_topics_and_primary_links() {
        let value: serde_json::Value = serde_json::from_str(include_str!("../../research-catalog.json")).unwrap();
        let topics = value["strategies"].as_array().unwrap();
        let mut ids = std::collections::HashSet::new();
        for item in topics {
            let id = item["id"].as_str().unwrap();
            assert!(ids.insert(id));
            assert!(item["url"].as_str().unwrap().starts_with("https://"));
            assert!(!item["limitation"].as_str().unwrap().is_empty());
        }
    }
}
#[tauri::command]
pub fn import_research_candidate(db: State<'_, Arc<Database>>) -> Result<i64, String> {
    let _permit = GATE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .try_acquire()
        .map_err(|_| "研究任务正在运行")?;
    let file = workspace()?.join("taskbook.json");
    let metadata = std::fs::symlink_metadata(&file)
        .map_err(|_| "尚无 taskbook.json，请让 Claude 完成模型假说")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16384 {
        return Err("候选必须是 16 KiB 以内的普通 JSON 文件".into());
    }
    let value = serde_json::from_str(&std::fs::read_to_string(file).map_err(|e| e.to_string())?)
        .map_err(|_| "候选 JSON 格式错误")?;
    let mut value:Value=value;
    let registry:Value=serde_json::from_str(include_str!("../../../research/research-center-runner/registry.json")).map_err(|e|e.to_string())?;
    value["frozen_source"]=registry["files"].clone();
    db.register_model_task(&value)
}
