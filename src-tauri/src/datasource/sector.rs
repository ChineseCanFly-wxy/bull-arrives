//! 东方财富行业 / 概念板块排行适配器。
//!
//! 一期负责板块列表、排行和按需成分股。历史行情留给后续按需接口，避免打开板块中心时
//! 为每个板块追加请求。字段按板块接口的 f-code 解析，而不是依赖返回数组顺序。

use crate::datasource::eastmoney_universe::universe_client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const HOSTS: [&str; 3] = [
    "https://17.push2.eastmoney.com",
    "https://79.push2.eastmoney.com",
    "https://push2.eastmoney.com",
];
const MEMBER_HOSTS: [&str; 4] = [
    "https://29.push2.eastmoney.com",
    "https://17.push2.eastmoney.com",
    "https://79.push2.eastmoney.com",
    "https://push2.eastmoney.com",
];
const UT_TOKEN: &str = "bd1d9ddb04089700cf9c27f6f7426281";
const PAGE_SIZE: u32 = 100;
const MAX_PAGES: u32 = 20;
pub const SUMMARY_TTL: Duration = Duration::from_secs(60);
pub const MEMBER_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectorKind {
    Industry,
    Concept,
}

impl SectorKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "industry" => Ok(Self::Industry),
            "concept" => Ok(Self::Concept),
            _ => Err("板块类型必须是 industry 或 concept".into()),
        }
    }

    fn market_filter(self) -> &'static str {
        match self {
            Self::Industry => "m:90 t:2 f:!50",
            Self::Concept => "m:90 t:3 f:!50",
        }
    }

    fn rank_field(self) -> &'static str {
        match self {
            Self::Industry => "f3",
            Self::Concept => "f12",
        }
    }

    fn preferred_rank_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f1", "f2"],
            Self::Concept => &["f2", "f1"],
        }
    }

    fn preferred_code_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f14", "f15"],
            Self::Concept => &["f15", "f14"],
        }
    }

    fn preferred_change_amount_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f5", "f8"],
            Self::Concept => &["f8", "f5"],
        }
    }

    fn preferred_turnover_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f9", "f12"],
            Self::Concept => &["f12", "f9"],
        }
    }

    fn preferred_market_cap_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f23", "f24"],
            Self::Concept => &["f24", "f23"],
        }
    }

    fn preferred_up_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f136", "f124"],
            Self::Concept => &["f124", "f136"],
        }
    }

    fn preferred_down_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f115", "f107"],
            Self::Concept => &["f107", "f115"],
        }
    }

    fn preferred_leader_fields(self) -> &'static [&'static str] {
        match self {
            Self::Industry => &["f104", "f136"],
            Self::Concept => &["f136", "f104"],
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorSummary {
    pub kind: SectorKind,
    pub code: String,
    pub name: String,
    pub rank: u32,
    pub latest: Option<f64>,
    pub change_amount: Option<f64>,
    pub change_pct: Option<f64>,
    pub amount: Option<f64>,
    pub market_cap: Option<f64>,
    pub turnover_rate: Option<f64>,
    pub up_count: Option<u32>,
    pub down_count: Option<u32>,
    pub leader_name: Option<String>,
    pub leader_change_pct: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorSummaryPage {
    pub kind: SectorKind,
    pub page: u32,
    pub page_size: u32,
    pub total: usize,
    pub items: Vec<SectorSummary>,
    pub as_of: String,
    pub source: String,
    pub stale: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorMember {
    pub code: String,
    pub market: String,
    pub name: String,
    pub price: Option<f64>,
    pub change_amount: Option<f64>,
    pub change_pct: Option<f64>,
    pub amount: Option<f64>,
    pub turnover_rate: Option<f64>,
    pub market_cap: Option<f64>,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorMemberPage {
    pub kind: SectorKind,
    pub sector_code: String,
    pub page: u32,
    pub page_size: u32,
    pub total: usize,
    pub items: Vec<SectorMember>,
    pub as_of: String,
    pub source: String,
    pub stale: bool,
}

#[derive(Debug, thiserror::Error)]
enum SectorError {
    #[error("板块接口 HTTP {0}")]
    Status(u16),
    #[error("板块接口请求失败: {0}")]
    Http(#[from] reqwest::Error),
    #[error("板块接口没有返回有效数据")]
    Empty,
}

#[derive(Debug, Deserialize)]
struct ClistResponse {
    data: Option<ClistData>,
}

#[derive(Debug, Deserialize)]
struct ClistData {
    total: Option<u32>,
    diff: Option<serde_json::Value>,
}

fn number(value: &serde_json::Value) -> Option<f64> {
    let parsed = match value {
        serde_json::Value::Number(value) => value.as_f64(),
        serde_json::Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    parsed.is_finite().then_some(parsed)
}

fn number_at(row: &serde_json::Value, fields: &[&str]) -> Option<f64> {
    fields
        .iter()
        .find_map(|field| row.get(*field).and_then(number))
}

fn text_at(row: &serde_json::Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        let value = row.get(*field)?;
        let text = match value {
            serde_json::Value::String(value) => value.trim().to_owned(),
            serde_json::Value::Number(value) => value.to_string(),
            _ => return None,
        };
        (!text.is_empty() && text != "-").then_some(text)
    })
}

fn count_at(row: &serde_json::Value, fields: &[&str]) -> Option<u32> {
    number_at(row, fields).and_then(|value| {
        (value >= 0.0 && value <= u32::MAX as f64).then_some(value.round() as u32)
    })
}

fn parse_summary(
    kind: SectorKind,
    row: &serde_json::Value,
    fallback_rank: usize,
) -> Option<SectorSummary> {
    let code = text_at(row, kind.preferred_code_fields())?;
    if !code.starts_with("BK") {
        return None;
    }
    let name = text_at(row, &["f16", "f14"])?;
    let rank = number_at(row, kind.preferred_rank_fields())
        .filter(|value| *value >= 1.0)
        .map(|value| value.round() as u32)
        .unwrap_or(fallback_rank as u32);

    Some(SectorSummary {
        kind,
        code,
        name,
        rank,
        latest: number_at(row, &["f3"]),
        change_amount: number_at(row, kind.preferred_change_amount_fields()),
        change_pct: number_at(row, &["f4"]),
        amount: number_at(row, &["f6"]),
        market_cap: number_at(row, kind.preferred_market_cap_fields()),
        turnover_rate: number_at(row, kind.preferred_turnover_fields()),
        up_count: count_at(row, kind.preferred_up_fields()),
        down_count: count_at(row, kind.preferred_down_fields()),
        leader_name: text_at(row, kind.preferred_leader_fields()),
        leader_change_pct: number_at(row, &["f141", "f128"]),
    })
}

fn diff_rows(kind: SectorKind, diff: Option<serde_json::Value>) -> Vec<SectorSummary> {
    let Some(serde_json::Value::Array(rows)) = diff else {
        return Vec::new();
    };
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| parse_summary(kind, row, index + 1))
        .collect()
}

fn market_for_stock_code(code: &str) -> &'static str {
    if code.starts_with("43")
        || code.starts_with("83")
        || code.starts_with("87")
        || code.starts_with("920")
    {
        "bj"
    } else if code.starts_with('6') || code.starts_with('5') || code.starts_with('9') {
        "sh"
    } else {
        "sz"
    }
}

fn parse_member(row: &serde_json::Value) -> Option<SectorMember> {
    let code = text_at(row, &["f12"])?;
    if code.len() != 6 || !code.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    let name = text_at(row, &["f14"])?;
    Some(SectorMember {
        market: market_for_stock_code(&code).to_owned(),
        code,
        name,
        price: number_at(row, &["f2"]),
        change_amount: number_at(row, &["f4"]),
        change_pct: number_at(row, &["f3"]),
        amount: number_at(row, &["f6"]),
        turnover_rate: number_at(row, &["f8"]),
        market_cap: number_at(row, &["f20"]),
        pe: number_at(row, &["f9"]),
        pb: number_at(row, &["f23"]),
    })
}

fn diff_members(diff: Option<serde_json::Value>) -> Vec<SectorMember> {
    let Some(serde_json::Value::Array(rows)) = diff else {
        return Vec::new();
    };
    rows.iter().filter_map(parse_member).collect()
}

async fn fetch_page_uncached(
    kind: SectorKind,
    page: u32,
    page_size: u32,
) -> Result<(Vec<SectorSummary>, usize), SectorError> {
    let client = universe_client();
    let fields =
        "f1,f2,f3,f4,f5,f6,f8,f9,f12,f14,f15,f16,f23,f24,f104,f107,f115,f124,f128,f136,f141";
    let params = [
        ("pn", page.to_string()),
        ("pz", page_size.to_string()),
        ("po", "1".to_owned()),
        ("np", "1".to_owned()),
        ("ut", UT_TOKEN.to_owned()),
        ("fltt", "2".to_owned()),
        ("invt", "2".to_owned()),
        ("fid", kind.rank_field().to_owned()),
        ("fs", kind.market_filter().to_owned()),
        ("fields", fields.to_owned()),
    ];
    let mut last_error = None;

    for host in HOSTS {
        let url = format!("{host}/api/qt/clist/get");
        let response = client
            .get(&url)
            .header("Referer", "https://quote.eastmoney.com/")
            .query(&params)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(SectorError::Http(error));
                continue;
            }
        };
        if !response.status().is_success() {
            last_error = Some(SectorError::Status(response.status().as_u16()));
            continue;
        }

        match response.json::<ClistResponse>().await {
            Ok(payload) => {
                let Some(data) = payload.data else {
                    last_error = Some(SectorError::Empty);
                    continue;
                };
                let total = data.total.unwrap_or(0) as usize;
                let rows = diff_rows(kind, data.diff);
                if rows.is_empty() {
                    last_error = Some(SectorError::Empty);
                    continue;
                }
                return Ok((rows, total));
            }
            Err(error) => last_error = Some(SectorError::Http(error)),
        }
    }

    Err(last_error.unwrap_or(SectorError::Empty))
}

fn valid_sector_code(code: &str) -> bool {
    code.len() == 6
        && code.starts_with("BK")
        && code[2..].chars().all(|value| value.is_ascii_digit())
}

async fn fetch_member_page_uncached(
    sector_code: &str,
    page: u32,
    page_size: u32,
) -> Result<(Vec<SectorMember>, usize), SectorError> {
    let client = universe_client();
    let params = [
        ("pn", page.to_string()),
        ("pz", page_size.to_string()),
        ("po", "1".to_owned()),
        ("np", "1".to_owned()),
        ("ut", UT_TOKEN.to_owned()),
        ("fltt", "2".to_owned()),
        ("invt", "2".to_owned()),
        ("fid", "f12".to_owned()),
        ("fs", format!("b:{sector_code} f:!50")),
        ("fields", "f2,f3,f4,f6,f8,f9,f12,f14,f20,f23".to_owned()),
    ];
    let mut last_error = None;

    for host in MEMBER_HOSTS {
        let url = format!("{host}/api/qt/clist/get");
        let response = match client
            .get(&url)
            .header("Referer", "https://quote.eastmoney.com/")
            .query(&params)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(SectorError::Http(error));
                continue;
            }
        };
        if !response.status().is_success() {
            last_error = Some(SectorError::Status(response.status().as_u16()));
            continue;
        }

        match response.json::<ClistResponse>().await {
            Ok(payload) => {
                let Some(data) = payload.data else {
                    last_error = Some(SectorError::Empty);
                    continue;
                };
                let total = data.total.unwrap_or(0) as usize;
                let rows = diff_members(data.diff);
                if rows.is_empty() {
                    last_error = Some(SectorError::Empty);
                    continue;
                }
                return Ok((rows, total));
            }
            Err(error) => last_error = Some(SectorError::Http(error)),
        }
    }

    Err(last_error.unwrap_or(SectorError::Empty))
}

fn sort_summaries(rows: &mut [SectorSummary]) {
    rows.sort_by(|a, b| {
        b.change_pct
            .unwrap_or(f64::NEG_INFINITY)
            .partial_cmp(&a.change_pct.unwrap_or(f64::NEG_INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });
}

async fn fetch_catalog(kind: SectorKind) -> Result<Vec<SectorSummary>, SectorError> {
    let (mut rows, total) = fetch_page_uncached(kind, 1, PAGE_SIZE).await?;
    let pages = ((total.max(rows.len()) as u32 + PAGE_SIZE - 1) / PAGE_SIZE).min(MAX_PAGES);
    for page in 2..=pages {
        let (mut next, _) = fetch_page_uncached(kind, page, PAGE_SIZE).await?;
        rows.append(&mut next);
    }

    let mut unique = HashMap::with_capacity(rows.len());
    for row in rows {
        unique.entry(row.code.clone()).or_insert(row);
    }
    let mut rows: Vec<_> = unique.into_values().collect();
    sort_summaries(&mut rows);
    if rows.is_empty() {
        return Err(SectorError::Empty);
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    kind: SectorKind,
    page: u32,
    page_size: u32,
    keyword: String,
}

#[derive(Debug, Clone)]
struct CachedPage {
    value: SectorSummaryPage,
    fetched_at: Instant,
}

static SUMMARY_CACHE: OnceLock<RwLock<HashMap<CacheKey, CachedPage>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MemberCacheKey {
    kind: SectorKind,
    sector_code: String,
    page: u32,
    page_size: u32,
}

#[derive(Debug, Clone)]
struct CachedMembers {
    value: SectorMemberPage,
    fetched_at: Instant,
}

static MEMBER_CACHE: OnceLock<RwLock<HashMap<MemberCacheKey, CachedMembers>>> = OnceLock::new();

fn now_text() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn page_of(rows: &[SectorSummary], page: u32, page_size: u32) -> Vec<SectorSummary> {
    let start = page.saturating_sub(1) as usize * page_size as usize;
    rows.iter()
        .skip(start)
        .take(page_size as usize)
        .cloned()
        .collect()
}

pub async fn fetch_summaries(
    kind: SectorKind,
    page: u32,
    page_size: u32,
    keyword: &str,
    force_refresh: bool,
) -> Result<SectorSummaryPage, String> {
    let page = page.max(1);
    let page_size = page_size.clamp(20, PAGE_SIZE);
    let keyword = keyword.trim().to_owned();
    let key = CacheKey {
        kind,
        page,
        page_size,
        keyword: keyword.clone(),
    };
    let cache = SUMMARY_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let mut guard = cache.write().await;

    if !force_refresh {
        if let Some(cached) = guard.get(&key) {
            if cached.fetched_at.elapsed() < SUMMARY_TTL {
                return Ok(cached.value.clone());
            }
        }
    }

    let result = if keyword.is_empty() {
        fetch_page_uncached(kind, page, page_size)
            .await
            .map(|(mut rows, total)| {
                sort_summaries(&mut rows);
                SectorSummaryPage {
                    kind,
                    page,
                    page_size,
                    total: total.max(rows.len()),
                    items: rows,
                    as_of: now_text(),
                    source: "eastmoney".to_owned(),
                    stale: false,
                }
            })
    } else {
        fetch_catalog(kind).await.map(|rows| {
            let filtered: Vec<_> = rows
                .iter()
                .filter(|row| row.name.contains(&keyword) || row.code.contains(&keyword))
                .cloned()
                .collect();
            SectorSummaryPage {
                kind,
                page,
                page_size,
                total: filtered.len(),
                items: page_of(&filtered, page, page_size),
                as_of: now_text(),
                source: "eastmoney".to_owned(),
                stale: false,
            }
        })
    };

    match result {
        Ok(value) => {
            guard.insert(
                key,
                CachedPage {
                    value: value.clone(),
                    fetched_at: Instant::now(),
                },
            );
            Ok(value)
        }
        Err(error) => {
            if let Some(cached) = guard.get(&key) {
                let mut value = cached.value.clone();
                value.stale = true;
                log::warn!("板块排行刷新失败，回退到旧缓存（stale）: {error}");
                Ok(value)
            } else {
                Err(error.to_string())
            }
        }
    }
}

pub async fn fetch_members(
    kind: SectorKind,
    sector_code: &str,
    page: u32,
    page_size: u32,
    force_refresh: bool,
) -> Result<SectorMemberPage, String> {
    let sector_code = sector_code.trim().to_uppercase();
    if !valid_sector_code(&sector_code) {
        return Err("板块代码格式无效".into());
    }
    let page = page.max(1);
    let page_size = page_size.clamp(20, PAGE_SIZE);
    let key = MemberCacheKey {
        kind,
        sector_code: sector_code.clone(),
        page,
        page_size,
    };
    let cache = MEMBER_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let mut guard = cache.write().await;

    if !force_refresh {
        if let Some(cached) = guard.get(&key) {
            if cached.fetched_at.elapsed() < MEMBER_TTL {
                return Ok(cached.value.clone());
            }
        }
    }

    let result = fetch_member_page_uncached(&sector_code, page, page_size)
        .await
        .map(|(items, total)| SectorMemberPage {
            kind,
            sector_code: sector_code.clone(),
            page,
            page_size,
            total: total.max(items.len()),
            items,
            as_of: now_text(),
            source: "eastmoney".to_owned(),
            stale: false,
        });

    match result {
        Ok(value) => {
            guard.insert(
                key,
                CachedMembers {
                    value: value.clone(),
                    fetched_at: Instant::now(),
                },
            );
            Ok(value)
        }
        Err(error) => {
            if let Some(cached) = guard.get(&key) {
                let mut value = cached.value.clone();
                value.stale = true;
                log::warn!("板块成分刷新失败，回退到旧缓存（stale）: {error}");
                Ok(value)
            } else {
                Err(error.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_industry_summary_by_field_name() {
        let row = json!({
            "f1": 3, "f3": 1234.56, "f4": 2.5, "f5": 30.12, "f6": 900000000,
            "f9": 1.8, "f14": "BK1001", "f16": "半导体", "f23": 1200000000000_i64,
            "f104": "示例科技", "f115": 4, "f136": 18, "f141": 9.9
        });
        let parsed = parse_summary(SectorKind::Industry, &row, 1).unwrap();
        assert_eq!(parsed.code, "BK1001");
        assert_eq!(parsed.name, "半导体");
        assert_eq!(parsed.rank, 3);
        assert_eq!(parsed.change_pct, Some(2.5));
        assert_eq!(parsed.up_count, Some(18));
        assert_eq!(parsed.down_count, Some(4));
        assert_eq!(parsed.leader_name.as_deref(), Some("示例科技"));
    }

    #[test]
    fn parses_concept_summary_with_concept_field_layout() {
        let row = json!({
            "f2": 1, "f3": 888.0, "f4": -1.2, "f8": -10.0, "f12": 3.2,
            "f15": "BK2002", "f16": "机器人", "f24": 800000000000_i64,
            "f107": 12, "f124": 8, "f136": "领涨机器人"
        });
        let parsed = parse_summary(SectorKind::Concept, &row, 1).unwrap();
        assert_eq!(parsed.code, "BK2002");
        assert_eq!(parsed.change_amount, Some(-10.0));
        assert_eq!(parsed.turnover_rate, Some(3.2));
        assert_eq!(parsed.up_count, Some(8));
        assert_eq!(parsed.down_count, Some(12));
        assert_eq!(parsed.leader_name.as_deref(), Some("领涨机器人"));
    }

    #[test]
    fn rejects_non_sector_rows_and_empty_pages() {
        assert!(parse_summary(
            SectorKind::Industry,
            &json!({"f14":"600000","f16":"股票"}),
            1
        )
        .is_none());
        assert!(diff_rows(SectorKind::Industry, Some(json!({}))).is_empty());
        assert!(diff_rows(SectorKind::Concept, Some(json!([]))).is_empty());
    }

    #[test]
    fn sector_kind_is_strict() {
        assert_eq!(SectorKind::parse("industry").unwrap(), SectorKind::Industry);
        assert_eq!(SectorKind::parse("concept").unwrap(), SectorKind::Concept);
        assert!(SectorKind::parse("stock").is_err());
    }

    #[test]
    fn parses_member_and_derives_exchange_prefix() {
        let row = json!({
            "f2": 12.3, "f3": 4.5, "f4": 0.53, "f6": 1000000,
            "f8": 2.1, "f9": 20.0, "f12": "920001", "f14": "示例北交所",
            "f20": 5000000000_i64, "f23": 1.2
        });
        let parsed = parse_member(&row).unwrap();
        assert_eq!(parsed.market, "bj");
        assert_eq!(parsed.code, "920001");
        assert_eq!(parsed.change_pct, Some(4.5));
        assert_eq!(parsed.name, "示例北交所");
    }

    #[test]
    fn rejects_invalid_member_codes() {
        assert!(parse_member(&json!({"f12":"BK1001","f14":"板块"})).is_none());
        assert!(parse_member(&json!({"f12":"600000"})).is_none());
        assert!(valid_sector_code("BK1001"));
        assert!(!valid_sector_code("600000"));
    }
}
