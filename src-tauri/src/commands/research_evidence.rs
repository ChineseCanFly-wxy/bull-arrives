//! 保留全部模型结果。导入只登记证据，不把自报收益转换为策略准入。
use crate::db::Database;
use serde_json::{json, Value};
use sha2::{Digest,Sha256};
use std::sync::Arc;
use tauri::State;

fn decode(raw:&str)->Result<Value,String>{
    if raw.len()>1024*1024{return Err("研究证据超过一 MiB".into());}
    let envelope:Value=serde_json::from_str(raw).map_err(|_|"证据文件不是有效 JSON")?;
    if envelope["schema"]!="research-evidence-bundle-v1" {return Err("证据文件版本不支持".into());}
    let content=envelope["content"].as_str().ok_or("缺少证据正文")?;
    let hash=hex::encode(Sha256::digest(content.as_bytes()));
    if envelope["content_sha256"].as_str()!=Some(&hash){return Err("证据内容指纹不符，拒绝导入".into());}
    let mut evidence:Value=serde_json::from_str(content).map_err(|_|"证据正文结构无效")?;
    if evidence["schema"]!="ashare-evidence-v1" || evidence["production_admission"]!=false || evidence["exploration"]!=true {return Err("本入口只接收探索性证据；不得通过导入自动宣称生产准入".into());}
    let run_id=evidence["run_id"].as_str().ok_or("证据缺运行标识")?;
    if run_id.is_empty() || run_id.len()>100 || !run_id.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_.".contains(&b)){return Err("证据运行标识无效".into());}
    let date=evidence["as_of"].as_str().ok_or("证据缺截止日期")?;
    let date=chrono::NaiveDate::parse_from_str(date,"%Y-%m-%d").map_err(|_|"证据截止日期无效")?;
    if date>chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap()).date_naive(){return Err("证据截止日期在未来".into());}
    let data=&evidence["data"];
    if data["stocks"].as_u64().is_none_or(|n|n==0) || data["sessions"].as_u64().is_none_or(|n|n==0)
        || data["source"].as_str().is_none_or(str::is_empty) || data["sha256"].as_str().is_none_or(|s|s.len()!=64||!s.bytes().all(|b|b.is_ascii_hexdigit()))
        || data["first_date"].as_str().and_then(|d|chrono::NaiveDate::parse_from_str(d,"%Y-%m-%d").ok()).is_none()
        || data["last_date"].as_str().and_then(|d|chrono::NaiveDate::parse_from_str(d,"%Y-%m-%d").ok()).is_none(){return Err("数据覆盖字段缺失或无效".into());}
    for key in ["limitations","next_checks"]{if evidence[key].as_array().is_none_or(|a|a.iter().any(|v|v.as_str().is_none())){return Err("研究说明列表结构无效".into());}}
    for key in ["annual_choices","stress","checks"]{if evidence.get(key).is_none(){return Err("研究实验或检查记录缺失".into());}}
    let models=evidence["models"].as_array().ok_or("缺少模型比较")?;
    if models.is_empty()||models.len()>200{return Err("模型比较数量无效".into());}
    let mut ids=std::collections::HashSet::new();
    for row in models {
        let id=row["id"].as_str().ok_or("模型缺标识")?;
        if !ids.insert(id)||id.len()>100 {return Err("模型标识重复或过长".into());}
        if row["name"].as_str().is_none()||row["status"].as_str().is_none()||row["features"].as_u64().is_none_or(|n|n==0)
            ||row["holding_days"].as_u64().is_none_or(|n|n==0)||row["closed_cycles"].as_u64().is_none()||row["unique_stocks"].as_u64().is_none(){return Err("模型描述或样本字段缺失".into());}
        for key in ["double_cost_return_pct","win_rate_pct"]{if row[key].as_f64().is_none_or(|n|!n.is_finite()){return Err("成本或胜率字段缺失".into());}}
        if row["win_rate_pct"].as_f64().is_none_or(|n|!(0.0..=100.0).contains(&n)){return Err("周期胜率范围无效".into());}
        for key in ["train","validation","test"]{if row["baseline_percentile"][key].as_f64().is_none_or(|n|!(0.0..=100.0).contains(&n)){return Err("同池基线数值分位缺失".into());}}
        for period in ["train","validation","test"] {
            let net=row[period]["net_return_pct"].as_f64().ok_or("模型缺净收益")?;
            let dd=row[period]["max_drawdown_pct"].as_f64().ok_or("模型缺回撤")?;
            if !net.is_finite()||!dd.is_finite()||!(0.0..=100.0).contains(&dd)||net < -100.0{return Err("模型收益或回撤数值无效".into());}
        }
    }
    evidence["content_sha256"]=hash.into();
    evidence["integrity_note"]="正文 SHA256 已校验；仅验证内部完整性，不证明收益计算正确或来源已认证；导入不建账户、不生成订单".into();
    Ok(evidence)
}
#[tauri::command]
pub fn get_research_evidence(db:State<'_,Arc<Database>>)->Result<Value,String>{
    read_evidence(&db)
}
fn read_evidence(db:&Database)->Result<Value,String>{
    if let Some(raw)=db.get_setting("research_evidence_bundle").map_err(|e|e.to_string())?{return decode(&raw);}
    decode(include_str!("../../research-evidence.json"))
}

fn decode_screen(raw:&str)->Result<Value,String>{
    if raw.len()>1024*1024{return Err("模型候选文件过大".into());}
    let envelope:Value=serde_json::from_str(raw).map_err(|_|"模型候选文件无效")?;
    let content=envelope["content"].as_str().ok_or("模型候选缺正文")?;
    if envelope["schema"]!="model-screen-bundle-v1" || envelope["content_sha256"].as_str()!=Some(&hex::encode(Sha256::digest(content.as_bytes()))) {return Err("模型候选指纹或格式无效".into());}
    let mut screen:Value=serde_json::from_str(content).map_err(|_|"模型候选正文无效")?;
    if screen["schema"]!="ashare-model-screen-v1" || screen["production_admission"]!=false || screen["exploration"]!=true {return Err("候选只能登记为未准入研究观察".into());}
    let day=screen["as_of"].as_str().ok_or("候选缺日期")?;
    chrono::NaiveDate::parse_from_str(day,"%Y-%m-%d").map_err(|_|"候选日期无效")?;
    let hash_valid=|v:&Value|v.as_str().is_some_and(|h|h.len()==64&&h.bytes().all(|b|b.is_ascii_hexdigit()));
    if !hash_valid(&screen["snapshot_sha256"]) || screen["run_id"].as_str().is_none() || screen["limitations"].as_array().is_none_or(|a|a.iter().any(|v|v.as_str().is_none())) {return Err("候选来源或说明缺失".into());}
    let models=screen["models"].as_array().ok_or("候选模型缺失")?;
    if models.len()!=2{return Err("常用入口只接收两个冻结研究模型".into());}
    let mut ids=std::collections::HashSet::new();
    for model in models {
        let id=model["id"].as_str().ok_or("候选模型标识缺失")?;
        let features=match id{"breadth22_h20"=>22,"index26_h20"=>26,_=>return Err("未知研究模型".into())};
        if !ids.insert(id)||model["feature_count"]!=features||model["holding_days"]!=20||model["name"].as_str().is_none(){return Err("模型版本描述不符".into());}
        let rows=model["candidates"].as_array().ok_or("候选股票缺失")?;
        if rows.len()>100||model["positive_count"].as_u64()!=Some(rows.len() as u64)||model["scored_stocks"].as_u64().is_none_or(|n|n<rows.len() as u64||n>10000){return Err("候选覆盖统计无效".into());}
        let mut all_scores=std::collections::HashMap::new();
        if let Some(scores)=model.get("all_scores"){
            let scores=scores.as_array().ok_or("完整评分表结构无效")?;
            if scores.len()!=model["scored_stocks"].as_u64().unwrap() as usize||!hash_valid(&model["model_sha256"])||!hash_valid(&model["score_cache_sha256"]){return Err("完整评分覆盖或模型来源指纹无效".into());}
            for row in scores {
                let symbol=row["symbol"].as_str().ok_or("完整评分缺股票代码")?;
                let ordinary=symbol.is_ascii()&&symbol.len()==8&&symbol[2..].bytes().all(|b|b.is_ascii_digit())
                    && (symbol.starts_with("sh60")||symbol.starts_with("sh68")||symbol.starts_with("sz00")||symbol.starts_with("sz30"));
                let score=row["score"].as_f64().filter(|s|s.is_finite()).ok_or("完整评分数值无效")?;
                if !ordinary||all_scores.insert(symbol,score).is_some(){return Err("完整评分包含重复、北交所或非普通沪深股票".into());}
            }
            if all_scores.values().filter(|v|**v>0.0).count()!=rows.len(){return Err("完整评分正分数量与候选不一致".into());}
        }
        let mut stocks=std::collections::HashSet::new();
        for row in rows {
            let symbol=row["symbol"].as_str().ok_or("股票代码缺失")?;
            let ordinary=symbol.is_ascii()&&symbol.len()==8&&symbol[2..].bytes().all(|b|b.is_ascii_digit())&&((symbol.starts_with("sh60")||symbol.starts_with("sh68"))||(symbol.starts_with("sz00")||symbol.starts_with("sz30")));
            let name=row["name"].as_str().unwrap_or("");
            if !ordinary||!stocks.insert(symbol)||name.to_uppercase().contains("ST")||name.contains('退')||row["as_of"].as_str()!=Some(day){return Err("候选包含重复、ST、北交所、非普通股或日期错配".into());}
            if row["close"].as_f64().is_none_or(|v|!v.is_finite()||v<=0.0)||row["score"].as_f64().is_none_or(|v|!v.is_finite()||v<=0.0)||row["signal_eligible"].as_bool().is_none()
                ||!hash_valid(&row["model_sha256"])||!hash_valid(&row["score_cache_sha256"])||row["reasons"].as_array().is_none_or(|a|a.iter().any(|v|v.as_str().is_none())) {return Err("候选评分、资格或来源字段无效".into());}
            if model.get("all_scores").is_some()&&(all_scores.get(symbol).is_none_or(|score|(*score-row["score"].as_f64().unwrap()).abs()>1e-12)
                ||row["model_sha256"]!=model["model_sha256"]||row["score_cache_sha256"]!=model["score_cache_sha256"]){return Err("候选与完整评分或模型指纹不一致".into());}
        }
    }
    screen["content_sha256"]=envelope["content_sha256"].clone();Ok(screen)
}

async fn revalidate_screen(db:&Database, mut screen:Value)->(Value,bool){
    let enabled=db.get_setting("local_history_enabled").ok().flatten().as_deref()==Some("1");
    let cfg=crate::datasource::history::LocalHistoryConfig::new(db.get_setting("local_history_url").ok().flatten().unwrap_or_else(||"http://127.0.0.1:7899".into()));
    let day=screen["as_of"].as_str().unwrap().to_owned();let mut complete=enabled;
    for model in screen["models"].as_array_mut().unwrap() {
        for row in model["candidates"].as_array_mut().unwrap() {
            let result=if !enabled {Err("本地历史数据未启用，冻结观察未核对".to_string())} else {
                let symbol=row["symbol"].as_str().unwrap();
                crate::datasource::history::fetch_daily(&cfg,symbol,Some(&day),Some(&day)).await.and_then(|history|{
                    if history.st_by_date.iter().find(|(d,_)|d==&day).and_then(|(_,st)|*st)!=Some(false){return Err("截止日ST状态为是或未知，停止当前观察".into());}
                    let raw=history.raw_klines.last().ok_or("截止日行情缺失")?;
                    if raw.date!=day||(raw.close-row["close"].as_f64().unwrap()).abs()>0.011{return Err("截止日或价格与冻结候选不符，不能复用旧预测".into());}
                    Ok(())
                })
            };
            row["live_status"]=match result{Ok(())=>json!("截止日行情与非ST核对通过；仍非买入许可"),Err(e)=>{complete=false;json!(e)}};
        }
    }
    (screen,complete)
}

fn screen_evidence_matches(screen:&Value,evidence:&Value)->Result<(),String>{
    // The screen was exported from this exact frozen study, not any similarly named imported run.
    const EVIDENCE_SHA:&str="f7001af4893c4a90a13f906c25f7a0fab174dc7d649b027cd7a94846ac0f3a46";
    if evidence["content_sha256"].as_str()!=Some(EVIDENCE_SHA)||screen["run_id"]!=evidence["run_id"]||screen["snapshot_sha256"]!=evidence["data"]["sha256"] {
        return Err("研究证据与候选的冻结正文版本不同；新导入绩效不能套用旧候选，需匹配的新推断版本".into());
    }
    Ok(())
}

fn frozen_context(screen:&Value,evidence:&Value,symbol:Option<&str>,as_of:&str)->Result<Value,String>{
    screen_evidence_matches(screen,evidence)?;
    chrono::NaiveDate::parse_from_str(as_of,"%Y-%m-%d").map_err(|_|"个股研究截止日期无效")?;
    if let Some(symbol)=symbol {
        let ordinary=symbol.is_ascii()&&symbol.len()==8&&symbol[2..].bytes().all(|b|b.is_ascii_digit())
            && (symbol.starts_with("sh60")||symbol.starts_with("sh68")||symbol.starts_with("sz00")||symbol.starts_with("sz30"));
        if !ordinary{return Err("研究上下文只支持沪深普通A股，排除北交所及B股".into());}
    }
    let date_matches=screen["as_of"].as_str()==Some(as_of);
    let mut models=Vec::new();
    for model in screen["models"].as_array().ok_or("冻结模型记录缺失")? {
        let performance=evidence["models"].as_array().ok_or("多年证据缺失")?.iter().find(|r|r["id"]==model["id"]).ok_or("冻结模型无对应多年证据")?;
        if performance["features"]!=model["feature_count"]||performance["holding_days"]!=model["holding_days"] {return Err("冻结模型的特征与持有期不匹配".into());}
        let observation=if date_matches {symbol.and_then(|s|model["candidates"].as_array()?.iter().find(|r|r["symbol"].as_str()==Some(s)))}else{None};
        let full_score=if date_matches{symbol.and_then(|s|model["all_scores"].as_array()?.iter().find(|r|r["symbol"].as_str()==Some(s)).and_then(|r|r["score"].as_f64()))}else{None};
        let score=full_score.or_else(||observation.and_then(|r|r["score"].as_f64()));
        let status=if !date_matches{"date_mismatch"}else if symbol.is_none(){"not_requested"}else if observation.is_some(){"positive_record"}else if full_score.is_some(){"nonpositive_record"}
            else if model.get("all_scores").is_some(){"not_in_scored_export"}else{"not_in_positive_export"};
        models.push(json!({"id":model["id"],"name":model["name"],"feature_count":model["feature_count"],"holding_days":model["holding_days"],
            "signal_status":status,"score":score,"score_as_of":score.map(|_|screen["as_of"].clone()),"model_sha256":model["model_sha256"],"score_cache_sha256":model["score_cache_sha256"],
            "observation":observation,"scored_stocks":model["scored_stocks"],"positive_count":model["positive_count"],"performance":performance}));
    }
    Ok(json!({"schema":"stock-research-context-v1","symbol":symbol,"requested_as_of":as_of,"frozen_as_of":screen["as_of"],"date_matches":date_matches,
        "run_id":screen["run_id"],"snapshot_sha256":screen["snapshot_sha256"],"screen_sha256":screen["content_sha256"],"evidence_sha256":evidence["content_sha256"],"evidence_as_of":evidence["as_of"],
        "exploration":true,"production_admission":false,"models":models,
        "limitations":["读取冻结完整评分时，非正分来自真实模型记录；不在评分导出中表示未知，不能猜作零分；旧版只有正分表时缺失也未知；没有本地重新推断", "模型分值是二十日收益标签代理，非上涨概率、胜率或买入许可", "截止日不一致时只保留多年模型证据，不展示为当前评分；行情、ST及可成交性仍需独立核对", "多年收益、成本压力和回撤属于对应组合实验，不能归为这只个股的预期收益或这条主线的历史胜率"]}))
}

pub(crate) fn model_research_context(db:&Database,as_of:&str)->Result<Value,String>{
    frozen_context(&decode_screen(include_str!("../../model-screen.json"))?,&read_evidence(db)?,None,as_of)
}

/// Read frozen, hash-bound observations only. Never calls a provider, network or inference.
pub(crate) fn stock_research_context(db:&Database,symbol:&str,as_of:&str)->Result<Value,String>{
    let mut context=frozen_context(&decode_screen(include_str!("../../model-screen.json"))?,&read_evidence(db)?,Some(symbol),as_of)?;
    let runner_sha=hex::encode(Sha256::digest(include_bytes!("../../../research/research-center-runner/model_runner.py")));
    apply_current_scores(&mut context,&db.enabled_model_runs()?,symbol,as_of,&runner_sha);
    let mut groups=Vec::new();
    for summary in db.research_jobs()?.iter().filter(|j|j["kind"]=="scan"&&j["state"]=="complete") {
        let job=db.research_job(summary["id"].as_i64().ok_or("任务缺身份")?)?;
        groups.extend(job["results"].as_array().into_iter().flatten().map(|s|s["group"].clone()));
    }
    apply_scan_scores(&mut context,&groups,symbol,as_of,&runner_sha)?;
    Ok(context)
}

fn apply_scan_scores(context:&mut Value,groups:&[Value],symbol:&str,as_of:&str,runner_sha:&str)->Result<(),String>{
    let registry:Value=serde_json::from_str(include_str!("../../../research/research-center-runner/registry.json")).map_err(|_|"模型注册表损坏")?;
    let year=as_of.get(..4).and_then(|s|s.parse::<i64>().ok()).ok_or("模型日期无效")?;
    let mut scans=Vec::new();
    for id in crate::commands::research_jobs::MODEL_IDS {
        let spec=&registry["models"][id];
        let expected=if year==2026 {Some(&registry["files"][format!("{id}_model2026")]["sha256"])}else{None};
        let Some(group)=groups.iter().find(|g|g["model_id"]==id&&g["as_of"]==as_of&&g["runner_sha256"]==runner_sha&&g["production_admission"]==false&&Some(&g["model_sha256"])==expected&&g["threshold"].as_f64().is_some()&&g["threshold"].as_f64()==spec["signal_threshold"].as_f64()) else {continue;};
        let score=group["current_scores"].as_array().into_iter().flatten().find(|r|r["symbol"]==symbol&&r["as_of"]==as_of).and_then(|r|r["score"].as_f64()).filter(|s|s.is_finite());
        let threshold=group["threshold"].as_f64().ok_or("模型阈值无效")?;
        let observation=group["candidates"].as_array().into_iter().flatten().find(|r|r["symbol"]==symbol).cloned().unwrap_or(Value::Null);
        let status=match score{Some(s) if s>threshold=>"positive_record",Some(_)=>"nonpositive_record",None=>"not_in_scored_export"};
        let source="手动筛选任务的当日冻结模型推断；原历史组合绩效不是该股未来胜率";
        let mut item=group.clone();let fields=item.as_object_mut().ok_or("模型组无效")?;fields.remove("candidates");fields.remove("current_scores");
        item["score"]=json!(score);item["signal_status"]=json!(status);item["score_source"]=json!(source);item["observation"]=observation.clone();
        scans.push(item);
        if let Some(model)=context["models"].as_array_mut().into_iter().flatten().find(|m|m["id"]==id&&m["model_sha256"]==group["model_sha256"]){
            model["score"]=json!(score);model["score_as_of"]=json!(score.map(|_|as_of));model["signal_status"]=json!(status);model["signal_threshold"]=json!(threshold);model["observation"]=observation;
            model["score_source"]=json!(source);model["model_run_id"]=Value::Null;model["model_run_sha256"]=Value::Null;model["model_scan_job_id"]=group["job_id"].clone();model["model_scan_source_fingerprint"]=group["source_fingerprint"].clone();model["current_data_sha256"]=group["data_sha256"].clone();
        }
    }
    context["latest_model_scan"]=json!({"as_of":as_of,"models":scans,"production_admission":false,"message":"只附上同日同受信模型版本的实际筛选证据；不存在的模型/个股评分保持未知，不补零。分数按各模型自己的标签解释"});Ok(())
}

fn apply_current_scores(context:&mut Value,runs:&[Value],symbol:&str,as_of:&str,runner_sha:&str){
    for model in context["models"].as_array_mut().into_iter().flatten(){
        let Some(run)=runs.iter().rev().find(|r|r["mode"]=="forward"&&r["as_of"].as_str()==Some(as_of)&&r["model_id"]==model["id"]&&r["runner_sha256"].as_str()==Some(runner_sha)&&r["model_sha256"]==model["model_sha256"]&&r["current_scores"].is_array()) else {continue;};
        let score=run["current_scores"].as_array().unwrap().iter().find(|r|r["symbol"].as_str()==Some(symbol)).and_then(|r|r["score"].as_f64());
        let threshold=run["signal_threshold"].as_f64().unwrap();
        model["score"]=json!(score);model["score_as_of"]=json!(score.map(|_|as_of));model["signal_threshold"]=json!(threshold);
        model["signal_status"]=json!(match score{Some(n) if n>threshold=>"positive_record",Some(_)=>"nonpositive_record",None=>"not_in_scored_export"});
        model["observation"]=run["signal_watch"].as_array().unwrap().iter().find(|r|r["symbol"].as_str()==Some(symbol)).cloned().unwrap_or(Value::Null);
        model["score_source"]=json!("受信本机前向程序当日推断；原多年绩效仍为历史研究");
        model["model_run_id"]=run["id"].clone();model["model_run_sha256"]=run["content_sha256"].clone();model["current_data_sha256"]=run["data_sha256"].clone();
    }
}

async fn model_screen_view(db:&Database)->Result<Value,String>{
    let evidence=read_evidence(db)?;
    let mut screen=decode_screen(include_str!("../../model-screen.json"))?;
    screen_evidence_matches(&screen,&evidence)?;
    let mut models=Vec::new();
    for model in screen["models"].as_array().unwrap(){
        let row=evidence["models"].as_array().unwrap().iter().find(|r|r["id"]==model["id"]).ok_or("研究证据缺对应模型")?;
        if row["features"]!=model["feature_count"]||row["holding_days"]!=model["holding_days"]{return Err("模型特征或期限版本不符".into());}
        models.push(row.clone());
    }
    let day=crate::commands::mainline::completed_day(chrono::Utc::now())?.to_string();
    let fresh=screen["as_of"].as_str()==Some(&day);
    let mut limitations=vec!["刷新仅读取冻结预测并核对截止日价格、ST；没有重新推断模型，也没有认证整份行情或未来走势".to_string(),"正分是二十日收益标签代理，不是胜率；下一开盘、分钟确认和可成交性仍未知".to_string()];
    let mut complete=false;
    if fresh {
        match tokio::time::timeout(std::time::Duration::from_secs(25),revalidate_screen(db,screen.clone())).await {
            Ok((value,ok))=>{screen=value;complete=ok;},Err(_)=>limitations.push("本地行情核对超时，全部保留为未核对冻结观察".into())
        }
    }else{limitations.push("候选截止日不是最新已完成市场交易日，禁止当作当日预测；须新增行情推断并保存新版本".into());}
    Ok(json!({"schema":"research-model-screen-view-v1","current_as_of":day,"fresh":fresh,"screen":screen,"models":models,"revalidation_complete":complete,"limitations":limitations}))
}
#[tauri::command]
pub async fn get_research_model_screen(db:State<'_,Arc<Database>>)->Result<Value,String>{model_screen_view(&db).await}
#[tauri::command]
pub fn import_research_evidence(db:State<'_,Arc<Database>>,path:String)->Result<Value,String>{
    let path=std::path::PathBuf::from(path);
    let meta=std::fs::metadata(&path).map_err(|e|format!("无法读取研究证据：{e}"))?;
    if !meta.is_file()||meta.len()>1024*1024 {return Err("请选一 MiB 内的 JSON 证据文件".into());}
    let raw=std::fs::read_to_string(path).map_err(|e|e.to_string())?;
    let evidence=decode(&raw)?;
    db.set_setting("research_evidence_bundle",&raw).map_err(|e|e.to_string())?;
    Ok(evidence)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn forward_score_requires_matching_date_model_and_trusted_runner(){
        let mut context=json!({"models":[{"id":"breadth22_h20","model_sha256":"model","score":null,"signal_status":"date_mismatch"}]});
        let row=json!({"id":3,"mode":"forward","model_id":"breadth22_h20","model_sha256":"model","runner_sha256":"runner","as_of":"2026-09-29","signal_threshold":0,"current_scores":[{"symbol":"sz301068","score":-0.01}],"signal_watch":[]});
        apply_current_scores(&mut context,&[row.clone()],"sz301068","2026-09-30","runner");assert!(context["models"][0]["score"].is_null());
        apply_current_scores(&mut context,&[row.clone()],"sz301068","2026-09-29","wrong");assert!(context["models"][0]["score"].is_null());
        apply_current_scores(&mut context,&[row.clone()],"sz301068","2026-09-29","runner");assert_eq!(context["models"][0]["score"],-0.01);assert_eq!(context["models"][0]["signal_status"],"nonpositive_record");
        apply_current_scores(&mut context,&[row],"sz300000","2026-09-29","runner");assert!(context["models"][0]["score"].is_null());assert_eq!(context["models"][0]["signal_status"],"not_in_scored_export");
    }
    #[test]
    fn current_scan_evidence_requires_exact_day_and_registered_artifact_hash(){
        let registry:Value=serde_json::from_str(include_str!("../../../research/research-center-runner/registry.json")).unwrap();
        let id="breadth22_rank20";let sha=&registry["files"]["breadth22_rank20_model2026"]["sha256"];
        let mut context=json!({"models":[]});let mut group=json!({"model_id":id,"as_of":"2026-09-30","runner_sha256":"runner","model_sha256":sha,"production_admission":false,"threshold":0.6,"current_scores":[{"symbol":"sz000001","as_of":"2026-09-30","score":0.59}],"candidates":[],"job_id":1});
        apply_scan_scores(&mut context,&[group.clone()],"sz000001","2026-09-30","runner").unwrap();assert_eq!(context["latest_model_scan"]["models"][0]["score"],0.59);assert_eq!(context["latest_model_scan"]["models"][0]["signal_status"],"nonpositive_record");
        let zero_id="breadth22_h20";let zero=json!({"model_id":zero_id,"as_of":"2026-09-30","runner_sha256":"runner","model_sha256":registry["files"]["breadth22_h20_model2026"]["sha256"],"production_admission":false,"threshold":0.0,"current_scores":[{"symbol":"sz000001","as_of":"2026-09-30","score":0.02}],"candidates":[],"job_id":2});let mut zero_context=json!({"models":[]});apply_scan_scores(&mut zero_context,&[zero],"sz000001","2026-09-30","runner").unwrap();assert_eq!(zero_context["latest_model_scan"]["models"][0]["signal_status"],"positive_record");
        apply_scan_scores(&mut context,&[group.clone()],"sz000002","2026-09-30","runner").unwrap();assert!(context["latest_model_scan"]["models"][0]["score"].is_null());
        apply_scan_scores(&mut context,&[group.clone()],"sz000001","2026-09-29","runner").unwrap();assert_eq!(context["latest_model_scan"]["models"],json!([]));
        group["model_sha256"]=json!("different-artifact");
        assert_ne!(group["model_sha256"],*sha);apply_scan_scores(&mut context,&[group],"sz000001","2026-09-30","runner").unwrap();assert_eq!(context["latest_model_scan"]["models"],json!([]));
    }
    #[test]
    fn tamper_and_self_admission_are_rejected(){
        let envelope:Value=serde_json::from_str(include_str!("../../research-evidence.json")).unwrap();
        let mut content:Value=serde_json::from_str(envelope["content"].as_str().unwrap()).unwrap();
        let wrap=|v:&Value|{let text=v.to_string();json!({"schema":"research-evidence-bundle-v1","content_sha256":hex::encode(Sha256::digest(text.as_bytes())),"content":text}).to_string()};
        assert!(decode(&wrap(&content)).is_ok());
        let mut corrupt:Value=serde_json::from_str(&wrap(&content)).unwrap();corrupt["content"]="{}".into();assert!(decode(&corrupt.to_string()).is_err());
        content["production_admission"]=true.into();assert!(decode(&wrap(&content)).is_err());
        content["production_admission"]=false.into();content["models"][0]["test"]["max_drawdown_pct"]=101.into();assert!(decode(&wrap(&content)).is_err());
        content["models"][0]["test"]["max_drawdown_pct"]=2.into();content.as_object_mut().unwrap().remove("data");assert!(decode(&wrap(&content)).is_err());
    }
    #[test]
    fn model_observations_reject_tamper_scope_and_admission(){
        let raw=include_str!("../../model-screen.json");let mut screen=decode_screen(raw).unwrap();
        assert_eq!(screen["models"][0]["positive_count"],7);assert_eq!(screen["models"][1]["positive_count"],0);
        let mut evidence=decode(include_str!("../../research-evidence.json")).unwrap();assert!(screen_evidence_matches(&screen,&evidence).is_ok());
        evidence["content_sha256"]=json!("changed-while-ids-and-snapshot-stay-same");assert!(screen_evidence_matches(&screen,&evidence).is_err());
        let wrap=|v:&Value|{let text=v.to_string();json!({"schema":"model-screen-bundle-v1","content_sha256":hex::encode(Sha256::digest(text.as_bytes())),"content":text}).to_string()};
        screen["models"][0]["candidates"][0]["symbol"]=json!("bj920001");assert!(decode_screen(&wrap(&screen)).is_err());
        screen=decode_screen(raw).unwrap();screen["production_admission"]=json!(true);assert!(decode_screen(&wrap(&screen)).is_err());
        screen=decode_screen(raw).unwrap();screen["models"][0]["candidates"][0]["name"]=json!("*ST测试");assert!(decode_screen(&wrap(&screen)).is_err());
        let mut broken:Value=serde_json::from_str(raw).unwrap();broken["content"]=json!("{}");assert!(decode_screen(&broken.to_string()).is_err());
    }
    #[test]
    fn individual_context_never_invents_scores_or_reuses_a_different_date(){
        let screen=decode_screen(include_str!("../../model-screen.json")).unwrap();
        let evidence=decode(include_str!("../../research-evidence.json")).unwrap();
        let symbol=screen["models"][0]["candidates"][0]["symbol"].as_str().unwrap();
        let day=screen["as_of"].as_str().unwrap();
        let matched=frozen_context(&screen,&evidence,Some(symbol),day).unwrap();
        assert_eq!(matched["models"][0]["signal_status"],"positive_record");
        assert!(matched["models"][0]["score"].as_f64().unwrap()>0.0);
        assert_eq!(matched["models"][1]["signal_status"],"nonpositive_record");
        assert!(matched["models"][1]["score"].as_f64().unwrap()<=0.0);
        assert_eq!(matched["production_admission"],false);
        let stale=frozen_context(&screen,&evidence,Some(symbol),"2026-09-29").unwrap();
        assert_eq!(stale["models"][0]["signal_status"],"date_mismatch");
        assert!(stale["models"][0]["score"].is_null()&&stale["models"][0]["observation"].is_null());
        assert!(stale["models"][0]["performance"]["test"].is_object());
        assert!(frozen_context(&screen,&evidence,Some("bj920001"),day).is_err());
        let mut legacy=screen.clone();for model in legacy["models"].as_array_mut().unwrap(){model.as_object_mut().unwrap().remove("all_scores");}
        let legacy=frozen_context(&legacy,&evidence,Some(symbol),day).unwrap();assert_eq!(legacy["models"][1]["signal_status"],"not_in_positive_export");assert!(legacy["models"][1]["score"].is_null());
        let mut incomplete=screen.clone();incomplete["models"][0]["all_scores"]=json!([]);let text=incomplete.to_string();let raw=json!({"schema":"model-screen-bundle-v1","content":text,"content_sha256":hex::encode(Sha256::digest(text.as_bytes()))}).to_string();assert!(decode_screen(&raw).is_err());
        let unknown=frozen_context(&screen,&evidence,Some("sh609999"),day).unwrap();assert_eq!(unknown["models"][0]["signal_status"],"not_in_scored_export");assert!(unknown["models"][0]["score"].is_null());
        let wrap=|v:&Value|{let text=v.to_string();json!({"schema":"model-screen-bundle-v1","content_sha256":hex::encode(Sha256::digest(text.as_bytes())),"content":text}).to_string()};
        let mut duplicate=screen.clone();duplicate["models"][0]["all_scores"][1]=duplicate["models"][0]["all_scores"][0].clone();assert!(decode_screen(&wrap(&duplicate)).is_err());
        let mut altered=screen.clone();altered["models"][0]["candidates"][0]["score"]=json!(42.0);assert!(decode_screen(&wrap(&altered)).is_err());
    }
    #[tokio::test]
    #[ignore="read-only current loopback StockDB revalidation of seven frozen model observations"]
    async fn live_frozen_model_candidate_revalidation(){
        let root=std::env::temp_dir().join(format!("bull-model-screen-{}",uuid::Uuid::new_v4()));
        let db=Database::open(root.clone()).unwrap();db.set_setting("local_history_enabled","1").unwrap();
        let value=model_screen_view(&db).await.unwrap();assert_eq!(value["fresh"],true);assert_eq!(value["revalidation_complete"],true);
        println!("{}",value);drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}
