//! 完整成分股的当前趋势观察。研究假说，不替代多年组合模型的准入。
use crate::{datasource::{history, sector, trading_calendar, DataSource}, db::Database, domain::KLineData};
use chrono::{Duration, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock, atomic::{AtomicU64, Ordering}};
use tauri::State;

static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
static DISCOVERY_GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
static DISCOVERY_CONTROL: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
static DISCOVERY_WAKE: OnceLock<tokio::sync::Notify> = OnceLock::new();
static DISCOVERY_REVISION: AtomicU64 = AtomicU64::new(0);
const DISCOVERY_SCHEMA: &str = "mainline-discovery-v2";
static WATCH_GATE: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
const DISCOVERY_STATE: &str = "mainline_discovery_state";
const WATCH_KEY: &str = "mainline_watchlist";
const SNAPSHOT_ARCHIVE: &str = "mainline_evidence_archive";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watch { pub kind: String, pub code: String, pub name: String }

fn watches(db: &Database) -> Result<Vec<Watch>, String> {
    serde_json::from_str(&db.get_setting(WATCH_KEY).map_err(|e| e.to_string())?.unwrap_or_else(|| "[]".into()))
        .map_err(|_| "主线关注记录损坏".into())
}
fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.starts_with("BK") && code[2..].bytes().all(|c| c.is_ascii_digit())
}
fn validate(kind: &str, code: &str, name: &str) -> Result<sector::SectorKind, String> {
    let kind = sector::SectorKind::parse(kind)?;
    if !(valid_code(code)||valid_sw_code(code)) || name.trim().is_empty() || name.chars().count() > 30 { return Err("板块名称或代码无效".into()); }
    if excluded_sector_name(name) {return Err("ST、北交所及动态资格集合不作为研究主线".into());}
    Ok(kind)
}
fn excluded_sector_name(name:&str)->bool {
    let upper=name.to_uppercase();
    upper.contains("ST") || ["北交","北证","退市","昨日","连板","首板","涨停","跌停","融资融券","沪股通","深股通","机构重仓","基金重仓"].iter().any(|k|name.contains(k))
}
async fn benchmark(day:&str)->Result<Vec<KLineData>,String>{
    // 显式指数代码，绝不把股票000300或000001当沪深300。
    let rows=crate::datasource::tencent::TencentAdapter::new().fetch_kline("sh000300","CN","daily",Some(day),Some(100)).await.map_err(|e|e.to_string())?;
    let rows:Vec<_>=rows.into_iter().filter(|b|b.date.as_str()<=day).collect();
    if rows.last().map(|b|b.date.as_str())!=Some(day)||rows.len()<66{return Err("真实沪深300指数日线缺失或截止日不一致".into());}
    Ok(rows)
}
fn aligned_market_window(bars:&[KLineData],index:&[KLineData])->Result<(Vec<KLineData>,usize),String> {
    if index.len()<66 || index.windows(2).any(|r|r[0].date>=r[1].date) {return Err("真实指数历史不足、重复或倒序".into());}
    if bars.windows(2).any(|r|r[0].date>=r[1].date) {return Err("行情包含重复或倒序日期".into());}
    let base=&index[index.len()-66..];
    let first=base.first().unwrap().date.as_str();let last=base.last().unwrap().date.as_str();
    let mut removed=0;
    for row in bars.iter().filter(|r|r.date.as_str()>=first && r.date.as_str()<=last) {
        let date=NaiveDate::parse_from_str(&row.date,"%Y-%m-%d").map_err(|_|"行情日期无效")?;
        if base.binary_search_by(|r|r.date.cmp(&row.date)).is_err() {
            if trading_calendar::is_trading_day_at(Utc::now(),date)? {
                return Err(format!("指数缺少真实交易日{}，停止比较强度",row.date));
            }
            removed+=1;
        }
    }
    let aligned=base.iter().map(|day| {
        bars.binary_search_by(|row|row.date.cmp(&day.date)).map(|i|bars[i].clone())
            .map_err(|_|format!("缺少真实交易日{}的行情，不能比较强度",day.date))
    }).collect::<Result<Vec<_>,_>>()?;
    Ok((aligned,removed))
}
fn relative_metrics(bars:&[KLineData],index:&[KLineData])->Result<Value,String>{
    // Align actual sessions; discard only dates confirmed closed by the shared market calendar.
    // Never intersect away a missing trading session or fill an absent price.
    let (aligned,removed)=aligned_market_window(bars,index)?;
    let mut m=metrics(&aligned).ok_or("行情窗口不足或无效")?;let bm=metrics(index).ok_or("指数行情窗口无效")?;
    m["rs20_vs_hs300"]=json!(number(&m,"r20")-number(&bm,"r20"));m["rs60_vs_hs300"]=json!(number(&m,"r60")-number(&bm,"r60"));
    m["strong"]=json!(m["strong"]==true && number(&m,"rs20_vs_hs300")>0.0 && number(&m,"rs60_vs_hs300")>0.0);
    m["removed_non_trading_rows"]=json!(removed);m["window_sessions"]=json!(66);
    m["window_start"]=json!(aligned[0].date);m["window_end"]=json!(aligned.last().unwrap().date);
    Ok(m)
}
fn member_metrics(data:&history::LocalHistoryResult,index:&[KLineData],as_of:&str,historical_st:bool)->Result<Option<Value>,String>{
    if historical_st { match data.st_by_date.iter().find(|(day,_)|day.as_str()==as_of).and_then(|(_,status)|*status){
        Some(true)=>return Ok(None),
        Some(false)=>{},
        None=>return Err("截止交易日ST状态缺失，拒绝猜测".into()),
    }}
    if data.end_date.as_deref()!=Some(as_of){return Err("日线截止日期落后或停牌".into());}
    relative_metrics(&data.klines,index).map(Some)
}

#[tauri::command]
pub fn get_mainline_watchlist(db: State<'_, Arc<Database>>) -> Result<Vec<Watch>, String> { watches(&db) }
#[tauri::command]
pub fn set_mainline_watch(db: State<'_, Arc<Database>>, kind: String, sector_code: String, sector_name: String, enabled: bool) -> Result<Vec<Watch>, String> {
    let _guard=WATCH_GATE.get_or_init(||std::sync::Mutex::new(())).lock().unwrap_or_else(|e|e.into_inner());
    validate(&kind, &sector_code, &sector_name)?;
    let mut rows = watches(&db)?;
    rows.retain(|r| r.code != sector_code);
    if enabled {
        if rows.len() >= 8 { return Err("最多关注八个具体板块，请先取消一项".into()); }
        rows.push(Watch { kind, code: sector_code, name: sector_name });
    }
    db.set_setting(WATCH_KEY, &serde_json::to_string(&rows).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(rows)
}

fn discovery_status(db:&Database)->Result<Value,String> {
    let mut state:Value=serde_json::from_str(&db.get_setting(DISCOVERY_STATE).map_err(|e|e.to_string())?.unwrap_or_else(||"{}".into())).map_err(|_|"主线发现进度损坏")?;
    if state.as_object().is_none(){return Err("主线发现进度结构无效".into());}
    state.as_object_mut().unwrap().remove("queue");
    state.as_object_mut().unwrap().remove("retry_queue");
    state["enabled"]=json!(db.get_setting("mainline_discovery_enabled").ok().flatten().as_deref()==Some("1"));
    if let Some(error)=db.get_setting("mainline_discovery_last_error").ok().flatten(){state["last_error"]=serde_json::from_str(&error).unwrap_or(json!(error));}
    state["busy"]=json!(DISCOVERY_GATE.get_or_init(||tokio::sync::Semaphore::new(1)).available_permits()==0);
    Ok(state)
}
fn valid_sw_code(code:&str)->bool {code.len()==8&&code.starts_with("SW801")&&code[2..].bytes().all(|b|b.is_ascii_digit())}
async fn sector_history(code:&str,index:&[KLineData])->Result<sector::SectorHistory,String>{
    if valid_sw_code(code){crate::datasource::sw_sector::fetch_history(code).await}else{
        if index.len()<66{return Err("真实指数历史不足，无法核验板块日线".into());}
        let required:Vec<_>=index[index.len()-66..].iter().map(|day|day.date.clone()).collect();
        sector::fetch_history_for_sessions(code,&required).await
    }
}
async fn sector_members(kind:sector::SectorKind,code:&str)->Result<sector::SectorMemberPage,String>{
    if valid_sw_code(code){crate::datasource::sw_sector::fetch_members(code).await}else{sector::fetch_research_members(kind,code).await}
}
#[tauri::command]
pub fn get_mainline_discovery_status(db:State<'_,Arc<Database>>)->Result<Value,String>{discovery_status(&db)}
fn discovery_enabled(db:&Database)->bool {db.get_setting("mainline_discovery_enabled").ok().flatten().as_deref()==Some("1")}
fn discovery_wake()->&'static tokio::sync::Notify {DISCOVERY_WAKE.get_or_init(tokio::sync::Notify::new)}
fn prepare_discovery_retry(state:&mut Value)->Result<bool,String> {
    if state["finished"]!=true {return Ok(false);}
    let failed:std::collections::HashSet<String>=state["failed"].as_array().into_iter().flatten()
        .filter_map(|row|row["code"].as_str().map(str::to_string)).collect();
    let retry_catalog=state["catalog_errors"].as_array().is_some_and(|rows|!rows.is_empty());
    if failed.is_empty()&&!retry_catalog {return Ok(false);}
    let queue=state["queue"].as_array().ok_or("发现队列缺失，原候选保留，无法恢复失败板块")?;
    let retry:Vec<Value>=queue.iter().filter(|row|row["code"].as_str().is_some_and(|code|failed.contains(code))).cloned().collect();
    if retry.len()!=failed.len() {return Err("失败板块与已保存目录不一致，保留原结果等待核对".into());}
    state["retry_queue"]=json!(retry);state["retry_processed"]=json!(0);state["retry_total"]=json!(retry.len());
    state["catalog_retry_pending"]=json!(retry_catalog);state["finished"]=json!(false);
    Ok(true)
}
fn apply_discovery_result(state:&mut Value,row:&Value,result:Result<Value,String>)->Result<(),String> {
    let code=row["code"].as_str().ok_or("板块代码缺失")?;
    state["failed"].as_array_mut().ok_or("失败进度无效")?.retain(|previous|previous["code"]!=code);
    state["candidates"].as_array_mut().ok_or("候选进度无效")?.retain(|previous|previous["code"]!=code);
    match result {
        Ok(candidate)=>{if candidate["metrics"]["strong"]==true&&number(&candidate["metrics"],"r20")>0.0 {
            state["candidates"].as_array_mut().unwrap().push(candidate);
        }},
        Err(reason)=>state["failed"].as_array_mut().unwrap().push(json!({"kind":row["kind"],"code":code,"name":row["name"],"reason":reason})),
    }
    Ok(())
}
fn set_discovery_enabled(db:&Database,enabled:bool,restart:bool)->Result<Value,String> {
    let _guard=DISCOVERY_CONTROL.get_or_init(||std::sync::Mutex::new(())).lock().unwrap_or_else(|e|e.into_inner());
    if restart {
        let mut state:Value=serde_json::from_str(&db.get_setting(DISCOVERY_STATE).map_err(|e|e.to_string())?.unwrap_or_else(||"{}".into())).map_err(|_|"主线发现进度损坏")?;
        if prepare_discovery_retry(&mut state)? {
            db.set_setting(DISCOVERY_STATE,&state.to_string()).map_err(|e|e.to_string())?;
        }
        db.set_setting("mainline_discovery_last_error","null").map_err(|e|e.to_string())?;
    }
    db.set_setting("mainline_discovery_enabled",if enabled{"1"}else{"0"}).map_err(|e|e.to_string())?;
    log::info!(target: "automation::mainline", "{}",if enabled{"全市场扫描已开启，继续已有进度"}else{"全市场扫描已暂停，已有进度保留"});
    DISCOVERY_REVISION.fetch_add(1,Ordering::SeqCst);
    discovery_wake().notify_waiters();discovery_wake().notify_one();
    discovery_status(db)
}
#[tauri::command]
pub fn set_mainline_discovery_enabled(db:State<'_,Arc<Database>>,enabled:bool)->Result<Value,String>{set_discovery_enabled(&db,enabled,enabled)}
#[tauri::command]
pub async fn run_mainline_discovery(db:State<'_,Arc<Database>>)->Result<Value,String>{
    // The app's existing worker owns the scan; closing this window never owns/cancels it.
    set_discovery_enabled(&db,true,true)
}
async fn discovery_changed(db:&Database,revision:u64) {
    loop {
        let changed=discovery_wake().notified();tokio::pin!(changed);changed.as_mut().enable();
        if DISCOVERY_REVISION.load(Ordering::SeqCst)!=revision || !discovery_enabled(db) {return;}
        changed.await;
    }
}
pub async fn wait_for_discovery_tick(db:&Database) {
    let continuing=discovery_enabled(db) && db.get_setting(DISCOVERY_STATE).ok().flatten()
        .and_then(|s|serde_json::from_str::<Value>(&s).ok()).map_or(true,|v|v["finished"]!=true);
    tokio::select! { _=discovery_wake().notified()=>{}, _=tokio::time::sleep(std::time::Duration::from_secs(if continuing{2}else{60}))=>{} }
}
fn save_discovery_progress(db:&Database,state:&Value,revision:u64)->Result<bool,String> {
    let _guard=DISCOVERY_CONTROL.get_or_init(||std::sync::Mutex::new(())).lock().unwrap_or_else(|e|e.into_inner());
    if !discovery_enabled(db) || DISCOVERY_REVISION.load(Ordering::SeqCst)!=revision {return Ok(false);}
    db.set_setting(DISCOVERY_STATE,&state.to_string()).map_err(|e|e.to_string())?;
    db.set_setting("mainline_discovery_last_error","null").map_err(|e|e.to_string())?;
    let failed=state["failed"].as_array().map_or(0,Vec::len);
    log::info!(target: "automation::mainline", "行情 {}：已扫描 {}/{} 个板块，候选 {} 个，失败 {} 个{}",
        state["as_of"].as_str().unwrap_or("待核实"),state["processed"].as_u64().unwrap_or(0),state["total"].as_u64().unwrap_or(0),
        state["candidates"].as_array().map_or(0,Vec::len),failed,if state["finished"]==true{"；本轮完成"}else{"；继续后台扫描"});
    Ok(true)
}
fn start_discovery_chunk(db:&Database,revision:u64,chunk:&[Value],day:&str,index:&[KLineData])
    ->Option<tokio::task::JoinSet<(Value,Result<Value,String>)>>{
    let _control=DISCOVERY_CONTROL.get_or_init(||std::sync::Mutex::new(())).lock().unwrap_or_else(|e|e.into_inner());
    if !discovery_enabled(db)||DISCOVERY_REVISION.load(Ordering::SeqCst)!=revision{return None;}
    let mut tasks=tokio::task::JoinSet::new();
    for row in chunk {
        let row=row.clone();let cutoff=day.to_owned();let index=index.to_vec();
        tasks.spawn(async move {
            let code=row["code"].as_str().unwrap_or("");
            let result=tokio::time::timeout(std::time::Duration::from_secs(50),sector_history(code,&index)).await
                .map_err(|_|"板块日线请求超时".to_string()).and_then(|r|r);
            let result=result.and_then(|hist|{
                let bars:Vec<_>=hist.items.iter().filter(|b|b.date<=cutoff).map(|b|KLineData{date:b.date.clone(),open:b.open,high:b.high,low:b.low,close:b.close,volume:0,turnover:0.0}).collect();
                if bars.last().map(|r|r.date.as_str())!=Some(&cutoff){return Err("板块日线截止日期不一致".into());}
                let m=relative_metrics(&bars,&index)?;
                let extension=(number(&m,"close")-number(&m,"ma20"))/number(&m,"atr");
                Ok(json!({"kind":row["kind"],"code":row["code"],"name":row["name"],"as_of":cutoff,"metrics":m,
                    "extension_atr":extension,"entry_ready":extension<=2.0,"observation_state":if extension<=2.0{"ready"}else{"waiting_pullback"},"source":hist.source,"model_status":"unvalidated_sector_observation"}))
            });(row,result)
        });
    }
    Some(tasks)
}
async fn discover_batch(db:&Database)->Result<(),String> {
    let _permit=DISCOVERY_GATE.get_or_init(||tokio::sync::Semaphore::new(1)).try_acquire().map_err(|_|"全市场发现正在扫描")?;
    if !discovery_enabled(db){return Ok(());}
    let revision=DISCOVERY_REVISION.load(Ordering::SeqCst);
    tokio::select! { biased; _=discovery_changed(db,revision)=>Ok(()), result=discover_batch_inner(db,revision)=>result }
}
async fn retry_discovery_catalog(state:&mut Value)->Result<(),String> {
    let (bk,sw)=tokio::join!(sector::fetch_filter_catalog(),crate::datasource::sw_sector::fetch_catalog());
    let mut errors=Vec::new();let mut catalog=Vec::new();
    match bk {Ok(mut rows)=>catalog.append(&mut rows),Err(error)=>errors.push(format!("东方财富目录：{error}"))};
    match sw {Ok(mut rows)=>catalog.append(&mut rows),Err(error)=>errors.push(format!("申万目录：{error}"))};
    let mut known:std::collections::HashSet<String>=state["queue"].as_array().ok_or("发现队列缺失")?.iter()
        .chain(state["excluded_sectors"].as_array().into_iter().flatten()).filter_map(|row|row["code"].as_str().map(str::to_string)).collect();
    for row in catalog {
        if !known.insert(row.code.clone()) {continue;}
        if excluded_sector_name(&row.name) {
            state["excluded_sectors"].as_array_mut().ok_or("排除目录无效")?.push(json!({"code":row.code,"name":row.name}));
        } else {state["queue"].as_array_mut().unwrap().push(json!({"kind":row.kind,"code":row.code,"name":row.name}));}
    }
    state["total"]=json!(state["queue"].as_array().unwrap().len());
    state["catalog_total"]=json!(state["queue"].as_array().unwrap().len()+state["excluded_sectors"].as_array().map_or(0,Vec::len));
    state["catalog_errors"]=json!(errors);state["catalog_retry_pending"]=json!(false);
    Ok(())
}
async fn discover_batch_inner(db:&Database,revision:u64)->Result<(),String> {
    let day=completed_day(Utc::now())?.to_string();
    let mut state:Value=serde_json::from_str(&db.get_setting(DISCOVERY_STATE).map_err(|e|e.to_string())?.unwrap_or_else(||"{}".into())).map_err(|_|"主线发现进度损坏")?;
    if state["as_of"].as_str()!=Some(&day) || state["schema"].as_str()!=Some(DISCOVERY_SCHEMA) {
        let (bk,sw)=tokio::join!(sector::fetch_filter_catalog(),crate::datasource::sw_sector::fetch_catalog());
        let mut errors=Vec::new();let mut catalog=match bk{Ok(rows)=>rows,Err(e)=>{errors.push(format!("东方财富目录：{e}"));Vec::new()}};
        match sw{Ok(mut rows)=>{rows.append(&mut catalog);catalog=rows;},Err(e)=>errors.push(format!("申万目录：{e}"))};
        let queue:Vec<Value>=catalog.iter().filter(|r|!excluded_sector_name(&r.name)).map(|r|json!({"kind":r.kind,"code":r.code,"name":r.name})).collect();
        if queue.is_empty(){return Err("全市场板块目录为空，停止扫描".into());}
        state=json!({"schema":DISCOVERY_SCHEMA,"as_of":day,"total":queue.len(),"catalog_total":catalog.len(),"excluded_sectors":catalog.iter().filter(|r|excluded_sector_name(&r.name)).map(|r|json!({"code":r.code,"name":r.name})).collect::<Vec<_>>(),"processed":0,"failed":[],"candidates":[],"finished":false,"last_error":null,"queue":queue,
            "scope":"申万发行方行业与东方财富行业/概念目录；共享真实交易日窗口，非交易日记录剔除，真实交易日缺失拒绝；失败单列","catalog_errors":errors});
        if !save_discovery_progress(db,&state,revision)?{return Ok(());}
    }
    if state["finished"]==true {return Ok(());}
    if state["catalog_retry_pending"]==true {
        retry_discovery_catalog(&mut state).await?;
        if !save_discovery_progress(db,&state,revision)? {return Ok(());}
    }
    if state["retry_queue"].as_array().is_some_and(|rows|state["retry_processed"].as_u64()==Some(rows.len() as u64)) {
        state.as_object_mut().unwrap().remove("retry_queue");
    }
    if !state["retry_queue"].is_array()&&state["processed"].as_u64()==state["total"].as_u64() {
        state["finished"]=json!(true);save_discovery_progress(db,&state,revision)?;return Ok(());
    }
    let retrying=state["retry_queue"].is_array();
    let counter=if retrying{"retry_processed"}else{"processed"};
    let begin=state[counter].as_u64().ok_or("发现进度无效")? as usize;
    let queue=state[if retrying{"retry_queue"}else{"queue"}].as_array().ok_or("发现队列缺失")?.clone();
    if begin>queue.len(){return Err("发现进度超出目录".into());}
    let limit=(begin+80).min(queue.len());let index=benchmark(&day).await?;
    // ponytail: eight concurrent requests and one checkpoint per eight; pause may redo at most eight sectors.
    for chunk in queue[begin..limit].chunks(8) {
        let Some(mut tasks)=start_discovery_chunk(db,revision,chunk,&day,&index) else{return Ok(());};
        while let Some(result)=tasks.join_next().await {
            let (row,result)=result.map_err(|e|e.to_string())?;
            apply_discovery_result(&mut state,&row,result)?;
        }
        let processed=state[counter].as_u64().unwrap() as usize+chunk.len();
        state[counter]=json!(processed);
        if retrying&&processed==queue.len() {
            state.as_object_mut().unwrap().remove("retry_queue");
            state["finished"]=json!(state["processed"].as_u64()==state["total"].as_u64());
        } else {state["finished"]=json!(!retrying&&processed==queue.len());}
        state["candidates"].as_array_mut().ok_or("候选进度无效")?.sort_by(|a,b|number(&b["metrics"],"rs20_vs_hs300").total_cmp(&number(&a["metrics"],"rs20_vs_hs300")).then_with(||a["code"].as_str().cmp(&b["code"].as_str())));
        if !save_discovery_progress(db,&state,revision)?{return Ok(());}
    }
    Ok(())
}

pub(crate) fn completed_day(now: chrono::DateTime<Utc>) -> Result<NaiveDate, String> {
    let local = now.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
    let mut day = local.date_naive();
    if local.hour() * 60 + local.minute() < 15 * 60 + 10 { day -= Duration::days(1); }
    for _ in 0..35 {
        if trading_calendar::is_trading_day_at(now, day)? { return Ok(day); }
        day -= Duration::days(1);
    }
    Err("无法确认最近已完成交易日".into())
}

fn mean(bars: &[KLineData], n: usize) -> f64 { bars[bars.len()-n..].iter().map(|b| b.close).sum::<f64>() / n as f64 }
fn metrics(bars: &[KLineData]) -> Option<Value> {
    if bars.len() < 66 || bars.iter().any(|b| [b.open,b.high,b.low,b.close].iter().any(|p|!p.is_finite()||*p<=0.0)||b.high<b.open.max(b.close)||b.low>b.open.min(b.close)||b.low>b.high) { return None; }
    let n = bars.len(); let last = &bars[n-1];
    let ma20 = mean(bars,20); let ma60 = mean(bars,60);
    let atr = bars[n-14..].iter().enumerate().map(|(i,b)| {
        let prev = bars[n-15+i].close;
        (b.high-b.low).max((b.high-prev).abs()).max((b.low-prev).abs())
    }).sum::<f64>() / 14.0;
    if !atr.is_finite() || atr <= 0.0 { return None; }
    let persistence = (0..5).filter(|offset| {
        let slice = &bars[..n-offset]; let p = slice.last().unwrap().close;
        p > mean(slice,20) && mean(slice,20) > mean(slice,60)
    }).count();
    let strong = last.close > ma20 && ma20 > ma60 && ma20 > mean(&bars[..n-5],20) && persistence >= 3;
    Some(json!({"close":last.close,"ma20":ma20,"ma60":ma60,"atr":atr,"r5":(last.close/bars[n-6].close-1.0)*100.0,
        "r20":(last.close/bars[n-21].close-1.0)*100.0,"r60":(last.close/bars[n-61].close-1.0)*100.0,"persistence":persistence,
        "strong":strong,"prior_high20":bars[n-21..n-1].iter().map(|b|b.high).fold(0.0,f64::max)}))
}
fn number(v: &Value, key: &str) -> f64 { v[key].as_f64().unwrap_or(0.0) }
fn is_stock(member: &sector::SectorMember) -> bool {
    let code=&member.code;
    code.len()==6 && code.bytes().all(|b|b.is_ascii_digit()) &&
    ((member.market=="sh" && (code.starts_with("60")||code.starts_with("68"))) ||
     (member.market=="sz" && (code.starts_with("00")||code.starts_with("30")))) &&
    !crate::market_rules::is_st(&member.name) && !member.name.contains('退')
}
fn plan(bars: &[KLineData], raw: &[KLineData], m: &Value,symbol:&str) -> Value {
    let unavailable=json!({"status":"reference_unavailable","reason":"截止日未复权价格或指标缺失/无效，暂无价位参考"});
    let Some(last)=bars.last() else{return unavailable;};
    let Some(raw_last)=raw.last() else{return unavailable;};
    if last.date!=raw_last.date || [last.close,raw_last.close,number(m,"ma20"),number(m,"atr"),number(m,"prior_high20")].iter().any(|v|!v.is_finite()||*v<=0.0) {return unavailable;}
    let scale=raw_last.close/last.close;
    let ma20=number(m,"ma20")*scale; let atr=number(m,"atr")*scale;
    let prior=number(m,"prior_high20")*scale;
    if [ma20-0.25*atr,ma20+0.25*atr,prior,prior+0.5*atr].iter().any(|v|!v.is_finite()||*v<=0.0) {return unavailable;}
    let trial_high=ma20+0.25*atr;
    let (minimum,step)=crate::market_rules::buy_lot(symbol);
    let cash=100_000.0;let trial_budget=cash*0.016;let maximum_budget=cash*0.08;
    let minimum_cost=minimum as f64*trial_high;
    let fee=5.0f64.max(minimum_cost*0.0003)+minimum_cost*0.00001;
    let trial_quantity=if trial_budget>=minimum_cost+fee {
        minimum+(((trial_budget-fee)/trial_high-minimum as f64)/step as f64).floor().max(0.0) as i64*step
    }else{0};
    json!({"status":"unvalidated_observation","max_position_pct":8,
        "reference_prices":{"as_of":raw_last.date,"close":raw_last.close,"ma20":ma20,"atr":atr,"prior_high20":prior},
        "lot_budget":{"reference_cash_cny":cash,"minimum_shares":minimum,"quantity_step":step,"trial_budget_cny":trial_budget,"max_stock_budget_cny":maximum_budget,
            "minimum_trade_value_cny":minimum_cost,"estimated_minimum_fees_cny":fee,"trial_quantity":trial_quantity,"trial_feasible":trial_quantity>=minimum,
            "within_max_stock_budget":minimum_cost+fee<=maximum_budget,"note":"仅按十万元研究参考账户估算，非个人可用资金；费用为参考代理；买不起最小单位则仅形态观察，不自动扩大仓位"},
        "trial":{"position_pct":1.6,"buy_low":ma20-0.25*atr,"buy_high":ma20+0.25*atr,"condition":"回踩区止跌，下一完成分钟重新站上VWAP；不得仅按触价买入"},
        "confirm":{"position_pct":2.4,"buy_low":prior,"buy_high":prior+0.5*atr,"condition":"前高突破且完成分钟量价确认，已买批次T+1；高开超过区间则放弃"},
        "final":{"position_pct":4,"condition":"突破后回踩不破并重新走强才补齐；任何破位取消后续加仓"},
        "profit_trim":{"sell_position_pct":30,"atr_above_cost":2,"trigger":"相对持仓成本盈利达到两倍ATR后，跌破最近完成分钟VWAP再保护盈利"},
        "weakness_trim":{"sell_position_pct":50,"trigger_price":ma20,"condition":"完成日收盘跌破MA20，下一可成交时减仓；跌停无法卖出则保留待处理"},
        "exit":{"atr_from_cost":3,"preferred_days":15,"max_holding_days":20,"review_range_days":[5,20],"condition":"成本下三倍ATR或期限复核趋势失效时退出；T+1、停牌和涨跌停约束仍适用"},
        "price_basis":"前复权指标换算到该截止日未复权价格；这是观察计划，分批动作尚未证明优于整仓"})
}
fn recent_news(db: &Database, name: &str, leaders: &[Value],as_of:&str) -> Result<Vec<Value>, String> {
    let symbols=leaders.iter().filter_map(|r|r["symbol"].as_str().map(str::to_owned)).collect::<Vec<_>>();
    let names=leaders.iter().filter_map(|r|r["name"].as_str().map(str::to_owned)).collect::<Vec<_>>();
    db.recent_research_news(&[name.to_owned()],&symbols,&names,as_of)
}

fn hash_value(row:&Value)->Result<String,String>{Ok(hex::encode(Sha256::digest(serde_json::to_vec(row).map_err(|e|e.to_string())?)))}
fn evidence_fingerprint(row:&Value)->Result<String,String>{
    let mut evidence=row.clone();let fields=evidence.as_object_mut().ok_or("主线快照结构无效")?;
    // Retrieval times expire independently; identical evidence keeps the same notification target.
    for field in ["fingerprint","content_sha256","generated_at","membership_observed_at"]{fields.remove(field);}
    hash_value(&evidence)
}
pub(crate) fn verify_snapshot(row:&Value)->Result<(),String>{
    let fingerprint=row["fingerprint"].as_str().ok_or("快照无证据指纹")?;
    if let Some(expected)=row["content_sha256"].as_str(){
        let mut content=row.clone();content.as_object_mut().ok_or("快照结构无效")?.remove("content_sha256");
        if hash_value(&content)?!=expected||evidence_fingerprint(row)?!=fingerprint{return Err("主线快照正文或证据指纹校验失败".into());}
    }else{
        // Existing snapshots used a full-content hash; preserve their history without silent relabeling.
        let mut original=row.clone();original.as_object_mut().ok_or("快照结构无效")?.remove("fingerprint");
        if hash_value(&original)?!=fingerprint{return Err("旧主线快照指纹校验失败".into());}
    }
    Ok(())
}
fn seal_snapshot(mut row:Value)->Result<Value,String>{
    row.as_object_mut().ok_or("快照结构无效")?.remove("content_sha256");
    row["fingerprint"]=evidence_fingerprint(&row)?.into();
    row["content_sha256"]=hash_value(&row)?.into();Ok(row)
}
fn save_snapshot(db:&Database,row:&Value)->Result<(),String>{
    verify_snapshot(row)?;
    let mut archive:Vec<Value>=serde_json::from_str(&db.get_setting(SNAPSHOT_ARCHIVE).map_err(|e|e.to_string())?.unwrap_or_else(||"[]".into())).map_err(|_|"主线证据留存结构损坏")?;
    archive.retain(|r|r["fingerprint"]!=row["fingerprint"]);archive.insert(0,row.clone());archive.truncate(64);
    db.set_setting(SNAPSHOT_ARCHIVE,&json!(archive).to_string()).map_err(|e|e.to_string())?;
    db.set_setting(&format!("mainline_snapshot_{}",row["sector_code"].as_str().ok_or("快照无板块代码")?),&row.to_string()).map_err(|e|e.to_string())
}
fn find_snapshot(db:&Database,fingerprint:&str)->Result<Value,String>{
    if fingerprint.len()!=64||!fingerprint.bytes().all(|b|b.is_ascii_hexdigit()){return Err("观察快照指纹无效".into());}
    let archive:Vec<Value>=serde_json::from_str(&db.get_setting(SNAPSHOT_ARCHIVE).map_err(|e|e.to_string())?.unwrap_or_else(||"[]".into())).map_err(|_|"主线证据留存结构损坏")?;
    let saved=archive.into_iter().find(|r|r["fingerprint"].as_str()==Some(fingerprint)).or_else(||
        db.get_all_settings().ok()?.into_iter().filter(|(k,_)|k.starts_with("mainline_snapshot_BK")||k.starts_with("mainline_snapshot_SW"))
            .filter_map(|(_,raw)|serde_json::from_str::<Value>(&raw).ok()).find(|r|r["fingerprint"].as_str()==Some(fingerprint)));
    let row=match saved {Some(row)=>row,None=>{
        let now=Utc::now();db.brief_alerts_between(&(now-Duration::days(14)).to_rfc3339(),&now.to_rfc3339())?.into_iter()
            .filter_map(|r|r.get("mainline_snapshot").cloned()).find(|r|r["fingerprint"].as_str()==Some(fingerprint)).ok_or("通知对应的历史快照未留存或已过期，不能用当前结果代替")?
    }};
    verify_snapshot(&row)?;Ok(row)
}

pub(crate) fn news_targets(db:&Database)->Result<Vec<Value>,String>{
    let day=completed_day(Utc::now())?.to_string();let watched=watches(db)?;
    let mut targets=Vec::new();let mut added=std::collections::HashSet::new();
    let snapshots=db.get_all_settings().map_err(|e|e.to_string())?.into_iter().filter(|(k,_)|k.starts_with("mainline_snapshot_BK")||k.starts_with("mainline_snapshot_SW"))
        .filter_map(|(_,raw)|serde_json::from_str::<Value>(&raw).ok()).filter(|r|verify_snapshot(r).is_ok()).collect::<Vec<_>>();
    for watch in watched {
        if validate(&watch.kind,&watch.code,&watch.name).is_err(){continue;}
        let current=snapshots.iter().find(|r|r["sector_code"].as_str()==Some(&watch.code)&&r["as_of"].as_str()==Some(&day)&&r["complete"]==true);
        added.insert(watch.code.clone());targets.push(news_target(&watch.kind,&watch.code,&watch.name,current));
    }
    for row in snapshots.iter().filter(|r|r["as_of"].as_str()==Some(&day)&&r["complete"]==true&&r["strong"]==true){
        let code=row["sector_code"].as_str().unwrap_or("");let kind=row["kind"].as_str().unwrap_or("");let name=row["sector_name"].as_str().unwrap_or("");
        if validate(kind,code,name).is_ok()&&added.insert(code.to_owned()){targets.push(news_target(kind,code,name,Some(row)));}
    }
    Ok(targets)
}
fn news_target(kind:&str,code:&str,name:&str,row:Option<&Value>)->Value{
    let leaders=row.and_then(|r|r["leaders"].as_array());
    json!({"kind":kind,"sector_code":code,"sector_name":name,"as_of":row.map(|r|r["as_of"].clone()),"fingerprint":row.map(|r|r["fingerprint"].clone()),"snapshot_current":row.is_some(),
        "symbols":leaders.map(|rows|rows.iter().filter_map(|r|r["symbol"].as_str()).collect::<Vec<_>>()).unwrap_or_default(),
        "names":leaders.map(|rows|rows.iter().filter_map(|r|r["name"].as_str()).collect::<Vec<_>>()).unwrap_or_default()})
}
#[tauri::command]
pub fn get_mainline_alert_history(db:State<'_,Arc<Database>>)->Result<Vec<Value>,String>{
    let now=Utc::now();Ok(db.brief_alerts_between(&(now-Duration::days(14)).to_rfc3339(),&now.to_rfc3339())?.into_iter().filter(|r|r["signal_kind"]=="research").take(100).collect())
}

pub async fn scan(db: &Database, kind: &str, code: &str, name: &str) -> Result<Value,String> {
    let kind=validate(kind,code,name)?;
    let _permit=GATE.get_or_init(||tokio::sync::Semaphore::new(1)).try_acquire().map_err(|_|"主线研究正在扫描，请稍后重试")?;
    let local_history=db.get_setting("local_history_enabled").map_err(|e|e.to_string())?.as_deref()==Some("1");
    let day=completed_day(Utc::now())?; let as_of=day.to_string();
    let (members,index)=tokio::try_join!(sector_members(kind,code),benchmark(&as_of))?;
    let hist=sector_history(code,&index).await?;
    let bars:Vec<_>=hist.items.iter().filter(|b|b.date<=as_of).map(|b|KLineData{date:b.date.clone(),open:b.open,high:b.high,low:b.low,close:b.close,volume:0,turnover:0.0}).collect();
    let sector_metrics=relative_metrics(&bars,&index)?;
    let sector_date=bars.last().unwrap().date.clone();
    let excluded_base=members.items.iter().filter(|m|!is_stock(m)).count();
    let eligible:Vec<_>=members.items.iter().filter(|m|is_stock(m)).cloned().collect();
    if eligible.len()>1500 {return Err("该概念覆盖超过一千五百只股票，请选具体主线板块；不会以部分股票冒充完整扫描".into());}
    let config=history::LocalHistoryConfig::new(db.get_setting("local_history_url").map_err(|e|e.to_string())?.unwrap_or_else(||"http://127.0.0.1:7899".into()));
    let start=(day-Duration::days(400)).to_string();
    let mut queue=tokio::task::JoinSet::new(); let mut results=Vec::new();
    // Eight reads at once; online observations never supply historical ST or raw execution prices.
    for member in eligible.iter().cloned() {
        let cfg=config.clone();let begin=start.clone();let end=as_of.clone();
        queue.spawn(async move {
            let symbol=format!("{}{}",member.market,member.code);
            let history=if local_history {history::fetch_daily(&cfg,&symbol,Some(&begin),Some(&end)).await} else {
                crate::datasource::kline::fetch_qfq_daily_kline_with_source(&symbol,120).await.map(|(rows,source)| {
                    let rows:Vec<_>=rows.into_iter().filter(|row|row.date<=end).collect();
                    history::LocalHistoryResult {start_date:rows.first().map(|r|r.date.clone()),end_date:rows.last().map(|r|r.date.clone()),sample_count:rows.len(),
                        klines:rows,raw_klines:Vec::new(),st_by_date:Vec::new(),source:source.label().into(),protocol:history::LocalHistoryProtocol::Json}
                })
            };
            (member,history)
        });
        if queue.len()>=8 {results.push(queue.join_next().await.ok_or("扫描队列缺失")?.map_err(|e|e.to_string())?);}
    }
    while let Some(row)=queue.join_next().await {results.push(row.map_err(|e|e.to_string())?);}
    let mut candidates=Vec::new();let mut missing=Vec::new();let mut covered=0;let mut trending=0;let mut excluded=excluded_base;
    for (member,result) in results {
        let symbol=format!("{}{}",member.market,member.code);
        let quoted=member.quoted_at_unix.and_then(|t|chrono::DateTime::from_timestamp(t,0));
        if !valid_sw_code(code) && !quoted.is_some_and(|t|t<=Utc::now()+Duration::minutes(5) && t.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap()).date_naive()==day) {
            missing.push(json!({"symbol":symbol,"reason":"该成分行情时间缺失、未来或不是截止交易日"}));continue;
        }
        let history=match result {Ok(v)=>v,Err(e)=>{missing.push(json!({"symbol":symbol,"reason":e}));continue;}};
        let m=match member_metrics(&history,&index,&as_of,local_history){
            Ok(Some(value))=>value,
            Ok(None)=>{excluded+=1;continue;},
            Err(reason)=>{missing.push(json!({"symbol":symbol,"reason":reason}));continue;},
        };
        covered+=1;
        if m["strong"]==true{trending+=1;}
        let r20=number(&m,"r20");let r60=number(&m,"r60");let distance=(number(&m,"close")-number(&m,"ma20"))/number(&m,"atr");
        if m["strong"]!=true || r20<=number(&sector_metrics,"r20") || r20<=0.0 || distance>2.0 || history.klines.last().unwrap().volume==0 {continue;}
        let plan=if local_history {plan(&history.klines,&history.raw_klines,&m,&symbol)} else {json!({"status":"reference_unavailable","reason":"当前为在线近期趋势观察；历史买卖价位研究需安装并启用StockDB"})};
        let mut row=m;row["symbol"]=symbol.into();row["name"]=member.name.into();row["as_of"]=as_of.clone().into();
        row["score"]=(0.5*r20+0.3*r60-0.2*distance).into();row["plan"]=plan;row["pe"]=json!(member.pe);row["pb"]=json!(member.pb);
        candidates.push(row);
    }
    candidates.sort_by(|a,b|number(b,"score").total_cmp(&number(a,"score")).then_with(||a["symbol"].as_str().cmp(&b["symbol"].as_str())));
    let member_date=members.as_of.get(..10).unwrap_or("");
    let complete=missing.is_empty() && !eligible.is_empty() && sector_date==as_of && (valid_sw_code(code)||member_date==as_of);
    let observed_members_at=members.as_of.clone();
    let breadth=if covered>0 {trending as f64/covered as f64}else{0.0};
    let strong=complete && sector_metrics["strong"]==true && breadth>=0.5 && !candidates.is_empty();
    let mut leaders:Vec<_>=candidates.into_iter().take(10).collect();
    let context_failure=|error:String|json!({"schema":"stock-research-context-v1","production_admission":false,"exploration":true,"models":[],"limitations":[error]});
    let model_research=super::research_evidence::model_research_context(db,&as_of).unwrap_or_else(context_failure);
    for leader in &mut leaders {leader["model_research"]=super::research_evidence::stock_research_context(db,leader["symbol"].as_str().unwrap_or(""),&as_of).unwrap_or_else(context_failure);}
    let news=recent_news(db,name,&leaders,&as_of)?;
    let mut output=json!({"schema":"mainline-observation-v1","model_status":"unvalidated_hypothesis","sector_code":code,"sector_name":name,"kind":kind,
        "as_of":as_of,"sector_source":hist.source,"sector_history_end":sector_date,"member_source":members.source,"member_as_of":as_of,"membership_observed_at":observed_members_at,"membership_source_is_current_snapshot":true,
        "member_history_source":if local_history{"StockDB"}else{"在线近期前复权日线"},"historical_research_available":local_history,
        "total_members":members.total,"excluded_members":excluded,"covered_members":covered,"missing_members":missing,
        "complete":complete,"strong":strong,"status":if strong{"持续趋势及强股条件满足，研究观察"}else if !complete{"数据覆盖或时点不完整，停止提醒"}else{"持续趋势或强股条件未通过"},
        "metrics":sector_metrics,"benchmark":{"symbol":"sh000300","source":"Tencent explicit index daily","as_of":as_of},"trend_breadth":breadth,"trending_members":trending,"leaders":leaders,"model_research":model_research,"news":news,"generated_at":Utc::now().to_rfc3339(),
        "limitations":["近期主线观察可直接获取在线行情；多年走势、历史ST和买卖价位研究需安装并启用StockDB；在线观察不把前复权价格冒充实际成交价", "当前成分股不是历史成分，不能倒推多年板块策略收益；短于六十六个真实交易日的历史记为缺失，未核验上市日期不排除", "主线及分层计划未获多年模型准入；主线条件加入股票模型尚未证明长期增益，形态排序分不是上涨概率", "PE/PB是当前估值代理，缺公告时点财报，不能判定成长质量", "资讯仅最近七日采集归档最多五千条中的结构化股票/名称/主题匹配，非全网完整覆盖；采集时间不同于发布时间，截点之后的公告只作为新信息", "盘中确认、T+1及无法成交只写观察条件，本模块不自动下单"]});
    output=seal_snapshot(output)?;save_snapshot(db,&output)?;
    Ok(output)
}
#[tauri::command]
pub async fn get_sector_mainline(db: State<'_,Arc<Database>>, kind:String, sector_code:String, sector_name:String,fingerprint:Option<String>)->Result<Value,String> {
    if let Some(fingerprint)=fingerprint{let row=find_snapshot(&db,&fingerprint)?;if row["sector_code"]!=sector_code||row["kind"]!=kind{return Err("通知身份与留存快照不同，拒绝跳转".into());}return Ok(row);}
    scan(&db,&kind,&sector_code,&sector_name).await
}
#[tauri::command]
pub async fn analyze_mainline(db: State<'_,Arc<Database>>, fingerprint:String)->Result<Value,String> {
    let row=find_snapshot(&db,&fingerprint)?;
    let generated=chrono::DateTime::parse_from_rfc3339(row["generated_at"].as_str().ok_or("快照日期无效")?).map_err(|_|"快照日期无效")?;
    if generated>Utc::now()+Duration::minutes(1){return Err("观察快照生成时间在未来，拒绝汇总".into());}
    if Utc::now().signed_duration_since(generated)>Duration::hours(24) {return Err("观察快照超过一天，请重新扫描后汇总最新资讯".into());}
    crate::agent::summarize_mainline(&db,&fingerprint,row).await
}

fn discovery_alert_ready(state:&Value, day:&str)->bool {
    state["as_of"].as_str()==Some(day) && state["finished"]==true
        && state["catalog_errors"].as_array().is_some_and(Vec::is_empty)
        && state["failed"].as_array().is_some_and(Vec::is_empty)
        && state["candidates"].as_array().is_some_and(|rows|rows.iter().all(|r|r["as_of"].as_str()==Some(day)))
}
fn snapshot_alert_ready(row:&Value,day:&str,now:chrono::DateTime<Utc>)->bool{
    row["complete"]==true && row["as_of"].as_str()==Some(day)
        && completed_day(now).is_ok_and(|current|current.to_string()==day)
}
fn research_notice(row:&Value,title:String,body:String)->Value{
    json!({"schema":"mainline-research-notification-v1","signal_id":format!("mainline:{}:{}",row["sector_code"].as_str().unwrap_or(""),row["as_of"].as_str().unwrap_or("")),
        "signal_kind":"research","signal_tag":"主线观察","title":title,"body":body,"sector_code":row["sector_code"],"sector_name":row["sector_name"],"kind":row["kind"],"as_of":row["as_of"],
        "fingerprint":row["fingerprint"],"mainline_snapshot":row,"production_admission":false})
}

pub async fn scheduled_tick(db:&Database, app:&tauri::AppHandle) {
    let now=Utc::now();let local=now.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
    let day=match completed_day(now) {Ok(d)=>d.to_string(),Err(_)=>return};
    if discovery_enabled(db) {
        let revision=DISCOVERY_REVISION.load(Ordering::SeqCst);
        let result=tokio::select! { biased; _=discovery_changed(db,revision)=>Ok(()), result=async {
        discover_batch(db).await?;
        // 完整全目录扫描结束后，由系统挑选前列主线，用户无需预先指定板块。
        if let Ok(state)=discovery_status(db) {
            if discovery_alert_ready(&state,&day) {
                for candidate in state["candidates"].as_array().into_iter().flatten().filter(|r|r["entry_ready"].as_bool().unwrap_or_else(||number(r,"extension_atr")<=2.0)).take(5) {
                    let code=candidate["code"].as_str().unwrap_or("");let key=format!("mainline_discovered_checked_{code}");
                    if db.get_setting(&key).ok().flatten().as_deref()==Some(&day){continue;}
                    match scan(db,candidate["kind"].as_str().unwrap_or(""),code,candidate["name"].as_str().unwrap_or("")).await {
                        Ok(row) if row["complete"]==true=>{
                            let _control=DISCOVERY_CONTROL.get_or_init(||std::sync::Mutex::new(())).lock().unwrap_or_else(|e|e.into_inner());
                            if !discovery_enabled(db) || DISCOVERY_REVISION.load(Ordering::SeqCst)!=revision
                                || !discovery_alert_ready(&state,&day) || !snapshot_alert_ready(&row,&day,Utc::now()) {break;}
                            if row["strong"]==true && db.claim_news(&format!("mainline:{code}:{day}"),"mainline",&Utc::now().to_rfc3339()).unwrap_or(false) {
                                let names=row["leaders"].as_array().into_iter().flatten().take(3).filter_map(|r|r["name"].as_str()).collect::<Vec<_>>().join("、");
                                crate::notifications::publish(app,research_notice(&row,format!("系统发现主线：{}",row["sector_name"].as_str().unwrap_or("")),
                                    format!("截至{day}全市场扫描后，完整成分验证通过。强势观察股：{names}。查看该次证据、模型记录和近期原文；主线仅辅助观察，模型尚未准入。")));
                            }
                            let _=db.set_setting(&key,&day);
                        },Ok(_)=>{},Err(e)=>log::warn!(target: "automation::mainline", "主线成分扫描失败：{e}"),
                    }
                }
            }
        }
        Ok::<(),String>(())
        }=>result };
        if let Err(error)=result {
            log::warn!(target: "automation::mainline", "全市场扫描失败：{error}");
            let _control=DISCOVERY_CONTROL.get_or_init(||std::sync::Mutex::new(())).lock().unwrap_or_else(|e|e.into_inner());
            if discovery_enabled(db) && DISCOVERY_REVISION.load(Ordering::SeqCst)==revision {
                let _=db.set_setting("mainline_discovery_last_error",&json!(error).to_string());
            }
        }
    }
    // Explicitly watched sectors keep their separate after-close reminder schedule.
    if local.hour()<15 || (local.hour()==15 && local.minute()<10) {return;}

    for watch in watches(db).unwrap_or_default() {
        let key=format!("mainline_checked_{}",watch.code);
        if db.get_setting(&key).ok().flatten().as_deref()==Some(&day) {continue;}
        match scan(db,&watch.kind,&watch.code,&watch.name).await {
            Ok(row) if snapshot_alert_ready(&row,&day,Utc::now())=>{
                if row["strong"]==true && db.claim_news(&format!("mainline:{}:{day}",watch.code),"mainline",&Utc::now().to_rfc3339()).unwrap_or(false) {
                    let names=row["leaders"].as_array().into_iter().flatten().take(3).filter_map(|r|r["name"].as_str()).collect::<Vec<_>>().join("、");
                    crate::notifications::publish(app,research_notice(&row,format!("主线观察：{}",watch.name),
                        format!("截至{day}持续趋势与完整扫描条件满足，强势观察股：{names}。查看该次证据、模型记录和近期原文；主线仅辅助观察，模型尚未准入。")));
                }
                if let Err(e)=db.set_setting(&key,&day) {log::warn!("[mainline] 保存提醒水位失败：{e}");}
            },
            Ok(_)=>{},Err(e)=>log::warn!(target: "automation::mainline", "板块 {} 检查失败：{e}",watch.code),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_alignment_discards_closed_rows_without_hiding_missing_real_sessions() {
        use chrono::Datelike;
        let mut date=NaiveDate::from_ymd_opt(2026,6,1).unwrap();let mut rows=Vec::new();
        while rows.len()<90 {
            if !matches!(date.weekday(),chrono::Weekday::Sat|chrono::Weekday::Sun) {
                let close=10.0+rows.len() as f64*0.1;
                rows.push(KLineData{date:date.to_string(),open:close,high:close+0.2,low:close-0.2,close,volume:100,turnover:1.0});
            }
            date+=Duration::days(1);
        }
        let mut contaminated=rows.clone();let mut weekend=rows[30].clone();weekend.date="2026-07-04".into();contaminated.push(weekend);contaminated.sort_by(|a,b|a.date.cmp(&b.date));
        let expected=relative_metrics(&rows,&rows).unwrap();let actual=relative_metrics(&contaminated,&rows).unwrap();
        assert_eq!(actual["removed_non_trading_rows"],1);assert_eq!(actual["window_sessions"],66);
        for key in ["r20","r60","ma20","ma60","atr","rs20_vs_hs300","rs60_vs_hs300"] {assert_eq!(expected[key],actual[key],"{key}");}
        let missing_date=contaminated[40].date.clone();contaminated.remove(40);
        assert!(relative_metrics(&contaminated,&rows).unwrap_err().contains(&missing_date));
        let mut data=history::LocalHistoryResult{klines:rows.clone(),raw_klines:rows.clone(),st_by_date:vec![(rows.last().unwrap().date.clone(),Some(false))],
            source:"offline fixture".into(),protocol:history::LocalHistoryProtocol::Json,start_date:Some(rows[0].date.clone()),end_date:Some(rows.last().unwrap().date.clone()),sample_count:rows.len()};
        let as_of=rows.last().unwrap().date.as_str();assert!(member_metrics(&data,&rows,as_of,true).unwrap().is_some());
        data.klines=rows[30..].to_vec();data.raw_klines=data.klines.clone();data.sample_count=data.klines.len();
        assert!(member_metrics(&data,&rows,as_of,true).unwrap_err().contains("缺少真实交易日"),"Short history must remain missing, not an inferred IPO exclusion");
        data.st_by_date[0].1=Some(true);assert!(member_metrics(&data,&rows,as_of,true).unwrap().is_none(),"Only verified ST status excludes this member");
        data.st_by_date[0].1=None;assert!(member_metrics(&data,&rows,as_of,true).is_err());
        data.st_by_date[0].1=Some(false);data.end_date=Some(rows[88].date.clone());assert!(member_metrics(&data,&rows,as_of,true).is_err());
        data.klines=rows.clone();data.raw_klines.clear();data.st_by_date.clear();data.end_date=Some(as_of.into());
        let observed=member_metrics(&data,&rows,as_of,false).unwrap().unwrap();
        assert!(member_metrics(&data,&rows,as_of,true).is_err(),"Online names must not become historical ST evidence");
        assert_eq!(plan(&data.klines,&data.raw_klines,&observed,"sh600519")["status"],"reference_unavailable");
        data.end_date=Some(rows[88].date.clone());assert!(member_metrics(&data,&rows,as_of,false).is_err());
        let mut duplicate=rows.clone();duplicate.insert(40,rows[40].clone());assert!(relative_metrics(&duplicate,&rows).is_err());
        let mut broken=rows.clone();broken[89].close=f64::NAN;assert!(relative_metrics(&broken,&rows).is_err());
    }
    #[tokio::test]
    async fn pause_cancels_inflight_tasks_and_rejects_late_progress() {
        let root=std::env::temp_dir().join(format!("bull-mainline-pause-{}",uuid::Uuid::new_v4()));let db=Arc::new(Database::open(root.clone()).unwrap());
        set_discovery_enabled(&db,true,false).unwrap();let revision=DISCOVERY_REVISION.load(Ordering::SeqCst);
        let first=json!({"schema":DISCOVERY_SCHEMA,"processed":8,"total":80,"finished":false});assert!(save_discovery_progress(&db,&first,revision).unwrap());
        // An empty index fails before HTTP; use the real batch starter without contacting any source.
        let chunk=vec![json!({"kind":"concept","code":"BK1305","name":"离线测试"})];
        let mut batch=start_discovery_chunk(&db,revision,&chunk,"2026-09-30",&[]).unwrap();
        assert!(batch.join_next().await.unwrap().unwrap().1.is_err());
        let started=Arc::new(tokio::sync::Notify::new());let stopped=Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_db=db.clone();let worker_started=started.clone();let worker_stopped=stopped.clone();
        let worker=tokio::spawn(async move {
            tokio::select! {biased; _=discovery_changed(&worker_db,revision)=>{}, _=async {
                let mut tasks=tokio::task::JoinSet::new();
                tasks.spawn(async move {
                    struct MarkStopped(Arc<std::sync::atomic::AtomicBool>);impl Drop for MarkStopped {fn drop(&mut self){self.0.store(true,Ordering::SeqCst);}}
                    let _guard=MarkStopped(worker_stopped);worker_started.notify_one();std::future::pending::<()>().await;
                });let _=tasks.join_next().await;
            }=>{} }
        });
        started.notified().await;set_discovery_enabled(&db,false,false).unwrap();
        assert!(start_discovery_chunk(&db,revision,&chunk,"2026-09-30",&[]).is_none(),"No new batch may start after pause returns");
        tokio::time::timeout(std::time::Duration::from_secs(1),worker).await.unwrap().unwrap();tokio::task::yield_now().await;
        assert!(stopped.load(Ordering::SeqCst));assert!(!save_discovery_progress(&db,&json!({"processed":16}),revision).unwrap());
        assert_eq!(discovery_status(&db).unwrap()["processed"],8);assert_eq!(discovery_status(&db).unwrap()["enabled"],false);
        let resumed=set_discovery_enabled(&db,true,true).unwrap();assert_eq!(resumed["processed"],8);
        assert!(start_discovery_chunk(&db,revision,&chunk,"2026-09-30",&[]).is_none(),"Resume cannot let an old worker start a batch");
        let new_revision=DISCOVERY_REVISION.load(Ordering::SeqCst);assert!(save_discovery_progress(&db,&json!({"processed":16,"finished":true,"failed":[],"catalog_errors":[]}),new_revision).unwrap());
        set_discovery_enabled(&db,true,true).unwrap();assert_eq!(discovery_status(&db).unwrap()["processed"],16,"completed same-day scans are not repeated");
        let good=json!({"kind":"industry","code":"SW801080","name":"电子"});
        let bad=json!({"kind":"concept","code":"BK1305","name":"测试概念"});
        let candidate=json!({"code":"SW801080","metrics":{"strong":true,"r20":5.0}});
        let complete=json!({"schema":DISCOVERY_SCHEMA,"as_of":"2026-09-30","processed":2,"total":2,"finished":true,
            "queue":[good,bad],"candidates":[candidate],"failed":[{"code":"BK1305","reason":"缺少交易日"}],"catalog_errors":[]});
        db.set_setting(DISCOVERY_STATE,&complete.to_string()).unwrap();
        let resumed=set_discovery_enabled(&db,true,true).unwrap();
        assert_eq!(resumed["processed"],2);assert_eq!(resumed["candidates"],complete["candidates"]);
        assert_eq!(resumed["retry_total"],1);assert_eq!(resumed["finished"],false);assert!(resumed["retry_queue"].is_null());
        let mut private:Value=serde_json::from_str(&db.get_setting(DISCOVERY_STATE).unwrap().unwrap()).unwrap();
        assert_eq!(private["retry_queue"],json!([bad]));
        private["retry_processed"]=json!(1);assert!(save_discovery_progress(&db,&private,DISCOVERY_REVISION.load(Ordering::SeqCst)).unwrap());
        set_discovery_enabled(&db,false,false).unwrap();let again=set_discovery_enabled(&db,true,true).unwrap();
        assert_eq!(again["retry_processed"],1,"pause/resume cannot reset the failure retry cursor");
        assert_eq!(again["candidates"],complete["candidates"]);
        apply_discovery_result(&mut private,&bad,Err("备用源仍缺历史".into())).unwrap();
        assert_eq!(private["failed"].as_array().unwrap().len(),1);assert_eq!(private["candidates"],complete["candidates"]);
        let recovered=json!({"code":"BK1305","metrics":{"strong":true,"r20":7.0}});
        apply_discovery_result(&mut private,&bad,Ok(recovered.clone())).unwrap();
        apply_discovery_result(&mut private,&bad,Ok(recovered)).unwrap();
        assert!(private["failed"].as_array().unwrap().is_empty());assert_eq!(private["candidates"].as_array().unwrap().len(),2);
        assert_eq!(private["processed"],2);assert_eq!(private["total"],2);
        let mut catalog_only=complete.clone();catalog_only["failed"]=json!([]);catalog_only["catalog_errors"]=json!(["目录请求失败"]);
        assert!(prepare_discovery_retry(&mut catalog_only).unwrap());assert_eq!(catalog_only["retry_total"],0);
        assert_eq!(catalog_only["catalog_retry_pending"],true);assert_eq!(catalog_only["candidates"],complete["candidates"]);
        let mut corrupt=complete.clone();corrupt["failed"]=json!([{"code":"BK9999"}]);
        assert!(prepare_discovery_retry(&mut corrupt).is_err());assert_eq!(corrupt["candidates"],complete["candidates"]);
        set_discovery_enabled(&db,false,false).unwrap();drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stable_evidence_preserves_history_but_rejects_tamper(){
        let first=seal_snapshot(json!({"schema":"mainline-observation-v1","sector_code":"SW801080","sector_name":"电子","kind":"industry","as_of":"2026-09-30","complete":true,"strong":true,
            "generated_at":"2026-10-01T01:00:00Z","membership_observed_at":"2026-10-01T01:00:00Z","leaders":[{"symbol":"sh600000","name":"浦发银行"}]})).unwrap();
        let mut refreshed=first.clone();refreshed.as_object_mut().unwrap().remove("fingerprint");refreshed.as_object_mut().unwrap().remove("content_sha256");
        refreshed["generated_at"]=json!("2026-10-01T02:00:00Z");refreshed["membership_observed_at"]=json!("2026-10-01T02:00:00Z");let refreshed=seal_snapshot(refreshed).unwrap();
        assert_eq!(first["fingerprint"],refreshed["fingerprint"]);assert_ne!(first["content_sha256"],refreshed["content_sha256"]);
        assert!(verify_snapshot(&refreshed).is_ok());let mut corrupt=refreshed.clone();corrupt["strong"]=json!(false);assert!(verify_snapshot(&corrupt).is_err());
        let root=std::env::temp_dir().join(format!("bull-mainline-evidence-{}",uuid::Uuid::new_v4()));let db=Database::open(root.clone()).unwrap();
        save_snapshot(&db,&first).unwrap();let mut newer=first.clone();newer.as_object_mut().unwrap().remove("content_sha256");newer["strong"]=json!(false);let newer=seal_snapshot(newer).unwrap();save_snapshot(&db,&newer).unwrap();
        assert_eq!(find_snapshot(&db,first["fingerprint"].as_str().unwrap()).unwrap()["strong"],true);
        let mut notice=research_notice(&first,"观察".into(),"详情".into());assert_eq!(notice["kind"],"industry");assert_eq!(notice["mainline_snapshot"]["fingerprint"],first["fingerprint"]);
        notice["id"]=json!("persisted-test");notice["received_at"]=json!(Utc::now().timestamp_millis());db.archive_brief_alert(&notice).unwrap();db.set_setting(SNAPSHOT_ARCHIVE,"[]").unwrap();
        assert_eq!(find_snapshot(&db,first["fingerprint"].as_str().unwrap()).unwrap()["strong"],true,"Archived notification still opens its evidence when the rolling snapshot cache evicts it");
        let target=news_target("industry","SW801080","电子",Some(&first));assert_eq!(target["symbols"][0],"sh600000");assert_eq!(target["snapshot_current"],true);
        let unknown=news_target("industry","SW801080","电子",None);assert!(unknown["fingerprint"].is_null());assert!(unknown["symbols"].as_array().unwrap().is_empty());
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reference_plan_preserves_raw_price_basis_and_rejects_missing_prices() {
        let adjusted=vec![KLineData{date:"2026-09-30".into(),open:20.0,high:21.0,low:19.0,close:20.0,volume:100,turnover:1.0}];
        let raw=vec![KLineData{date:"2026-09-30".into(),open:10.0,high:10.5,low:9.5,close:10.0,volume:100,turnover:1.0}];
        let m=json!({"close":20.0,"ma20":19.6,"atr":0.8,"prior_high20":21.0});
        let p=plan(&adjusted,&raw,&m,"sh600001");
        let near=|value:&Value,expected:f64|assert!((value.as_f64().unwrap()-expected).abs()<1e-10);
        assert_eq!(p["status"],"unvalidated_observation");assert_eq!(p["reference_prices"]["as_of"],"2026-09-30");
        near(&p["reference_prices"]["close"],10.0);near(&p["reference_prices"]["ma20"],9.8);near(&p["reference_prices"]["atr"],0.4);
        near(&p["trial"]["buy_low"],9.7);near(&p["trial"]["buy_high"],9.9);near(&p["confirm"]["buy_low"],10.5);near(&p["confirm"]["buy_high"],10.7);
        near(&p["weakness_trim"]["trigger_price"],9.8);assert_eq!(p["profit_trim"]["atr_above_cost"],2);assert_eq!(p["exit"]["atr_from_cost"],3);
        let mut mismatched=raw.clone();mismatched[0].date="2026-09-29".into();
        for missing in [&[][..],&mismatched[..]] {let rejected=plan(&adjusted,missing,&m,"sh600001");assert_eq!(rejected["status"],"reference_unavailable");assert!(rejected.get("trial").is_none());}
        let mut invalid=raw.clone();invalid[0].close=f64::NAN;
        assert_eq!(plan(&adjusted,&invalid,&m,"sh600001")["status"],"reference_unavailable");
        assert_eq!(plan(&[],&raw,&m,"sh600001")["status"],"reference_unavailable");
        let mut bad_metrics=m.clone();bad_metrics["atr"]=json!(0);assert_eq!(plan(&adjusted,&raw,&bad_metrics,"sh600001")["status"],"reference_unavailable");
        bad_metrics["atr"]=json!(1e308);assert_eq!(plan(&adjusted,&raw,&bad_metrics,"sh600001")["status"],"reference_unavailable");
    }
    #[test]
    fn causal_trend_and_calendar_guard() {
        let bars:Vec<_>=(0..90).map(|i|{let p=10.0+i as f64*0.1;KLineData{date:i.to_string(),open:p,high:p+0.15,low:p-0.15,close:p,volume:100,turnover:1.0}}).collect();
        let m=metrics(&bars).unwrap();assert_eq!(m["strong"],true);assert_eq!(m["persistence"],5);
        assert!(number(&m,"r20")>0.0);assert!(metrics(&bars[..60]).is_none());
        let mut declining=bars.clone();for b in declining.iter_mut(){b.close=30.0-b.close; b.open=b.close;b.high=b.close+0.15;b.low=b.close-0.15;}
        assert_eq!(metrics(&declining).unwrap()["strong"],false);
        use chrono::TimeZone;
        assert_eq!(completed_day(Utc.with_ymd_and_hms(2026,10,1,8,0,0).unwrap()).unwrap().to_string(),"2026-09-30");
        assert_eq!(completed_day(Utc.with_ymd_and_hms(2026,9,30,6,0,0).unwrap()).unwrap().to_string(),"2026-09-29");
        let before_close=Utc.with_ymd_and_hms(2026,9,30,7,9,59).unwrap();let after_close=Utc.with_ymd_and_hms(2026,9,30,7,10,0).unwrap();
        let previous=json!({"as_of":"2026-09-29","complete":true});let current=json!({"as_of":"2026-09-30","complete":true});
        assert!(snapshot_alert_ready(&previous,"2026-09-29",before_close));
        assert!(!snapshot_alert_ready(&previous,"2026-09-29",after_close),"An old tick must not publish or advance its watermark after the cutoff");
        assert!(!snapshot_alert_ready(&current,"2026-09-29",after_close),"A current snapshot cannot use an old discovery date");
        assert!(!snapshot_alert_ready(&current,"2026-09-30",before_close));assert!(snapshot_alert_ready(&current,"2026-09-30",after_close));
        assert!(!snapshot_alert_ready(&json!({"as_of":"2026-09-30","complete":false}),"2026-09-30",after_close));
        assert!(!valid_code("BK12/3"));
        let p=plan(&bars,&bars,&m,"sh688655");
        assert_eq!(p["lot_budget"]["minimum_shares"],200);
        assert_eq!(p["lot_budget"]["trial_feasible"],false);
        let mut discovery=json!({"as_of":"2026-09-30","finished":true,"catalog_errors":[],"failed":[],"candidates":[{"as_of":"2026-09-30"}]});
        assert!(discovery_alert_ready(&discovery,"2026-09-30"));
        assert!(!discovery_alert_ready(&discovery,"2026-10-08"));
        discovery["failed"]=json!([{"code":"BK0955"}]);
        assert!(!discovery_alert_ready(&discovery,"2026-09-30"));
        discovery["failed"]=json!([]);discovery["candidates"][0]["as_of"]=json!("2026-09-29");
        assert!(!discovery_alert_ready(&discovery,"2026-09-30"));
    }
    #[tokio::test]
    #[ignore = "requires restored loopback StockDB and complete Eastmoney sector network"]
    async fn live_complete_sector_scan() {
        let root=std::env::temp_dir().join(format!("bull-mainline-smoke-{}",uuid::Uuid::new_v4()));
        let db=Database::open(root.clone()).unwrap();db.set_setting("local_history_enabled","1").unwrap();
        let value=scan(&db,"industry","SW801080","电子").await.unwrap();
        assert_eq!(value["model_status"],"unvalidated_hypothesis");
        assert_eq!(value["total_members"].as_u64().unwrap(),value["covered_members"].as_u64().unwrap()+value["excluded_members"].as_u64().unwrap()+value["missing_members"].as_array().unwrap().len() as u64);
        println!("{}",value);
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore = "read-only live full catalog scan; no notifications or accounts"]
    async fn live_discovery_starts_without_user_sector_list(){
        let root=std::env::temp_dir().join(format!("bull-mainline-discovery-{}",uuid::Uuid::new_v4()));
        let db=Database::open(root.clone()).unwrap();
        assert!(watches(&db).unwrap().is_empty());
        set_discovery_enabled(&db,true,false).unwrap();discover_batch(&db).await.unwrap();let result=discovery_status(&db).unwrap();
        assert_eq!(result["processed"],80);assert!(result["total"].as_u64().unwrap()>100);
        assert!(!result["finished"].as_bool().unwrap());
        assert!(result["candidates"].as_array().unwrap().iter().all(|c| c["metrics"]["strong"]==true && number(&c["metrics"],"rs20_vs_hs300")>0.0));
        while !discovery_status(&db).unwrap()["finished"].as_bool().unwrap_or(false) {
            discover_batch(&db).await.unwrap();
        }
        let result=discovery_status(&db).unwrap();
        assert_eq!(result["processed"],result["total"]);
        let initial=result.clone();
        set_discovery_enabled(&db,false,false).unwrap();let resumed=set_discovery_enabled(&db,true,true).unwrap();
        assert_eq!(resumed["processed"],initial["processed"]);assert_eq!(resumed["candidates"],initial["candidates"]);
        while !discovery_status(&db).unwrap()["finished"].as_bool().unwrap_or(false) { discover_batch(&db).await.unwrap(); }
        let final_result=discovery_status(&db).unwrap();
        assert_eq!(final_result["processed"],initial["processed"]);assert_eq!(final_result["total"],initial["total"]);
        for candidate in initial["candidates"].as_array().unwrap() {
            assert!(final_result["candidates"].as_array().unwrap().iter().any(|row|row==candidate),"successful sector must be retained: {}",candidate["code"]);
        }
        let report=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/mainline-check/live-discovery-resume.json");
        std::fs::create_dir_all(report.parent().unwrap()).unwrap();
        std::fs::write(report,serde_json::to_vec_pretty(&json!({"initial":initial,"resumed":resumed,"final":final_result})).unwrap()).unwrap();
        set_discovery_enabled(&db,false,false).unwrap();
        println!("{}",json!({"as_of":result["as_of"],"processed":result["processed"],"total":result["total"],
            "failed":result["failed"].as_array().unwrap().len(),"candidates":result["candidates"].as_array().unwrap().len(),"catalog_errors":result["catalog_errors"]}));
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}
