//! Model research has its own ledger: never maps a learned model to a legacy rule.
use super::Database;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn decode_run(raw: &str) -> Result<Value, String> {
    if raw.len() > 32 * 1024 * 1024 { return Err("模型账本超过32 MiB".into()); }
    let envelope: Value = serde_json::from_str(raw).map_err(|_| "账本JSON无效")?;
    let text = envelope["content"].as_str().ok_or("缺少账本正文")?;
    let hash = hex::encode(Sha256::digest(text.as_bytes()));
    if envelope["schema"] != "model-run-bundle-v1" || envelope["content_sha256"].as_str() != Some(&hash) {
        return Err("模型账本正文指纹不符".into());
    }
    let mut v: Value = serde_json::from_str(text).map_err(|_| "账本正文无效")?;
    if v["schema"] != "ashare-model-run-v1" || v["production_admission"] != false || v["exploration"] != true
        || v["training_refitted"] != false || v["frozen_prefix_preserved"] != true || v["initial_cash_cny"] != 100000 {
        return Err("只允许未准入、无重训且历史前缀保持的10万元模型研究".into());
    }
    validate_task(&v)?;
    if !matches!(v["mode"].as_str(),Some("replay"|"forward")) || v["label_holding_days"] != 20 {
        return Err("账本模式或二十日训练标签不符".into());
    }
    let day = chrono::NaiveDate::parse_from_str(v["as_of"].as_str().ok_or("缺截止日")?,"%Y-%m-%d").map_err(|_|"日期无效")?;
    if day > chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap()).date_naive() {
        return Err("账本日期在未来".into());
    }
    let valid_sha=|s:&Value|s.as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|c|c.is_ascii_hexdigit()));
    for key in ["model_sha256","score_cache_sha256","runner_sha256"] {if !valid_sha(&v[key]) {return Err("模型或程序指纹缺失".into());}}
    for key in ["input_sha256","data_sha256"] {if v[key].as_object().is_none_or(|m|m.is_empty()||m.values().any(|s|!valid_sha(s))){return Err("数据来源指纹不完整".into());}}
    if v["limitations"].as_array().is_none_or(|a|a.is_empty()||a.iter().any(|s|s.as_str().is_none())) {return Err("账本局限说明缺失".into());}
    let curve=v["ledger"]["curve"].as_array().ok_or("缺净值曲线")?;
    if curve.is_empty() {return Err("没有真实净值记录".into());}
    let mut prior=0i64;let mut peak=100000f64;let mut max_dd=0f64;
    for row in curve {
        let date=row["date"].as_i64().ok_or("净值日期无效")?;
        if date<=prior {return Err("净值日期不是严格递增".into());} prior=date;
        for key in ["equity","cash","dividend_receivable"] {if row[key].as_f64().is_none_or(|n|!n.is_finite()||n < -0.01){return Err("现金/净值无效".into());}}
        chrono::NaiveDate::parse_from_str(&date.to_string(),"%Y%m%d").map_err(|_|"净值市场日期无效")?;
        let equity=row["equity"].as_f64().unwrap();peak=peak.max(equity);max_dd=max_dd.max(1.-equity/peak);
    }
    if prior.to_string()!=day.format("%Y%m%d").to_string() {return Err("曲线截止日与快照日期不同".into());}
    let ordinary=|s:&str|s.len()==6&&s.is_ascii()&&s.bytes().all(|c|c.is_ascii_digit())
        && ["000","001","002","003","300","301","600","601","603","605","688","689"].iter().any(|p|s.starts_with(p));
    let orders=v["ledger"]["orders"].as_array().ok_or("缺订单账本")?;
    for row in orders {
        if row["code"].as_str().is_none_or(|s|!ordinary(s))||!matches!(row["side"].as_str(),Some("buy"|"sell"))
            ||row["qty"].as_i64().is_none_or(|n|n<=0)||row["price"].as_f64().is_none_or(|n|!n.is_finite()||n<=0.)
            ||row["fee"].as_f64().is_none_or(|n|!n.is_finite()||n<0.) {return Err("订单含非法股票/方向/数值".into());}
        if row["date"].as_i64().is_none_or(|d|d>prior) {return Err("订单发生在曲线截止日之后".into());}
    }
    let cycles=v["ledger"]["cycles"].as_array().ok_or("缺完整持仓周期")?;
    let unclosed=v["ledger"]["unclosed"].as_array().ok_or("缺未平持仓")?;
    let mut pnl=0.;
    for row in cycles {if row["code"].as_str().is_none_or(|s|!ordinary(s))||row["holding_sessions"].as_u64().is_none_or(|n|n==0) {return Err("完整周期含北交所/非法股票或持有期".into());}pnl+=row["net_pnl"].as_f64().filter(|x|x.is_finite()).ok_or("周期损益无效")?;}
    for row in unclosed {if row["code"].as_str().is_none_or(|s|!ordinary(s))||row["qty"].as_i64().is_none_or(|n|n<=0)||row["bonus_locked"].as_i64().is_none_or(|n|n<0||n>row["qty"].as_i64().unwrap()) {return Err("未平持仓股票、股数或锁定权益无效".into());}pnl+=row["unrealized_and_receivable_pnl"].as_f64().filter(|x|x.is_finite()).ok_or("未平损益无效")?;}
    if (curve.last().unwrap()["equity"].as_f64().unwrap()-100000.-pnl).abs()>0.1 {return Err("完成/未平损益与净值不守恒".into());}
    let metrics=&v["ledger"]["metrics"];
    if metrics["completed_holding_cycles"].as_u64()!=Some(cycles.len() as u64) {return Err("周期统计与账本不符".into());}
    let net=(curve.last().unwrap()["equity"].as_f64().unwrap()/100000.-1.)*100.;
    if metrics["net_return_pct"].as_f64().is_none_or(|x|!x.is_finite()||(x-net).abs()>0.002) {return Err("账户收益与净值不符".into());}
    if metrics["max_drawdown_pct"].as_f64().is_none_or(|x|!x.is_finite()||(x-max_dd*100.).abs()>0.002) {return Err("组合回撤与真实净值不符".into());}
    if !cycles.is_empty(){let win=cycles.iter().filter(|r|r["net_pnl"].as_f64().unwrap()>0.).count() as f64/cycles.len() as f64*100.;if metrics["win_rate_pct"].as_f64().is_none_or(|x|(x-win).abs()>0.011){return Err("胜率没有按完整持仓周期统计".into());}}
    else if !metrics["win_rate_pct"].is_null(){return Err("零完整周期不能填胜率".into());}
    if v["checks"]["ST_BJ_buys"]!=0 || v["accounting"]["minimum_cash_cny"].as_f64().is_none_or(|x|x<0.) {return Err("买单排除/现金核验失败".into());}
    for watch in v["signal_watch"].as_array().ok_or("缺模型观察信号")? {
        let symbol=watch["symbol"].as_str().ok_or("观察代码缺失")?;
        if symbol.len()!=8||!symbol.is_ascii()||!ordinary(&symbol[2..])
            ||!(symbol.starts_with("sh6")||symbol.starts_with("sz0")||symbol.starts_with("sz3"))||watch["as_of"]!=v["as_of"]
            ||watch["threshold"]!=v["signal_threshold"]
            ||watch["score"].as_f64().is_none_or(|n|!n.is_finite()||n<=v["signal_threshold"].as_f64().unwrap_or(f64::INFINITY))||watch["signal_eligible"]!=true {return Err("模型观察信号无效".into());}
    }
    if let Some(scores)=v.get("current_scores") {
        let rows=scores.as_array().filter(|r|r.len()<=10000).ok_or("当日完整评分结构无效")?;
        let mut symbols=std::collections::HashSet::new();let mut positive=std::collections::HashSet::new();
        for row in rows {
            let symbol=row["symbol"].as_str().ok_or("完整评分缺股票")?;
            if symbol.len()!=8||!symbol.is_ascii()||!ordinary(&symbol[2..])||!(symbol.starts_with("sh6")||symbol.starts_with("sz0")||symbol.starts_with("sz3"))
                ||!symbols.insert(symbol)||row["as_of"]!=v["as_of"]||row["threshold"]!=v["signal_threshold"] {return Err("完整评分股票、日期或阈值不符".into());}
            let score=row["score"].as_f64().filter(|n|n.is_finite()).ok_or("完整评分数值无效")?;
            if score>v["signal_threshold"].as_f64().ok_or("模型阈值无效")? {positive.insert(symbol);}
        }
        let watched:std::collections::HashSet<_>=v["signal_watch"].as_array().unwrap().iter().filter_map(|r|r["symbol"].as_str()).collect();
        if positive!=watched {return Err("完整评分通过阈值的股票与观察名单不同".into());}
        for watch in v["signal_watch"].as_array().unwrap(){let matching=rows.iter().find(|r|r["symbol"]==watch["symbol"]).unwrap();if matching["score"]!=watch["score"]{return Err("完整评分与候选分值不同".into());}}
    }
    v["content_sha256"]=hash.into();
    Ok(v)
}

pub fn validate_task(v:&Value)->Result<(),String>{
    if v["model_id"]=="fundamental37_h20"{return if v["holding_days"]==20&&v["comparison"]=="baseline"&&v.get("mode").is_none_or(|m|m=="replay"){Ok(())}else{Err("财务37只允许原20日历史回放，未知前向财务不能开账户".into())};}
    if !matches!(v["model_id"].as_str(),Some("breadth22_h20"|"index26_h20"|"breadth22_excess_csi20"|"breadth22_rank20"|"breadth22_open_downside20")) {return Err("未知冻结模型，禁止替代旧规则".into());}
    match v["comparison"].as_str(){Some("holding15") if v["holding_days"]==15=>Ok(()),Some("baseline"|"cost_double"|"staged"|"verified_actions") if v["holding_days"]==20=>Ok(()),_=>Err("模型执行期限/对照不符，十五日不是重新训练模型".into())}
}

pub(super) fn save_model_run_checked(conn:&rusqlite::Connection,raw:&str,continuation:Option<i64>,expected_sha:Option<&str>)->Result<i64,String>{
        let v=decode_run(raw)?;
        let mechanism=json!({"model":v["model_id"],"hold":v["holding_days"],"comparison":v["comparison"],"mode":v["mode"],"start":v["forward_start"],"source":v["input_sha256"],"data":v["data_sha256"],"runner":v["runner_sha256"]});
        let identity=hex::encode(Sha256::digest(mechanism.to_string().as_bytes()));
        if let Some(id)=continuation {
            let old_raw:String=conn.query_row("SELECT bundle_json FROM model_research_runs WHERE id=?1",[id],|r|r.get(0)).map_err(|_|"前向账户不存在")?;
            let old=decode_run(&old_raw)?;
            if expected_sha.is_some_and(|sha|old["content_sha256"].as_str()!=Some(sha)){return Err("前向账户在作业期间改变，拒绝覆盖".into());}
            for key in ["model_id","holding_days","comparison","mode","forward_start","input_sha256","model_sha256","score_cache_sha256","runner_sha256"] {if old[key]!=v[key] {return Err("不能更改前向账户的冻结身份/费用/起点".into());}}
            if v["mode"]!="forward"||v["as_of"].as_str()<old["as_of"].as_str(){return Err("只能向前延续观察".into());}
            for key in ["curve","orders","cycles"] {
                let before=old["ledger"][key].as_array().unwrap();let after=v["ledger"][key].as_array().unwrap();
                if after.len()<before.len()||after[..before.len()]!=before[..] {return Err("增量重放改变旧账户账本，暂停更新".into());}
            }
            conn.execute("UPDATE model_research_runs SET identity=?1,bundle_json=?2,updated_at=?3 WHERE id=?4",params![identity,raw,chrono::Utc::now().to_rfc3339(),id]).map_err(|e|e.to_string())?;return Ok(id);
        }
        conn.execute("INSERT OR IGNORE INTO model_research_runs(identity,bundle_json,created_at,updated_at) VALUES(?1,?2,?3,?3)",params![identity,raw,chrono::Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        conn.query_row("SELECT id FROM model_research_runs WHERE identity=?1",[identity],|r|r.get(0)).map_err(|e|e.to_string())
}

impl Database {
    pub fn migrate_model_research(&self)->rusqlite::Result<()> {
        self.conn.lock().unwrap_or_else(|e|e.into_inner()).execute_batch("CREATE TABLE IF NOT EXISTS model_research_tasks(id INTEGER PRIMARY KEY,identity TEXT NOT NULL UNIQUE,task_json TEXT NOT NULL,created_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS model_research_runs(id INTEGER PRIMARY KEY,identity TEXT NOT NULL UNIQUE,bundle_json TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 0,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);")
    }
    pub fn register_model_task(&self,value:&Value)->Result<i64,String>{
        validate_task(value)?;
        if value.as_object().is_none_or(|m|m.keys().any(|k|!["name","hypothesis","model_id","holding_days","comparison","invalidation","missing_data","frozen_source"].contains(&k.as_str()))) {return Err("任务书含未知字段；禁止导入脚本或旧规则".into());}
        if value["name"].as_str().is_none_or(|s|s.trim().is_empty()||s.chars().count()>40)
            ||value["hypothesis"].as_str().is_none_or(|s|!(20..=800).contains(&s.chars().count())) {return Err("研究任务名称/假说不完整".into());}
        for key in ["invalidation","missing_data"] {if value[key].as_array().is_none_or(|a|a.len()>12||a.iter().any(|v|v.as_str().is_none_or(|s|s.chars().count()>200))){return Err("任务反证/缺失项无效".into());}}
        let mechanism=json!({"model":value["model_id"],"hold":value["holding_days"],"comparison":value["comparison"],"source":value["frozen_source"]});
        let identity=hex::encode(Sha256::digest(mechanism.to_string().as_bytes()));
        let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());
        conn.execute("INSERT OR IGNORE INTO model_research_tasks(identity,task_json,created_at) VALUES(?1,?2,?3)",params![identity,value.to_string(),chrono::Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        conn.query_row("SELECT id FROM model_research_tasks WHERE identity=?1",[identity],|r|r.get(0)).map_err(|e|e.to_string())
    }
    pub fn model_tasks(&self)->Result<Value,String>{
        let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());
        let mut stmt=conn.prepare("SELECT id,task_json,created_at FROM model_research_tasks ORDER BY id DESC LIMIT 100").map_err(|e|e.to_string())?;
        let rows=stmt.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
        rows.map(|row|{let(id,raw,date)=row.map_err(|e|e.to_string())?;let mut v:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;v["id"]=id.into();v["created_at"]=date.into();Ok(v)}).collect::<Result<Vec<_>,String>>().map(Value::Array)
    }
    pub fn save_model_run(&self,raw:&str,continuation:Option<i64>)->Result<i64,String>{
        let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());save_model_run_checked(&conn,raw,continuation,None)
    }
    pub fn model_runs(&self)->Result<Value,String>{
        let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());
        let mut stmt=conn.prepare("SELECT id,bundle_json,enabled,updated_at FROM model_research_runs WHERE enabled=1 OR id IN (SELECT id FROM model_research_runs ORDER BY id DESC LIMIT 30) ORDER BY id DESC").map_err(|e|e.to_string())?;
        let rows=stmt.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,r.get::<_,String>(3)?))).map_err(|e|e.to_string())?;
        rows.map(|row|{let(id,raw,on,date)=row.map_err(|e|e.to_string())?;let mut v=decode_run(&raw)?;v["id"]=id.into();v["enabled"]=on.into();v["updated_at"]=date.into();Ok(v)}).collect::<Result<Vec<_>,String>>().map(Value::Array)
    }
    pub fn model_run_bundle(&self,id:i64)->Result<String,String>{self.conn.lock().unwrap_or_else(|e|e.into_inner()).query_row("SELECT bundle_json FROM model_research_runs WHERE id=?1",[id],|r|r.get(0)).map_err(|e|e.to_string())}
    pub fn model_run(&self,id:i64)->Result<Value,String>{
        let (raw,on,updated):(String,bool,String)=self.conn.lock().unwrap_or_else(|e|e.into_inner()).query_row("SELECT bundle_json,enabled,updated_at FROM model_research_runs WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"模型账户不存在")?;
        let mut value=decode_run(&raw)?;value["id"]=id.into();value["enabled"]=on.into();value["updated_at"]=updated.into();Ok(value)
    }
    pub fn enabled_model_runs(&self)->Result<Vec<Value>,String>{
        let ids:Vec<i64>={let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());let mut stmt=conn.prepare("SELECT id FROM model_research_runs WHERE enabled=1 OR id IN (SELECT source_run_id FROM model_follow_accounts WHERE json_extract(binding_json,'$.enabled')=1) ORDER BY id").map_err(|e|e.to_string())?;let rows=stmt.query_map([],|r|r.get(0)).map_err(|e|e.to_string())?;rows.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?};
        ids.into_iter().map(|id|self.model_run(id)).collect()
    }
    pub fn has_model_replay(&self,task:&Value,input:&Value,runner_sha:&str)->Result<bool,String>{
        let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());
        let mut stmt=conn.prepare("SELECT json_extract(json_extract(bundle_json,'$.content'),'$.input_sha256') FROM model_research_runs WHERE json_extract(json_extract(bundle_json,'$.content'),'$.mode')='replay' AND json_extract(json_extract(bundle_json,'$.content'),'$.model_id')=?1 AND json_extract(json_extract(bundle_json,'$.content'),'$.holding_days')=?2 AND json_extract(json_extract(bundle_json,'$.content'),'$.comparison')=?3 AND json_extract(json_extract(bundle_json,'$.content'),'$.runner_sha256')=?4").map_err(|e|e.to_string())?;
        let rows=stmt.query_map(params![task["model_id"].as_str(),task["holding_days"].as_i64(),task["comparison"].as_str(),runner_sha],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?;
        for row in rows {let actual:Value=serde_json::from_str(&row.map_err(|e|e.to_string())?).map_err(|_|"已保存回放来源无效")?;if &actual==input{return Ok(true);}}
        Ok(false)
    }
    pub fn enable_model_observation(&self,id:i64,on:bool)->Result<(),String>{
        let conn=self.conn.lock().unwrap_or_else(|e|e.into_inner());
        let raw:Option<String>=conn.query_row("SELECT bundle_json FROM model_research_runs WHERE id=?1",[id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if decode_run(&raw.ok_or("模型观察不存在")?)?["mode"]!="forward" {return Err("只有前向账户可开启继续观察，历史回放不能当实时账户".into());}
        if !on&&conn.query_row("SELECT EXISTS(SELECT 1 FROM model_follow_accounts WHERE source_run_id=?1 AND json_extract(binding_json,'$.enabled')=1)",[id],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())?{return Err("此模型正在供自动跟随使用，不能从手动观察关闭其日线维护；手动股票观察可单独暂停".into());}
        conn.execute("UPDATE model_research_runs SET enabled=?1 WHERE id=?2",params![on,id]).map_err(|e|e.to_string())?;Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn learned_model_is_not_old_rule(){assert!(validate_task(&json!({"model_id":"trend_follow","holding_days":20,"comparison":"baseline"})).is_err());assert!(validate_task(&json!({"model_id":"breadth22_h20","holding_days":15,"comparison":"baseline"})).is_err());assert!(validate_task(&json!({"model_id":"breadth22_h20","holding_days":15,"comparison":"holding15"})).is_ok());}
    #[test]fn model_task_mechanism_dedup_ignores_name(){let root=std::env::temp_dir().join(format!("model-task-{}",uuid::Uuid::new_v4()));let db=Database::open(root.clone()).unwrap();let mut task=json!({"name":"固定研究","hypothesis":"这是足够长且可以反证的真实模型期限假说，不能声称已经获准交易。","model_id":"breadth22_h20","holding_days":15,"comparison":"holding15","invalidation":["成本后无改善"],"missing_data":[],"frozen_source":{"snapshot":"test"}});let first=db.register_model_task(&task).unwrap();task["name"]="另一个名字".into();assert_eq!(first,db.register_model_task(&task).unwrap());drop(db);std::fs::remove_dir_all(root).unwrap();}
    #[test]fn enabled_forward_account_survives_recent_history_window(){
        let root=std::env::temp_dir().join(format!("model-history-{}",uuid::Uuid::new_v4()));let db=Database::open(root.clone()).unwrap();
        let raw=include_str!("../../../research/research-center-runner/checks/verified-rank-forward-c0ae27eb-45f7-4dd6-b678-1fe478201281.json");let id=db.save_model_run(raw,None).unwrap();
        // Database lookup fixture only; this all-cash row is not a market performance result.
        let mut replay=decode_run(raw).unwrap();replay.as_object_mut().unwrap().remove("content_sha256");replay["mode"]="replay".into();replay["forward_start"]=Value::Null;
        let body=replay.to_string();let fixture=json!({"schema":"model-run-bundle-v1","content":body,"content_sha256":hex::encode(Sha256::digest(body.as_bytes()))}).to_string();let replay_id=db.save_model_run(&fixture,None).unwrap();
        {let conn=db.conn.lock().unwrap_or_else(|e|e.into_inner());for n in 0..35{conn.execute("INSERT INTO model_research_runs(identity,bundle_json,created_at,updated_at) VALUES(?1,?2,'test','test')",params![format!("history-only-{n}"),raw]).unwrap();}}
        assert_eq!(db.model_runs().unwrap().as_array().unwrap().len(),30);assert_eq!(db.model_run(id).unwrap()["id"],id);db.enable_model_observation(id,true).unwrap();
        assert!(!db.model_runs().unwrap().as_array().unwrap().iter().any(|v|v["id"]==replay_id));
        assert!(db.has_model_replay(&replay,&replay["input_sha256"],replay["runner_sha256"].as_str().unwrap()).unwrap());
        let mut other_input=replay["input_sha256"].clone();other_input["snapshot"]="0".repeat(64).into();assert!(!db.has_model_replay(&replay,&other_input,replay["runner_sha256"].as_str().unwrap()).unwrap());
        let enabled=db.enabled_model_runs().unwrap();assert_eq!(enabled.len(),1);assert_eq!(enabled[0]["id"],id);assert!(db.model_runs().unwrap().as_array().unwrap().iter().any(|v|v["id"]==id));
        db.enable_model_observation(id,false).unwrap();assert!(db.enabled_model_runs().unwrap().is_empty());drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}
