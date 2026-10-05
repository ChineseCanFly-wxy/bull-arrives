//! Official SWS Research first-level industries. SW codes are independent of BK concepts.
//! Constituent retrieval observes the current publisher list; it does not certify historical membership.
use super::sector::{SectorCatalogItem, SectorHistory, SectorKind, SectorKline, SectorMember, SectorMemberPage};
use chrono::{Datelike, NaiveDate, Utc, Weekday};
use serde_json::Value;
use std::{collections::HashSet, time::Duration,sync::OnceLock};

const API: &str = "https://www.swsresearch.com/institute-sw/api/index_publish/";

fn issuer_code(code: &str) -> Result<&str, String> {
    let raw = code.strip_prefix("SW").ok_or("申万行业代码必须使用独立的SW前缀")?;
    if raw.len() != 6 || !raw.starts_with("801") || !raw.bytes().all(|v| v.is_ascii_digit()) {
        return Err("申万行业代码必须是SW801加三位数字，不能猜测BK映射".into());
    }
    Ok(raw)
}

async fn request(path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    // Same public headers used by the recorded issuer probe; no authentication or quote-host substitution.
    static CLIENT:OnceLock<reqwest::Client>=OnceLock::new();
    // Windows certificate verification resolves the publisher's intermediate chain.
    // Keep verification enabled; never accept invalid certificates to make a probe pass.
    let client=CLIENT.get_or_init(||{
        let builder=reqwest::Client::builder().no_proxy().http1_only();
        #[cfg(target_os="windows")]
        let builder=builder.use_native_tls();
        builder.build().expect("SWS HTTP client")
    });
    let response = client.get(format!("{API}{path}"))
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0")
        .header(reqwest::header::REFERER, "https://quote.eastmoney.com/")
        .header(reqwest::header::ACCEPT, "application/json")
        .query(params).timeout(Duration::from_secs(12)).send().await
        .map_err(|e| format!("申万官方请求失败：{e:?}"))?
        .error_for_status().map_err(|e| format!("申万官方HTTP失败：{e}"))?;
    response.json().await.map_err(|e| format!("申万官方JSON无效：{e}"))
}

fn data(payload: &Value) -> Result<&Value, String> {
    if payload["code"].as_str() != Some("200") && payload["code"].as_i64() != Some(200) {
        return Err(format!("申万官方返回失败状态：{}", payload["message"]));
    }
    payload.get("data").filter(|v| !v.is_null()).ok_or_else(|| "申万官方数据缺失".into())
}

fn complete_rows(payload: &Value, max: usize) -> Result<&Vec<Value>, String> {
    let d = data(payload)?;
    let count = d["count"].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("申万当前名单缺少总数")?;
    let rows = d["results"].as_array().ok_or("申万当前名单缺少results")?;
    if count == 0 || count > max || rows.len() != count || d.get("next") != Some(&Value::Null)
        || d.get("previous") != Some(&Value::Null) {
        return Err("申万当前名单不完整：总数、分页或返回行数不一致".into());
    }
    Ok(rows)
}

fn required_text<'a>(row: &'a Value, key: &str) -> Result<&'a str, String> {
    row[key].as_str().filter(|v| !v.trim().is_empty() && *v == v.trim())
        .ok_or_else(|| format!("申万字段{key}缺失或无效"))
}

fn parse_catalog(payload: &Value) -> Result<Vec<SectorCatalogItem>, String> {
    let rows = complete_rows(payload, 100)?;
    if rows.len() != 31 { return Err("申万一级行业目录预期31个，发布者分类版本变化需核验".into()); }
    let mut seen = HashSet::new();
    rows.iter().map(|row| {
        let code = format!("SW{}", required_text(row, "swindexcode")?);
        issuer_code(&code)?;
        if !seen.insert(code.clone()) { return Err("申万一级行业代码重复".into()); }
        Ok(SectorCatalogItem { kind: SectorKind::Industry, code, name: required_text(row, "swindexname")?.into() })
    }).collect()
}

pub async fn fetch_catalog() -> Result<Vec<SectorCatalogItem>, String> {
    parse_catalog(&request("current/", &[("page", "1"), ("page_size", "100"), ("indextype", "一级行业")]).await?)
}

fn number(row: &Value, key: &str) -> Result<f64, String> {
    row[key].as_f64().or_else(|| row[key].as_str().and_then(|v| v.parse().ok()))
        .filter(|n| n.is_finite()).ok_or_else(|| format!("申万日线{key}不是有限数值"))
}

fn parse_history(code: &str, payload: &Value) -> Result<SectorHistory, String> {
    let issuer = issuer_code(code)?;
    let rows = data(payload)?.as_array().filter(|v| !v.is_empty()).ok_or("申万官方日线为空或结构无效")?;
    let mut dates = HashSet::new();
    let mut dated = Vec::with_capacity(rows.len());
    for row in rows {
        if required_text(row, "swindexcode")? != issuer { return Err("申万日线代码与请求不一致".into()); }
        let date = required_text(row, "bargaindate")?;
        let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "申万日线日期无效")?;
        if parsed.format("%Y-%m-%d").to_string() != date || !dates.insert(parsed) {
            return Err("申万日线包含无效或重复日期，拒绝去重后计算".into());
        }
        dated.push((parsed, row));
    }
    // The issuer returns full history. Order and validate exactly the last180 source observations.
    dated.sort_by_key(|(date, _)| *date);
    let mut items = Vec::with_capacity(dated.len().min(180));
    for (date, row) in dated.into_iter().rev().take(180).rev() {
        let open = number(row, "openindex")?; let close = number(row, "closeindex")?;
        let high = number(row, "maxindex")?; let low = number(row, "minindex")?;
        // Preserve publisher fields/units; no unverified conversion to shares or CNY.
        let volume = number(row, "bargainamount")?; let amount = number(row, "bargainsum")?;
        if [open, close, high, low].iter().any(|v| *v <= 0.0) || high < open.max(close)
            || low > open.min(close) || low > high || volume < 0.0 || amount < 0.0
            || matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            return Err(format!("申万日线{date}存在无效OHLC、负成交量额或周末日期"));
        }
        items.push(SectorKline { date: date.to_string(), open, close, high, low, volume, amount,
            change_pct: Some(number(row, "markup")?), turnover_rate: None });
    }
    Ok(SectorHistory { sector_code: code.into(), period: "daily".into(),
        as_of: items.last().ok_or("申万官方日线为空")?.date.clone(), items,
        source: format!("申万宏源研究官方 · {code} DAY · 成交量bargainamount/额bargainsum保留发布者原单位（未换算） · 抓取{}", Utc::now().to_rfc3339()) })
}

pub async fn fetch_history(code: &str) -> Result<SectorHistory, String> {
    let issuer = issuer_code(code)?;
    parse_history(code, &request("trend/", &[("swindexcode", issuer), ("period", "DAY")]).await?)
}

fn stock_market(code: &str) -> Result<&'static str, String> {
    if code.len() != 6 || !code.bytes().all(|v| v.is_ascii_digit()) { return Err("申万成分股代码无效，不能补零猜测".into()); }
    match &code[..2] {
        "60" | "68" | "90" => Ok("sh"), "00" | "30" | "20" => Ok("sz"),
        "43" | "83" | "87" | "92" => Ok("bj"),
        _ => Err(format!("申万成分股{code}交易所代码规则未知，停止完整名单")),
    }
}

fn parse_members(code: &str, payload: &Value, observed_at: String) -> Result<SectorMemberPage, String> {
    issuer_code(code)?;
    let rows = complete_rows(payload, 10000)?;
    let mut seen = HashSet::new(); let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let code = required_text(row, "stockcode")?;
        if !seen.insert(code.to_owned()) { return Err("申万成分股重复，不能去重后声称完整".into()); }
        items.push(SectorMember { code: code.into(), market: stock_market(code)?.into(), name: required_text(row, "stockname")?.into(),
            price: None, change_amount: None, change_pct: None, amount: None, turnover_rate: None,
            market_cap: None, pe: None, pb: None, quoted_at_unix: None });
    }
    Ok(SectorMemberPage { kind: SectorKind::Industry, sector_code: code.into(), page: 1, page_size: items.len() as u32,
        total: items.len(), items, as_of: observed_at.clone(), stale: false,
        source: format!("申万宏源研究官方 · 当前发布者成分快照 · 抓取{observed_at}（不是行情报价或历史名单生效时间）") })
}

pub async fn fetch_members(code: &str) -> Result<SectorMemberPage, String> {
    let issuer = issuer_code(code)?;
    let payload = request("details/component_stocks/", &[("swindexcode", issuer), ("page", "1"), ("page_size", "10000")]).await?;
    parse_members(code, &payload, Utc::now().to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn publisher_fixture_preserves_codes_dates_and_complete_current_members() {
        let mut catalog = json!({"code":"200","data":{"count":31,"next":null,"previous":null,"results":[]}});
        catalog["data"]["results"] = (0..31).map(|n| json!({"swindexcode":format!("801{n:03}"),"swindexname":format!("行业{n}")})).collect();
        assert_eq!(parse_catalog(&catalog).unwrap().len(), 31);
        catalog["data"]["results"][1]["swindexcode"] = catalog["data"]["results"][0]["swindexcode"].clone();
        assert!(parse_catalog(&catalog).is_err());
        let mut history = json!({"code":"200","data":[{"swindexcode":"801150","bargaindate":"2026-09-30","openindex":7783.46,"maxindex":8023.37,"minindex":7770.06,"closeindex":7989.13,"markup":2.73,"bargainamount":77.96244102,"bargainsum":1382.89681894}]});
        let result = parse_history("SW801150", &history).unwrap();
        assert_eq!(result.items[0].date, "2026-09-30"); assert_eq!(result.items[0].volume, 77.96244102);
        assert!(parse_history("BK0816", &history).is_err());
        history["data"][0]["maxindex"] = json!(7700);
        assert!(parse_history("SW801150", &history).is_err());
        history["data"][0]["maxindex"] = json!(8023.37); history["data"][0]["bargaindate"] = json!("2026-09-26");
        assert!(parse_history("SW801150", &history).is_err());
        let mut members = json!({"code":"200","data":{"count":2,"next":null,"previous":null,"results":[{"stockcode":"000078","stockname":"ST海王","newweight":"0.0664","beginningdate":"2021-12-13T08:00:00+08:00"},{"stockcode":"600276","stockname":"恒瑞医药"}]}});
        let result = parse_members("SW801150", &members, "2026-10-01T00:00:00Z".into()).unwrap();
        assert_eq!(result.items.len(), 2); assert_eq!(result.items[0].market, "sz");
        assert!(result.items.iter().all(|m| m.quoted_at_unix.is_none() && m.price.is_none()));
        members["data"]["next"] = json!("page2"); assert!(parse_members("SW801150", &members, String::new()).is_err());
        members["data"]["next"] = Value::Null; members["data"]["results"][1]["stockcode"] = json!("000078");
        assert!(parse_members("SW801150", &members, String::new()).is_err());
    }

    #[tokio::test]
    #[ignore = "read-only live publisher HTTP; explicit opt-in"]
    async fn live_sw_catalog_history_and_one_complete_member_snapshot() {
        let catalog = fetch_catalog().await.unwrap(); assert_eq!(catalog.len(), 31);
        let history = fetch_history("SW801150").await.unwrap(); assert_eq!(history.items.len(), 180);
        let members = fetch_members("SW801150").await.unwrap(); assert_eq!(members.total, members.items.len());
        assert!(members.items.iter().all(|m| m.quoted_at_unix.is_none()));
        println!("SWS official catalog={} history={} {}..{} members={} source={}", catalog.len(), history.items.len(), history.items.first().unwrap().date, history.as_of, members.total, members.source);
    }
}
