//! Frozen research evidence is read-only; no self-reported performance grants admission.
use chrono::NaiveDate;
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::sync::OnceLock;

const STOCKS:&str=include_str!("../../../research/ashare-fundamental-2026-10-01/current-stock-evidence.json");
const SUMMARY:&str=include_str!("../../../research/ashare-fundamental-2026-10-01/research-summary-with-neighbors.json");
static FINANCIAL:OnceLock<Value>=OnceLock::new();

pub(crate) fn stock_financial_context(symbol:&str,as_of:&str)->Result<Value,String>{
    let day=NaiveDate::parse_from_str(as_of,"%Y-%m-%d").map_err(|_|"财务研究截止日无效")?;
    let source=FINANCIAL.get_or_init(||serde_json::from_str(STOCKS).expect("compiled financial evidence"));
    financial_row(source,symbol,day)
}
fn financial_row(source:&Value,symbol:&str,day:NaiveDate)->Result<Value,String>{
    let date=day.format("%Y%m%d").to_string().parse::<i64>().unwrap();
    let mut result=json!({"schema":"stock-financial-context-v1","symbol":symbol,"requested_as_of":day.to_string(),"snapshot_as_of":source["as_of"],"source":source["source"],"source_sha256":hex::encode(Sha256::digest(STOCKS.as_bytes())),"history_version_certified":false,"production_admission":false,"available":false,"limitations":["按公告、修订、入库最晚日延后使用；供应商完整历史版本链仍未知", "资产负债表没有独立修订时间；这里只记录原始累计报告值，不自行换算单季度", "财务37增量模型的执行邻域反证未通过，不能把财务较好转成买入许可"]});
    if source["schema"]!="conservative-financial-stock-evidence-v1"||source["history_version_certified"]!=false {return Err("财务证据格式或版本认证身份不符".into());}
    if source["as_of"].as_str()!=Some(day.to_string().as_str()) {result["message"]=json!("财务快照截止日不同，没有对应时点版本，不能用当前财务倒填历史或假装实时刷新。");return Ok(result);}
    let Some(row)=source["stocks"][symbol].as_array()else{result["message"]=json!("该股票未获得财务研究记录，未知不填零。");return Ok(result);};
    let columns=source["columns"].as_array().ok_or("财务字段清单缺失")?;
    if row.len()!=columns.len(){return Err("财务字段数量不符".into());}
    let mut fields=serde_json::Map::new();for (key,value) in columns.iter().zip(row){fields.insert(key.as_str().ok_or("财务字段名无效")?.to_owned(),value.clone());}
    let max_known=["first_notice_date","revision_date","provider_ingestion_date"].iter().filter_map(|k|fields[*k].as_i64()).max().ok_or("财务日期缺失")?;
    let available=fields["available_signal_date"].as_i64().ok_or("财务可用信号日缺失")?;
    if available<=max_known||available>date||fields["balance_latest_notice_date"].as_i64().is_some_and(|n|n>=date){return Err("财务日期穿越或尚未可用，禁止解释为截止日已知信息".into());}
    result["available"]=json!(true);result["fields"]=Value::Object(fields);
    result["message"]=json!("保守公告时点财务研究值；原始版本链未认证，成长与现金流需要结合行业和报告期解读。");Ok(result)
}

const EXTERNAL_STUDY: &str = include_str!("../../external-strategy-evidence.json");

fn external_studies(raw: &str) -> Result<Vec<Value>, String> {
    let evidence: Value = serde_json::from_str(raw).map_err(|_| "外部策略研究证据损坏")?;
    if evidence["schema"] != "external-strategy-study-evidence-v1"
        || evidence["production_admission"] != false
        || evidence["new_usable_models"] != 0
    {
        return Err("外部研究只展示未准入证据，状态不符".into());
    }
    NaiveDate::parse_from_str(evidence["as_of"].as_str().ok_or("外部研究缺数据日期")?, "%Y-%m-%d")
        .map_err(|_| "外部研究数据日期无效")?;
    let studies = evidence["studies"].as_array().ok_or("外部研究比较缺失")?;
    if studies.len() != 2 { return Err("外部研究证据数量不符".into()); }
    let fingerprint = hex::encode(Sha256::digest(raw.as_bytes()));
    let mut result = Vec::new();
    for study in studies {
        if study["new_usable_models"] != 0 || study["as_of"] != evidence["as_of"] {
            return Err("研究卡日期或可用模型状态不符".into());
        }
        if study["cash_accounts"].as_u64().is_none_or(|v| v == 0) {
            return Err("研究卡缺真实账户次数".into());
        }
        let comparisons = study["comparisons"].as_array().ok_or("研究卡缺对照结果")?;
        if comparisons.is_empty() { return Err("研究卡对照为空".into()); }
        for row in comparisons {
            if row["name"].as_str().is_none_or(|v| v.is_empty())
                || row["holding_days"].as_u64().is_none_or(|v| v == 0)
                || row["cycles"].as_u64().is_none()
                || row["unique_stocks"].as_u64().is_none()
            { return Err("研究卡配置身份或交易数量缺失".into()); }
            for key in ["net_return_pct", "max_drawdown_pct", "validation_return_pct", "tail_return_pct", "double_cost_return_pct"] {
                if row[key].as_f64().is_none_or(|v| !v.is_finite()) {
                    return Err(format!("研究卡指标{key}缺失，不以零替代"));
                }
            }
        }
        let mut row = study.clone();
        row["source_sha256"] = json!(fingerprint);
        result.push(row);
    }
    Ok(result)
}

#[tauri::command]
pub fn research_extension_report()->Result<Value,String>{
    let summary:Value=serde_json::from_str(SUMMARY).map_err(|_|"财务研究摘要损坏")?;
    let metrics=summary["models"].as_object().ok_or("财务对照缺失")?.iter().map(|(id,row)|json!({"name":match id.as_str(){"frozen22_full"=>"原22全池","frozen22_financial_pool"=>"原22财务覆盖池","refit22_financial_pool"=>"同覆盖重训22",_=>"财务增量37"},"net_return_pct":row["periods"]["test"]["net_return_pct"],"max_drawdown_pct":row["periods"]["test"]["max_drawdown_pct"],"cycles":row["periods"]["test"]["completed_cycles"]})).collect::<Vec<_>>();
    let mut report = json!({"studies":[{"title":"公告时点财务增量37","status":"已完成多年对照；执行稳健性未通过","message":"取得38季度全市场财报与资产负债表，新增15项财务输入。主20日配置有增量，15/25日期限早段转负，延迟一天后段优势消失；保留探索，不替换主模型。下表仅2025至2026-09-30已见后段，非年化。","metrics":metrics,"source_sha256":hex::encode(Sha256::digest(SUMMARY.as_bytes())),"limitations":["2018真实财务覆盖为零，2019约61%，没有倒填；2020至2025约90%至94%", "公告/修订/入库三日期最晚之后才用；完整历史修订链与资产负债表独立修订时间未认证", "十四次年度拟合、一百三十四份账户回放；所有后段已见，不作为全新样本", "当前财务模型正分4021只，表示广候选排序，不是同时买入名单"]},{"title":"真实历史更新与权益记账核验","status":"历史链路重放通过","message":"9月28日至30日真实行情追加、前向延续和同日静默检查通过。七份实施公告补证送转上市与现金支付日期；在模型模拟研究选择已核验权益日期对照，可复跑账本。","limitations":["历史重放不是下一交易日实盘验收，原生通知是否显示仍由系统决定", "权益核验是记账修正，不是新的预测算法；原结果保留，只有核验事件释放", "程序版本升级不改变旧账户身份；新版本需要新开账户"]}]});
    let studies = report["studies"].as_array_mut().ok_or("研究卡集合无效")?;
    studies.splice(0..0, external_studies(EXTERNAL_STUDY)?);
    Ok(report)
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn financial_snapshot_cannot_fill_another_date_or_early_publication(){
        let mut source:Value=serde_json::from_str(STOCKS).unwrap();let day=NaiveDate::from_ymd_opt(2026,9,30).unwrap();
        assert_eq!(financial_row(&source,"sz000001",day).unwrap()["available"],true);
        assert_eq!(financial_row(&source,"sz000001",day-chrono::Duration::days(1)).unwrap()["available"],false);
        source["stocks"]["sz000001"][4]=json!(20260815);assert!(financial_row(&source,"sz000001",day).is_err());
        assert_eq!(stock_financial_context("sz301068","2026-09-30").unwrap()["available"],true);
        let report = research_extension_report().unwrap();
        assert_eq!(report["studies"].as_array().unwrap().iter().filter(|row| matches!(row["title"].as_str(), Some("公告时点财务增量37" | "真实历史更新与权益记账核验"))).count(), 2);
    }
}

#[cfg(test)]
mod external_study_tests {
    use super::*;

    #[test]
    fn actual_external_evidence_preserves_failure_and_positive_subresult() {
        let studies = external_studies(EXTERNAL_STUDY).unwrap();
        assert_eq!(studies.len(), 2);
        assert_eq!(studies[0]["cash_accounts"], 704);
        assert_eq!(studies[0]["native_matcher_accounts"], 100);
        assert_eq!(studies[1]["cash_accounts"], 192);
        assert_eq!(studies[1]["new_usable_models"], 0);
        assert_eq!(studies[1]["comparisons"].as_array().unwrap().len(), 10);
        let positive = studies[1]["comparisons"].as_array().unwrap().last().unwrap();
        assert_eq!(positive["net_return_pct"], 56.123);
        assert_eq!(positive["validation_return_pct"], -0.626);
        assert!(positive["status"].as_str().unwrap().contains("未通过"));
        assert_eq!(studies[1]["source_sha256"].as_str().unwrap().len(), 64);
        let report = research_extension_report().unwrap();
        assert_eq!(report["studies"].as_array().unwrap().len(), 4);
        assert_eq!(report["studies"][0]["cash_accounts"], 704);
        assert_eq!(report["studies"][1]["cash_accounts"], 192);
        assert_eq!(report["studies"][2]["title"], "公告时点财务增量37");
    }

    #[test]
    fn external_evidence_never_fills_missing_metrics_or_grants_admission() {
        let original: Value = serde_json::from_str(EXTERNAL_STUDY).unwrap();
        let mut missing = original.clone();
        missing["studies"][1]["comparisons"][0]["validation_return_pct"] = Value::Null;
        assert!(external_studies(&missing.to_string()).unwrap_err().contains("不以零替代"));
        let mut promoted = original.clone();
        promoted["production_admission"] = json!(true);
        assert!(external_studies(&promoted.to_string()).is_err());
        let mut drifted = original;
        drifted["studies"][1]["as_of"] = json!("2026-10-03");
        assert!(external_studies(&drifted.to_string()).is_err());
    }
}
