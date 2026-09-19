//! 东方财富行业 / 概念板块排行适配器。
//!
//! 一期负责板块列表、排行和按需成分股。历史行情留给后续按需接口，避免打开板块中心时
//! 为每个板块追加请求。字段按板块接口的 f-code 解析，而不是依赖返回数组顺序。

use crate::datasource::eastmoney_universe::universe_client;
use crate::datasource::headers::with_browser_headers;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// `push2delay` 在部分网络出口比标准 push2 域名稳定；其余主机作为兜底。
const HOSTS: [&str; 5] = [
    "https://push2delay.eastmoney.com",
    "https://push2.eastmoney.com",
    "https://82.push2.eastmoney.com",
    "https://17.push2.eastmoney.com",
    "https://79.push2.eastmoney.com",
];
const MEMBER_HOSTS: [&str; 6] = [
    "https://push2delay.eastmoney.com",
    "https://29.push2.eastmoney.com",
    "https://17.push2.eastmoney.com",
    "https://79.push2.eastmoney.com",
    "https://push2.eastmoney.com",
    "https://82.push2.eastmoney.com",
];
const KLINE_HOSTS: [&str; 3] = [
    "https://push2his.eastmoney.com",
    "https://17.push2his.eastmoney.com",
    "https://91.push2his.eastmoney.com",
];
const ROTATION_HOST: &str = "https://push2delay.eastmoney.com";
const UT_TOKEN: &str = "bd1d9ddb04089700cf9c27f6f7426281";
const PAGE_SIZE: u32 = 100;
const MAX_PAGES: u32 = 20;
const RETRY_ROUNDS: usize = 2;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
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
        "f3"
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
    pub change_pct_3d: Option<f64>,
    pub change_pct_5d: Option<f64>,
    pub change_pct_10d: Option<f64>,
    pub main_net_inflow: Option<f64>,
    pub main_net_ratio: Option<f64>,
    pub super_large_net_inflow: Option<f64>,
    pub large_net_inflow: Option<f64>,
    pub medium_net_inflow: Option<f64>,
    pub small_net_inflow: Option<f64>,
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorLimitUpStats {
    pub sector_code: String,
    pub limit_up_count: usize,
    pub member_count: usize,
    pub as_of: String,
    pub source: String,
    pub methodology: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorKline {
    pub date: String,
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub amount: f64,
    pub change_pct: Option<f64>,
    pub turnover_rate: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorHistory {
    pub sector_code: String,
    pub period: String,
    pub items: Vec<SectorKline>,
    pub as_of: String,
    pub source: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RotationStatus {
    pub kind: SectorKind,
    pub ok: bool,
    pub error: Option<String>,
    pub as_of: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorRotation {
    pub items: Vec<SectorSummary>,
    pub statuses: Vec<RotationStatus>,
    pub source: String,
    pub request_count: u8,
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
    // 行业和概念接口当前都使用标准 clist 字段：f12/f14/f2/f3。
    // 不沿用旧版按数组位置或行业/概念分支字段的映射，接口更新后会把名称当代码。
    let code = text_at(row, &["f12"])?;
    if !code.starts_with("BK") {
        return None;
    }
    let name = text_at(row, &["f14"])?;

    Some(SectorSummary {
        kind,
        code,
        name,
        rank: fallback_rank as u32,
        latest: number_at(row, &["f2"]),
        change_amount: number_at(row, &["f4"]),
        change_pct: number_at(row, &["f3"]),
        amount: number_at(row, &["f6"]),
        market_cap: number_at(row, &["f20"]),
        turnover_rate: number_at(row, &["f8"]),
        up_count: count_at(row, &["f104"]),
        down_count: count_at(row, &["f105"]),
        leader_name: text_at(row, &["f128"]),
        leader_change_pct: number_at(row, &["f136"]),
        change_pct_3d: number_at(row, &["f127"]),
        change_pct_5d: number_at(row, &["f109"]),
        change_pct_10d: number_at(row, &["f160"]),
        main_net_inflow: number_at(row, &["f62"]),
        main_net_ratio: number_at(row, &["f184"]),
        super_large_net_inflow: number_at(row, &["f66"]),
        large_net_inflow: number_at(row, &["f72"]),
        medium_net_inflow: number_at(row, &["f78"]),
        small_net_inflow: number_at(row, &["f84"]),
    })
}

fn values_from_diff(diff: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    match diff {
        Some(serde_json::Value::Array(rows)) => rows,
        Some(serde_json::Value::Object(rows)) => rows.into_values().collect(),
        _ => Vec::new(),
    }
}

fn diff_rows(kind: SectorKind, diff: Option<serde_json::Value>) -> Vec<SectorSummary> {
    values_from_diff(diff)
        .iter()
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
    values_from_diff(diff)
        .iter()
        .filter_map(parse_member)
        .collect()
}

async fn fetch_page_uncached(
    kind: SectorKind,
    page: u32,
    page_size: u32,
) -> Result<(Vec<SectorSummary>, usize), SectorError> {
    let client = universe_client();
    let fields = "f1,f2,f3,f4,f5,f6,f8,f9,f12,f14,f20,f23,f62,f66,f72,f78,f84,f104,f105,f109,f127,f128,f136,f160,f184";
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

    for round in 0..RETRY_ROUNDS {
        for host in HOSTS {
            let url = format!("{host}/api/qt/clist/get");
            let response = with_browser_headers(client.get(&url), "https://quote.eastmoney.com/")
                .query(&params)
                .timeout(REQUEST_TIMEOUT)
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
        if round + 1 < RETRY_ROUNDS {
            tokio::time::sleep(Duration::from_millis(300 * (round as u64 + 1))).await;
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

    for round in 0..RETRY_ROUNDS {
        for host in MEMBER_HOSTS {
            let url = format!("{host}/api/qt/clist/get");
            let response =
                match with_browser_headers(client.get(&url), "https://quote.eastmoney.com/")
                    .query(&params)
                    .timeout(REQUEST_TIMEOUT)
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
        if round + 1 < RETRY_ROUNDS {
            tokio::time::sleep(Duration::from_millis(300 * (round as u64 + 1))).await;
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

#[derive(Debug, Clone)]
struct CachedMemberCodes {
    value: HashSet<String>,
    fetched_at: Instant,
}

/// 筛选器只需要成分股代码，单独缓存完整集合。
///
/// 不复用分页表格的 `MEMBER_CACHE`：表格缓存按页切片，而筛选必须拿到
/// 全部成分，否则一个板块超过 100 只时会错误漏股。
static MEMBER_CODE_CACHE: OnceLock<RwLock<HashMap<(SectorKind, String), CachedMemberCodes>>> =
    OnceLock::new();

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

/// 获取一个行业/概念的全部成分股代码，供全市场筛选器使用。
///
/// 东财成分接口支持一次返回完整板块；这里使用与涨停统计相同的 5000 上限，
/// 并以 `MEMBER_TTL` 缓存，保证翻页时不重复请求同一板块。
pub async fn fetch_member_codes(
    kind: SectorKind,
    sector_code: &str,
) -> Result<HashSet<String>, String> {
    let sector_code = sector_code.trim().to_uppercase();
    if !valid_sector_code(&sector_code) {
        return Err("板块代码格式无效".into());
    }

    let key = (kind, sector_code.clone());
    let cache = MEMBER_CODE_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    {
        let guard = cache.read().await;
        if let Some(cached) = guard.get(&key) {
            if cached.fetched_at.elapsed() < MEMBER_TTL {
                return Ok(cached.value.clone());
            }
        }
    }

    let (items, _) = fetch_member_page_uncached(&sector_code, 1, 5_000)
        .await
        .map_err(|error| error.to_string())?;
    let value: HashSet<String> = items.into_iter().map(|item| item.code).collect();
    if value.is_empty() {
        return Err(format!("板块 {sector_code} 没有返回成分股"));
    }

    cache.write().await.insert(
        key,
        CachedMemberCodes {
            value: value.clone(),
            fetched_at: Instant::now(),
        },
    );
    Ok(value)
}

/// 用一次响应取回全部成分，保证派生数量来自同一快照；接口若截断则拒绝返回局部统计。
pub async fn fetch_limit_up_stats(sector_code: &str) -> Result<SectorLimitUpStats, String> {
    let sector_code = sector_code.trim().to_uppercase();
    if !valid_sector_code(&sector_code) {
        return Err("板块代码格式无效".into());
    }
    let (items, total) = fetch_member_page_uncached(&sector_code, 1, 5_000)
        .await
        .map_err(|error| error.to_string())?;
    if items.len() < total {
        return Err(format!(
            "成分股快照被截断（返回 {} / 应有 {}），未生成局部涨停家数",
            items.len(),
            total
        ));
    }
    let limit_up_count = items.iter().filter(|item| member_is_limit_up(item)).count();
    Ok(SectorLimitUpStats {
        sector_code,
        limit_up_count,
        member_count: items.len(),
        as_of: now_text(),
        source: "eastmoney_snapshot_derived".into(),
        methodology: "非官方派生：同一成分股快照按板块涨跌幅阈值统计当前涨停（主板 10%，含 2026-07-06 起并轨的主板 ST/*ST；创业板/科创板 20%；北交所 30%）；N/C 无涨跌幅限制新股不计。".into(),
    })
}

fn member_is_limit_up(item: &SectorMember) -> bool {
    use crate::datasource::eastmoney_universe::{is_limit_up, Board};

    let name = item.name.trim().to_uppercase();
    if name.starts_with('N') || name.starts_with('C') {
        return false;
    }
    item.change_pct.is_some_and(|change| {
        is_limit_up(change, Board::from_code(&item.code), name.contains("ST"))
    })
}

fn parse_kline(line: &str) -> Option<SectorKline> {
    let values: Vec<_> = line.split(',').collect();
    if values.len() < 7 {
        return None;
    }
    let parse = |index: usize| values.get(index)?.parse::<f64>().ok();
    Some(SectorKline {
        date: values[0].to_owned(),
        open: parse(1)?,
        close: parse(2)?,
        high: parse(3)?,
        low: parse(4)?,
        volume: parse(5)?,
        amount: parse(6)?,
        change_pct: parse(8),
        turnover_rate: parse(10),
    })
}

pub async fn fetch_history(sector_code: &str, period: &str) -> Result<SectorHistory, String> {
    let sector_code = sector_code.trim().to_uppercase();
    if !valid_sector_code(&sector_code) {
        return Err("板块代码格式无效".into());
    }
    let klt = match period {
        "daily" => "101",
        "weekly" => "102",
        "monthly" => "103",
        _ => return Err("板块 K 线周期必须是 daily / weekly / monthly".into()),
    };
    let params = [
        ("secid", format!("90.{sector_code}")),
        ("fields1", "f1,f2,f3,f4,f5,f6".to_owned()),
        (
            "fields2",
            "f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61".to_owned(),
        ),
        ("klt", klt.to_owned()),
        ("fqt", "0".to_owned()),
        ("end", "20500101".to_owned()),
        ("lmt", "180".to_owned()),
    ];
    let mut last_error = "板块 K 线接口没有返回有效数据".to_string();
    for host in KLINE_HOSTS {
        let url = format!("{host}/api/qt/stock/kline/get");
        let response =
            with_browser_headers(universe_client().get(&url), "https://quote.eastmoney.com/")
                .query(&params)
                .timeout(REQUEST_TIMEOUT)
                .send()
                .await;
        let response = match response {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                last_error = format!("板块 K 线接口 HTTP {}", response.status());
                continue;
            }
            Err(error) => {
                last_error = format!("板块 K 线请求失败：{error}");
                continue;
            }
        };
        let value: serde_json::Value = match response.json().await {
            Ok(value) => value,
            Err(error) => {
                last_error = format!("板块 K 线解析失败：{error}");
                continue;
            }
        };
        let items: Vec<_> = value
            .pointer("/data/klines")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .filter_map(parse_kline)
            .collect();
        if !items.is_empty() {
            return Ok(SectorHistory {
                sector_code,
                period: period.to_owned(),
                as_of: items
                    .last()
                    .map(|item| item.date.clone())
                    .unwrap_or_default(),
                items,
                source: "eastmoney · 90.BK · unadjusted".to_owned(),
            });
        }
    }
    Err(last_error)
}

async fn fetch_rotation_kind(kind: SectorKind) -> Result<Vec<SectorSummary>, String> {
    // 单次取完整目录，确保行业/概念轮动合计最多两个按需请求。
    let fields = "f2,f3,f12,f14,f62,f66,f72,f78,f84,f109,f127,f160,f184";
    let params = [
        ("pn", "1".to_owned()),
        ("pz", "500".to_owned()),
        ("po", "1".to_owned()),
        ("np", "1".to_owned()),
        ("ut", UT_TOKEN.to_owned()),
        ("fltt", "2".to_owned()),
        ("invt", "2".to_owned()),
        ("fid", "f62".to_owned()),
        ("fs", kind.market_filter().to_owned()),
        ("fields", fields.to_owned()),
    ];
    let url = format!("{ROTATION_HOST}/api/qt/clist/get");
    let response = with_browser_headers(universe_client().get(&url), "https://data.eastmoney.com/")
        .query(&params)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("轮动请求失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("轮动接口 HTTP 错误：{error}"))?
        .json::<ClistResponse>()
        .await
        .map_err(|error| format!("轮动响应解析失败：{error}"))?;
    let rows = diff_rows(kind, response.data.and_then(|data| data.diff));
    if rows.is_empty() {
        Err("轮动接口没有返回有效数据".into())
    } else {
        Ok(rows)
    }
}

pub async fn fetch_rotation() -> SectorRotation {
    let (industry, concept) = tokio::join!(
        fetch_rotation_kind(SectorKind::Industry),
        fetch_rotation_kind(SectorKind::Concept)
    );
    let as_of = now_text();
    let mut items = Vec::new();
    let mut statuses = Vec::with_capacity(2);
    for (kind, result) in [
        (SectorKind::Industry, industry),
        (SectorKind::Concept, concept),
    ] {
        match result {
            Ok(mut rows) => {
                items.append(&mut rows);
                statuses.push(RotationStatus {
                    kind,
                    ok: true,
                    error: None,
                    as_of: as_of.clone(),
                });
            }
            Err(error) => statuses.push(RotationStatus {
                kind,
                ok: false,
                error: Some(error),
                as_of: as_of.clone(),
            }),
        }
    }
    SectorRotation {
        items,
        statuses,
        source: "eastmoney · f127/f109/f160 · f62/f184".to_owned(),
        request_count: 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_current_clist_summary_by_field_name() {
        let row = json!({
            "f1": 2, "f2": 8423.96, "f3": 5.88, "f4": 467.83,
            "f6": 12945516510_i64, "f8": 6.27, "f12": "BK1625", "f14": "钨",
            "f20": 264683053000_i64, "f104": 4, "f105": 0,
            "f62": 1200000000_i64, "f109": 8.2, "f127": 6.1, "f160": 10.4,
            "f184": 5.6, "f128": "翔鹭钨业", "f136": 9.06
        });
        let parsed = parse_summary(SectorKind::Industry, &row, 1).unwrap();
        assert_eq!(parsed.code, "BK1625");
        assert_eq!(parsed.name, "钨");
        assert_eq!(parsed.rank, 1);
        assert_eq!(parsed.latest, Some(8423.96));
        assert_eq!(parsed.change_pct, Some(5.88));
        assert_eq!(parsed.up_count, Some(4));
        assert_eq!(parsed.down_count, Some(0));
        assert_eq!(parsed.leader_name.as_deref(), Some("翔鹭钨业"));
        assert_eq!(parsed.leader_change_pct, Some(9.06));
        assert_eq!(parsed.change_pct_3d, Some(6.1));
        assert_eq!(parsed.change_pct_5d, Some(8.2));
        assert_eq!(parsed.change_pct_10d, Some(10.4));
        assert_eq!(parsed.main_net_inflow, Some(1_200_000_000.0));
        assert_eq!(parsed.main_net_ratio, Some(5.6));
    }

    #[test]
    fn parses_sector_kline_and_rejects_broken_rows() {
        let row = parse_kline(
            "2026-09-18,2800.87,2888.23,2892.38,2791.27,28564253,223590676304,3.68,5.02,137.99,3.25",
        )
        .unwrap();
        assert_eq!(row.date, "2026-09-18");
        assert_eq!(row.close, 2888.23);
        assert_eq!(row.change_pct, Some(5.02));
        assert_eq!(row.turnover_rate, Some(3.25));
        assert!(parse_kline("2026-09-18,1,2").is_none());
    }

    #[test]
    fn parses_concept_summary_with_same_clist_layout() {
        let row = json!({
            "f2": 1730750.0, "f3": -1.03, "f4": -180.68, "f6": 99038984169_i64,
            "f8": 1.67, "f12": "BK0493", "f14": "新能源", "f20": 1000000000000_i64,
            "f104": 47, "f105": 80, "f128": "天顺风能", "f136": 10.0
        });
        let parsed = parse_summary(SectorKind::Concept, &row, 1).unwrap();
        assert_eq!(parsed.code, "BK0493");
        assert_eq!(parsed.change_amount, Some(-180.68));
        assert_eq!(parsed.turnover_rate, Some(1.67));
        assert_eq!(parsed.up_count, Some(47));
        assert_eq!(parsed.down_count, Some(80));
        assert_eq!(parsed.leader_name.as_deref(), Some("天顺风能"));
    }

    #[test]
    fn parses_object_shaped_diff_from_legacy_gateway() {
        let rows = diff_rows(
            SectorKind::Industry,
            Some(json!({
                "0": {"f2": 1.0, "f3": 1.0, "f12": "BK1001", "f14": "示例板块"}
            })),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "BK1001");
    }

    #[test]
    fn rejects_non_sector_rows_and_empty_pages() {
        assert!(parse_summary(
            SectorKind::Industry,
            &json!({"f12":"600000","f14":"股票"}),
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

    #[test]
    fn derived_limit_up_excludes_no_limit_new_stock_prefixes() {
        let regular =
            parse_member(&json!({"f2":11.0,"f3":10.0,"f12":"600000","f14":"普通股票"})).unwrap();
        let new_stock =
            parse_member(&json!({"f2":20.0,"f3":45.0,"f12":"600001","f14":"N新股"})).unwrap();
        assert!(member_is_limit_up(&regular));
        assert!(!member_is_limit_up(&new_stock));
    }

    #[tokio::test]
    #[ignore = "需要东方财富实时网络"]
    async fn live_limit_up_snapshot_smoke() {
        let stats = fetch_limit_up_stats("BK0475").await.unwrap();
        assert!(stats.member_count > 0);
        assert!(stats.limit_up_count <= stats.member_count);
    }
}
