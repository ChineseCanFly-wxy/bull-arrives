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
    pub source: String,
    pub protocol: LocalHistoryProtocol,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub sample_count: usize,
}

/// Verify that the configured loopback stockdb HTTP endpoint responds with a
/// supported payload. The deliberately empty date keeps the probe response small.
pub async fn probe(config: &LocalHistoryConfig) -> Result<LocalHistoryProtocol, String> {
    let client = client(config)?;
    let url = query_url(config, "vals", "日k", "000001", "key:19000101")?;
    let (_, protocol) = request_value(&client, url).await?;
    Ok(protocol)
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
    let (klines, raw_klines) = parse_rows(rows, factors, &code)?;
    let start_date = klines.first().map(|row| row.date.clone());
    let end_date = klines.last().map(|row| row.date.clone());
    Ok(LocalHistoryResult {
        sample_count: klines.len(),
        klines,
        raw_klines,
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
    let factors = parse_factors(factors)?;
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

fn parse_factors(value: Value) -> Result<Vec<(String, f64)>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "本地复权响应不是数组".to_string())?;
    let mut factors = Vec::with_capacity(items.len());
    for item in items {
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
    use super::*;
    use serde_json::json;

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
