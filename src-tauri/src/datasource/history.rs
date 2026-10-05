//! Local stockdb daily history adapter.

use crate::domain::KLineData;
use chrono::NaiveDate;
use reqwest::{header::CONTENT_TYPE, Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{net::IpAddr, time::Duration};

const SOURCE: &str = "local-stockdb";

#[derive(Debug, Clone)]
pub struct LocalHistoryConfig {
    pub base_url: String,
    pub timeout: Duration,
}

impl LocalHistoryConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalHistoryProtocol {
    Json,
    MessagePack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalHistoryResult {
    /// 前复权价格，用于信号与研究计算。
    pub klines: Vec<KLineData>,
    /// 未复权价格，用于模拟成交与现金账本。
    pub raw_klines: Vec<KLineData>,
    /// 原始日线的每日 ST 状态；缺字段保持 unknown，不用今天的名称倒推。
    pub st_by_date: Vec<(String, Option<bool>)>,
    pub source: String,
    pub protocol: LocalHistoryProtocol,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalMinuteBar {
    pub timestamp: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

/// Read one historical session. The caller must label the session date; it is
/// never an executable quote for the live simulator.
pub async fn fetch_minute_day(
    config: &LocalHistoryConfig,
    symbol: &str,
    day: &str,
) -> Result<Vec<LocalMinuteBar>, String> {
    let code = normalize_symbol(symbol)?;
    let day = normalize_date(day)?;
    let url = query_url(config, "vals", "分钟k", &code, &format!("fwd:{day}000000,{day}235959"))?;
    let (value, _) = request_value(&client(config)?, url).await?;
    let rows = value.as_array().ok_or("本地分钟 K 响应不是数组")?;
    if rows.len() > 600 {
        return Err("单日分钟 K 超过 600 根，拒绝异常数据".into());
    }
    parse_minute_rows(rows, &day)
}

fn parse_minute_rows(rows: &[Value], day: &str) -> Result<Vec<LocalMinuteBar>, String> {
    let mut bars = Vec::with_capacity(rows.len());
    for item in rows {
        let row = item.as_object().ok_or("本地分钟 K 包含无效记录")?;
        let timestamp = string_value(row.get("date"), "date")?;
        if timestamp.len() != 14 || !timestamp.starts_with(day)
            || chrono::NaiveDateTime::parse_from_str(&timestamp, "%Y%m%d%H%M%S").is_err()
        {
            return Err("分钟 K 时间戳与请求日期不符".into());
        }
        let bar = LocalMinuteBar {
            timestamp,
            open: number(row.get("open"), "open")?,
            high: number(row.get("high"), "high")?,
            low: number(row.get("low"), "low")?,
            close: number(row.get("close"), "close")?,
            volume: number(row.get("volume"), "volume")?,
            amount: number(row.get("amount"), "amount")?,
        };
        if [bar.open, bar.high, bar.low, bar.close].iter().any(|v| !v.is_finite() || *v <= 0.0)
            || bar.low > bar.open.min(bar.close) || bar.high < bar.open.max(bar.close)
            || bar.low > bar.high || bar.volume < 0.0 || bar.amount < 0.0
        {
            return Err(format!("分钟 K OHLC 或成交量无效：{}", bar.timestamp));
        }
        bars.push(bar);
    }
    bars.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    if bars.windows(2).any(|w| w[0].timestamp == w[1].timestamp) {
        return Err("分钟 K 包含重复时间戳".into());
    }
    Ok(bars)
}

/// Verify that the configured loopback stockdb HTTP endpoint responds with a
/// supported payload. The deliberately empty date keeps the probe response small.
pub async fn probe(config: &LocalHistoryConfig) -> Result<LocalHistoryProtocol, String> {
    let client = client(config)?;
    let url = query_url(config, "vals", "日k", "000001", "key:19000101")?;
    let (_, protocol) = request_value(&client, url).await?;
    Ok(protocol)
}

/// Read-only date and breadth verification after an updater exits successfully.
/// This does not certify model factors; model refresh keeps its own stricter checks.
pub async fn verify_completed_day(config:&LocalHistoryConfig,expected:&str,previous:&str)->Result<(),String>{
    async fn snapshot(config:&LocalHistoryConfig,date:&str)->Result<Value,String>{
        let mut url=validate_base_url(&config.base_url)?;url.set_query(None);
        url.query_pairs_mut().append_pair("cmd","vals").append_pair("t","日k").append_pair("json","1").append_pair("k1","all:").append_pair("k2",&format!("key:{}",normalize_date(date)?));
        let mut response=client(config)?.get(url).send().await.map_err(|e|format!("更新后读取StockDB失败：{e}"))?.error_for_status().map_err(|e|e.to_string())?;
        let mut bytes=Vec::new();
        while let Some(chunk)=response.chunk().await.map_err(|e|e.to_string())?{if bytes.len()+chunk.len()>32*1024*1024{return Err("StockDB日线响应超过32MiB，未确认更新完成".into());}bytes.extend_from_slice(&chunk);}
        serde_json::from_slice(&bytes).map_err(|e|format!("StockDB日线JSON无效：{e}"))
    }
    let (current,baseline)=tokio::try_join!(snapshot(config,expected),snapshot(config,previous))?;
    let current=completed_codes(&current,&normalize_date(expected)?)?;let baseline=completed_codes(&baseline,&normalize_date(previous)?)?;
    if baseline.len()<2000||current.len()<2000||current.intersection(&baseline).count()*100<baseline.len()*95{return Err(format!("StockDB更新尚未取得{expected}的完整日线：本期{}只、前期{}只；等待后台重试",current.len(),baseline.len()));}
    Ok(())
}
fn completed_codes(value:&Value,expected:&str)->Result<std::collections::HashSet<String>,String>{
    let mut codes=std::collections::HashSet::new();
    for item in value.as_array().ok_or("StockDB更新核验响应不是数组")?{
        let row=row_object(item).ok_or("StockDB更新核验含无效记录")?;
        if normalize_date(&string_value(row.get("date"),"date")?)?!=expected{return Err("StockDB更新核验返回其他日期，未确认成功".into());}
        let code=string_value(row.get("code"),"code")?;let plain=code.rsplit('.').next().unwrap_or(&code);let plain=plain.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        if (code.starts_with("sh")&&plain.starts_with("00"))||(code.starts_with("sz")&&plain.starts_with("39")){continue;}
        if plain.len()!=6||!plain.bytes().all(|b|b.is_ascii_digit())||!matches!(&plain[..2],"00"|"30"|"60"|"68"|"83"|"87"|"88"|"92"){continue;}
        let (open,high,low,close)=(number(row.get("open"),"open")?,number(row.get("high"),"high")?,number(row.get("low"),"low")?,number(row.get("close"),"close")?);
        if [open,high,low,close].iter().any(|v|!v.is_finite()||*v<=0.)||high<open.max(close)||low>open.min(close)||low>high{return Err("StockDB更新核验价格无效".into());}
        if !codes.insert(plain.to_string()){return Err("StockDB更新核验有重复代码".into());}
    }
    Ok(codes)
}

/// Fetch raw daily rows plus adjustment factors and return validated, ascending
/// qfq K-lines. Dates accept `YYYYMMDD` or `YYYY-MM-DD`.
pub async fn fetch_daily(
    config: &LocalHistoryConfig,
    symbol: &str,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<LocalHistoryResult, String> {
    let code = normalize_symbol(symbol)?;
    let start = start.map(normalize_date).transpose()?;
    let end = end.map(normalize_date).transpose()?;
    if matches!((&start, &end), (Some(start), Some(end)) if start > end) {
        return Err("本地历史数据开始日期不能晚于结束日期".to_string());
    }

    let range = match (&start, &end) {
        (None, None) => "all:".to_string(),
        (Some(start), Some(end)) => format!("fwd:{start},{end}"),
        (Some(start), None) => format!("fwd:{start},99991231"),
        (None, Some(end)) => format!("fwd:00000101,{end}"),
    };
    let client = client(config)?;
    let rows_url = query_url(config, "vals", "日k", &code, &range)?;
    let factors_url = query_url(config, "get", "复权", &code, "all:")?;
    let ((rows, protocol), (factors, _)) = tokio::try_join!(
        request_value(&client, rows_url),
        request_value(&client, factors_url)
    )?;
    let mut st_by_date = Vec::new();
    for item in rows.as_array().ok_or("本地日 K 响应不是数组")? {
        let row = row_object(item).ok_or("本地日 K 包含无效记录")?;
        let day = normalize_date(&string_value(row.get("date"), "date")?)?;
        let status = match row.get("is_st") {
            Some(Value::Bool(v)) => Some(*v),
            Some(v) if v.as_i64() == Some(0) => Some(false),
            Some(v) if v.as_i64() == Some(1) => Some(true),
            _ => None,
        };
        st_by_date.push((format!("{}-{}-{}", &day[..4], &day[4..6], &day[6..]), status));
    }
    st_by_date.sort_by(|a, b| a.0.cmp(&b.0));
    // 指定历史截止时，未来因子不得经价格舍入影响过去信号。
    let mut parsed_factors=parse_factors(factors,&code)?;
    if let Some(end)=end.as_ref(){parsed_factors.retain(|(date,_)|date<=end);}
    let factors=Value::Array(parsed_factors.into_iter().map(|(date,cum)|serde_json::json!({"date":date,"cum":cum})).collect());
    let (klines, raw_klines) = parse_rows(rows, factors, &code)?;
    let start_date = klines.first().map(|row| row.date.clone());
    let end_date = klines.last().map(|row| row.date.clone());
    Ok(LocalHistoryResult {
        sample_count: klines.len(),
        klines,
        raw_klines,
        st_by_date,
        source: SOURCE.to_string(),
        protocol,
        start_date,
        end_date,
    })
}

fn client(config: &LocalHistoryConfig) -> Result<Client, String> {
    validate_base_url(&config.base_url)?;
    Client::builder()
        .no_proxy()
        .timeout(config.timeout)
        .build()
        .map_err(|error| format!("创建本地历史数据客户端失败: {error}"))
}

fn validate_base_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("本地历史数据地址无效: {error}"))?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return Err("本地历史数据地址必须是无身份信息的 HTTP loopback 地址".to_string());
    }
    let loopback = match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    };
    if !loopback {
        return Err("本地历史数据地址仅允许 loopback 主机".to_string());
    }
    Ok(url)
}

fn query_url(
    config: &LocalHistoryConfig,
    command: &str,
    table: &str,
    code: &str,
    range: &str,
) -> Result<Url, String> {
    let mut url = validate_base_url(&config.base_url)?;
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("cmd", command)
        .append_pair("t", table)
        .append_pair("k1", &format!("key:{code}"))
        .append_pair("k2", range);
    Ok(url)
}

async fn request_value(client: &Client, url: Url) -> Result<(Value, LocalHistoryProtocol), String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("连接本地历史数据服务失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("本地历史数据服务返回错误: {error}"))?;
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取本地历史数据失败: {error}"))?;
    decode_payload(&bytes, &content_type)
}

fn decode_payload(
    bytes: &[u8],
    content_type: &str,
) -> Result<(Value, LocalHistoryProtocol), String> {
    let msgpack = content_type.contains("msgpack")
        || !bytes
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| matches!(byte, b'[' | b'{'));
    if msgpack {
        rmp_serde::from_slice(bytes)
            .map(|value| (value, LocalHistoryProtocol::MessagePack))
            .map_err(|error| format!("解析本地历史 MessagePack 失败: {error}"))
    } else {
        serde_json::from_slice(bytes)
            .map(|value| (value, LocalHistoryProtocol::Json))
            .map_err(|error| format!("解析本地历史 JSON 失败: {error}"))
    }
}

fn normalize_symbol(symbol: &str) -> Result<String, String> {
    let symbol = symbol.trim().to_ascii_lowercase();
    let code = ["sh", "sz", "bj"]
        .iter()
        .find_map(|prefix| symbol.strip_prefix(prefix))
        .unwrap_or(&symbol);
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("无效的 A 股代码: {symbol}"));
    }
    Ok(code.to_string())
}

fn normalize_date(date: &str) -> Result<String, String> {
    let compact = date.trim().replace('-', "");
    NaiveDate::parse_from_str(&compact, "%Y%m%d")
        .map(|date| date.format("%Y%m%d").to_string())
        .map_err(|_| format!("无效日期: {date}"))
}

fn parse_rows(
    rows: Value,
    factors: Value,
    code: &str,
) -> Result<(Vec<KLineData>, Vec<KLineData>), String> {
    let factors = parse_factors(factors, code)?;
    let latest_factor = factors.last().map(|(_, factor)| *factor).unwrap_or(1.0);
    let decimals = if code.starts_with('1') || code.starts_with('5') {
        3
    } else {
        2
    };
    let rows = rows
        .as_array()
        .ok_or_else(|| "本地日 K 响应不是数组".to_string())?;
    let mut klines = Vec::with_capacity(rows.len());
    let mut raw_klines = Vec::with_capacity(rows.len());
    for item in rows {
        check_stock_identity(item, code)?;
        let row = row_object(item).ok_or_else(|| "本地日 K 包含无效记录".to_string())?;
        let compact_date = normalize_date(&string_value(row.get("date"), "date")?)?;
        let factor = factors
            .iter()
            .rev()
            .find(|(date, _)| date <= &compact_date)
            .map(|(_, factor)| *factor)
            .unwrap_or(1.0);
        let ratio = latest_factor / factor;
        if !ratio.is_finite() || ratio <= 0.0 {
            return Err(format!("本地日 K {compact_date} 复权因子无效"));
        }
        let raw = |field| number(row.get(field), field);
        let raw_open = raw("open")?;
        let raw_high = raw("high")?;
        let raw_low = raw("low")?;
        let raw_close = raw("close")?;
        let open = round(raw_open / ratio, decimals);
        let high = round(raw_high / ratio, decimals);
        let low = round(raw_low / ratio, decimals);
        let close = round(raw_close / ratio, decimals);
        if [open, high, low, close]
            .iter()
            .any(|price| !price.is_finite() || *price <= 0.0)
            || high < open.max(close)
            || low > open.min(close)
            || low > high
            || [raw_open, raw_high, raw_low, raw_close]
                .iter()
                .any(|price| !price.is_finite() || *price <= 0.0)
            || raw_high < raw_open.max(raw_close)
            || raw_low > raw_open.min(raw_close)
            || raw_low > raw_high
        {
            return Err(format!("本地日 K {compact_date} OHLC 无效"));
        }
        let volume_shares = number(row.get("volume"), "volume")?;
        if !volume_shares.is_finite() || volume_shares < 0.0 || volume_shares > u64::MAX as f64 {
            return Err(format!("本地日 K {compact_date} 成交量无效"));
        }
        let turnover = row
            .get("turnover")
            .map(|value| number(Some(value), "turnover"))
            .transpose()?
            .unwrap_or(0.0);
        if !turnover.is_finite() || turnover < 0.0 {
            return Err(format!("本地日 K {compact_date} 换手率无效"));
        }
        let date = format!(
            "{}-{}-{}",
            &compact_date[..4],
            &compact_date[4..6],
            &compact_date[6..]
        );
        let volume = (volume_shares / 100.0) as u64;
        klines.push(KLineData {
            date: date.clone(),
            open,
            high,
            low,
            close,
            volume,
            turnover,
        });
        raw_klines.push(KLineData {
            date,
            open: raw_open,
            high: raw_high,
            low: raw_low,
            close: raw_close,
            volume,
            turnover,
        });
    }
    klines.sort_by(|left, right| left.date.cmp(&right.date));
    raw_klines.sort_by(|left, right| left.date.cmp(&right.date));
    if klines.windows(2).any(|rows| rows[0].date == rows[1].date) {
        return Err("本地日 K 包含重复日期".to_string());
    }
    Ok((klines, raw_klines))
}

fn parse_factors(value: Value, code: &str) -> Result<Vec<(String, f64)>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "本地复权响应不是数组".to_string())?;
    let mut factors = Vec::with_capacity(items.len());
    for item in items {
        check_stock_identity(item, code)?;
        let (date, row) = match item.as_array() {
            Some(pair) if pair.len() == 2 => {
                let key = pair[0]
                    .as_str()
                    .ok_or_else(|| "本地复权键无效".to_string())?;
                let date = key.rsplit(':').next().unwrap_or(key);
                (normalize_date(date)?, &pair[1])
            }
            _ => {
                let row = item
                    .as_object()
                    .ok_or_else(|| "本地复权记录无效".to_string())?;
                (
                    normalize_date(&string_value(row.get("date"), "date")?)?,
                    item,
                )
            }
        };
        let row = row
            .as_object()
            .ok_or_else(|| "本地复权值无效".to_string())?;
        let factor = number(row.get("cum"), "cum")?;
        if !factor.is_finite() || factor <= 0.0 {
            return Err(format!("本地复权因子 {date} 无效"));
        }
        factors.push((date, factor));
    }
    factors.sort_by(|left, right| left.0.cmp(&right.0));
    if factors.windows(2).any(|rows| rows[0].0 == rows[1].0) {
        return Err("本地复权数据包含重复日期".to_string());
    }
    Ok(factors)
}

fn row_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.as_object().or_else(|| {
        value
            .as_array()
            .filter(|pair| pair.len() == 2)
            .and_then(|pair| pair[1].as_object())
    })
}

fn check_stock_identity(value: &Value, expected: &str) -> Result<(), String> {
    let check = |returned: String| {
        if returned == expected { Ok(()) }
        else { Err(format!("本地历史股票身份{returned}与请求{expected}不一致")) }
    };
    if let Some(code) = row_object(value).and_then(|row| row.get("code")) {
        check(normalize_symbol(&string_value(Some(code), "code")?)?)?;
    }
    if let Some(pair) = value.as_array().filter(|pair| pair.len() == 2) {
        let key = pair[0].as_str().ok_or("本地历史记录键无效")?;
        for part in key.split(':') {
            if let Ok(code) = normalize_symbol(part) { check(code)?; }
        }
    }
    Ok(())
}

fn string_value(value: Option<&Value>, field: &str) -> Result<String, String> {
    match value {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        _ => Err(format!("本地历史字段 {field} 无效")),
    }
}

fn number(value: Option<&Value>, field: &str) -> Result<f64, String> {
    match value {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| format!("本地历史字段 {field} 无效"))
}

fn round(value: f64, decimals: i32) -> f64 {
    let scale = 10_f64.powi(decimals);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    #[test]
    fn update_verification_rejects_stale_duplicate_and_invalid_price_rows(){
        let row=serde_json::json!({"code":"600000","date":"20260930","open":10,"high":11,"low":9,"close":10.5});
        assert_eq!(super::completed_codes(&serde_json::json!([row.clone()]),"20260930").unwrap().len(),1);
        assert!(super::completed_codes(&serde_json::json!([row.clone()]),"20260929").is_err());
        assert!(super::completed_codes(&serde_json::json!([row.clone(),row.clone()]),"20260930").is_err());
        let mut invalid=row;invalid["close"]=serde_json::json!(20);assert!(super::completed_codes(&serde_json::json!([invalid]),"20260930").is_err());
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn minute_rows_require_matching_timestamp_and_valid_prices() {
        let good = json!([{"date":20260918093000i64,"open":10.0,"high":10.2,"low":9.9,"close":10.1,"volume":100,"amount":1010}]);
        assert_eq!(parse_minute_rows(good.as_array().unwrap(), "20260918").unwrap().len(), 1);
        assert!(parse_minute_rows(good.as_array().unwrap(), "20260919").is_err());
        let bad = json!([{"date":20260918093000i64,"open":10.0,"high":9.0,"low":9.9,"close":10.1,"volume":100,"amount":1010}]);
        assert!(parse_minute_rows(bad.as_array().unwrap(), "20260918").is_err());
    }

    fn rows() -> Value {
        json!([
            {"date": 20240102, "open": 20, "high": 22, "low": 19, "close": 21, "volume": 123400, "turnover": 1.2},
            ["day:600000:20240103", {"date": "20240103", "open": 21, "high": 24, "low": 20, "close": 23, "volume": "200000"}]
        ])
    }

    fn factors() -> Value {
        json!([
            ["复权:600000:20240101", {"cum": 1.0}],
            ["复权:600000:20240103", {"cum": 2.0}]
        ])
    }

    #[test]
    fn decodes_json_and_messagepack() {
        let json_bytes = serde_json::to_vec(&rows()).unwrap();
        assert_eq!(
            decode_payload(&json_bytes, "application/json").unwrap().1,
            LocalHistoryProtocol::Json
        );
        let packed = rmp_serde::to_vec(&rows()).unwrap();
        assert_eq!(
            decode_payload(&packed, "application/x-msgpack").unwrap().1,
            LocalHistoryProtocol::MessagePack
        );
    }

    #[test]
    fn applies_qfq_and_converts_shares_to_hands() {
        let (parsed, raw) = parse_rows(rows(), factors(), "600000").unwrap();
        assert_eq!(parsed[0].date, "2024-01-02");
        assert_eq!((parsed[0].open, parsed[0].close), (10.0, 10.5));
        assert_eq!(parsed[0].volume, 1234);
        assert_eq!(parsed[1].close, 23.0);
        assert_eq!(parsed[1].volume, 2000);
        assert_eq!(raw[0].close, 21.0);
        assert_eq!(raw[1].close, 23.0);
    }

    #[test]
    fn rejects_duplicate_dates_and_invalid_ohlc() {
        let duplicate = json!([
            {"date": 20240102, "open": 10, "high": 11, "low": 9, "close": 10, "volume": 100},
            {"date": 20240102, "open": 10, "high": 11, "low": 9, "close": 10, "volume": 100}
        ]);
        assert!(parse_rows(duplicate, json!([]), "600000")
            .unwrap_err()
            .contains("重复日期"));
        let bad = json!([{"date": 20240102, "open": 10, "high": 9, "low": 8, "close": 10, "volume": 100}]);
        assert!(parse_rows(bad, json!([]), "600000")
            .unwrap_err()
            .contains("OHLC"));
    }

    #[test]
    fn daily_parser_rejects_explicit_stock_identity_conflicts() {
        let row = rows()[0].clone();
        assert!(parse_rows(rows(), factors(), "600000").is_ok(), "Legacy rows without explicit identity stay supported");
        for code in [json!("600000"), json!(600000), json!("sh600000")] {
            let mut matching = row.clone(); matching["code"] = code;
            assert!(parse_rows(json!([matching]), json!([]), "600000").is_ok());
        }
        let mut conflict = row.clone(); conflict["code"] = json!("600001");
        assert!(parse_rows(json!([conflict.clone()]), json!([]), "600000").unwrap_err().contains("股票身份"));
        assert!(parse_rows(json!([["day:600000:20240102", conflict]]), json!([]), "600000").is_err(), "A matching key cannot hide a conflicting code field");
        let mut matching = row.clone(); matching["code"] = json!("600000");
        for key in ["day:600001:20240102", "day:sh600001:20240102", "600001:20240102", "600001"] {
            assert!(parse_rows(json!([[key, matching.clone()]]), json!([]), "600000").unwrap_err().contains("股票身份"), "Conflicting tuple key: {key}");
        }
        assert!(parse_rows(json!([["day:600000:20240102", matching]]), json!([]), "600000").is_ok());
        for factors in [json!([["复权:600001:20240101", {"cum":2.0}]]), json!([{"code":"600001","date":"20240101","cum":2.0}])] {
            assert!(parse_rows(json!([row.clone()]), factors, "600000").unwrap_err().contains("股票身份"), "Adjustment factors must belong to the requested stock too");
        }
    }

    #[test]
    fn only_allows_loopback_http() {
        assert!(validate_base_url("http://127.0.0.1:7899").is_ok());
        assert!(validate_base_url("http://[::1]:7899").is_ok());
        assert!(validate_base_url("http://localhost:7899").is_ok());
        assert!(validate_base_url("http://192.168.1.2:7899").is_err());
        assert!(validate_base_url("https://127.0.0.1:7899").is_err());
    }

    #[tokio::test]
    #[ignore = "requires the local stockdb service on 127.0.0.1:7899"]
    async fn local_stockdb_smoke() {
        let config = LocalHistoryConfig::new("http://127.0.0.1:7899");
        for symbol in ["600519", "000001", "920000"] {
            let result = fetch_daily(&config, symbol, Some("20260101"), Some("20261231"))
                .await
                .unwrap_or_else(|error| panic!("{symbol}: {error}"));
            assert!(result.sample_count > 0, "{symbol}: empty local history");
            assert_eq!(result.klines.len(), result.raw_klines.len());
            assert!(result
                .klines
                .iter()
                .zip(&result.raw_klines)
                .all(|(qfq, raw)| qfq.date == raw.date));
            assert!(
                result.start_date <= result.end_date,
                "{symbol}: invalid range"
            );
            eprintln!(
                "{symbol}: {:?} {:?}..{:?}, {} rows",
                result.protocol, result.start_date, result.end_date, result.sample_count
            );
        }
    }
}
