//! 东方财富行业 / 概念板块排行适配器。
//!
//! 一期负责板块列表、排行和按需成分股。历史行情留给后续按需接口，避免打开板块中心时
//! 为每个板块追加请求。字段按板块接口的 f-code 解析，而不是依赖返回数组顺序。

use crate::datasource::eastmoney_universe::universe_client;
use crate::datasource::headers::with_browser_headers;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 正式网关在部分网络会断开 TLS；push2test 是东财网页自身配置的备用网关。
/// 保留实际网关与行情时间，避免将请求时间当成报价时间。
const HOSTS: [&str; 4] = [
    "https://push2delay.eastmoney.com",
    "https://push2test.eastmoney.com",
    "https://push2.eastmoney.com",
    "https://82.push2.eastmoney.com",
];
static HOST_HINT: AtomicUsize = AtomicUsize::new(0);
const KLINE_HOSTS: [&str; 4] = [
    "https://push2his.eastmoney.com",
    "https://push2test.eastmoney.com",
    "https://17.push2his.eastmoney.com",
    "https://91.push2his.eastmoney.com",
];
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
    pub quoted_at_unix: Option<i64>,
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
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorCatalogItem {
    pub kind: SectorKind,
    pub code: String,
    pub name: String,
}

fn parse_filter_catalog(payload: &serde_json::Value) -> Vec<SectorCatalogItem> {
    let mut seen = HashSet::new();
    payload["bklist"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let kind = match row["type"].as_u64()? {
                2 => SectorKind::Industry,
                3 => SectorKind::Concept,
                _ => return None,
            };
            let code = row["code"].as_str()?;
            let name = row["name"].as_str()?.trim();
            if !valid_sector_code(code) || name.is_empty() || !seen.insert((kind, code.to_owned()))
            {
                return None;
            }
            Some(SectorCatalogItem {
                kind,
                code: code.into(),
                name: name.into(),
            })
        })
        .collect()
}

/// 筛选选项只需代码和名称，使用东财网页的目录，避免依赖实时资金排行。
pub async fn fetch_filter_catalog() -> Result<Vec<SectorCatalogItem>, String> {
    let response = with_browser_headers(
        universe_client().get("https://quote.eastmoney.com/center/api/sidemenu_new.json"),
        "https://quote.eastmoney.com/center/",
    )
    .timeout(REQUEST_TIMEOUT)
    .send()
    .await
    .map_err(|error| error.to_string())?
    .error_for_status()
    .map_err(|error| error.to_string())?;
    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    let items = parse_filter_catalog(&payload);
    if ![SectorKind::Industry, SectorKind::Concept]
        .iter()
        .all(|kind| items.iter().any(|item| item.kind == *kind))
    {
        return Err("行业/概念目录不完整，请重试".into());
    }
    Ok(items)
}

#[derive(Debug, thiserror::Error)]
enum SectorError {
    #[error("板块接口 HTTP {0}")]
    Status(u16),
    #[error("板块接口请求失败: {0}")]
    Http(#[from] reqwest::Error),
    #[error("板块接口没有返回有效数据")]
    Empty,
    #[error("板块数据不完整：返回 {received} / 应有 {total} 条")]
    Incomplete { received: usize, total: usize },
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

struct QuotePage<T> {
    items: Vec<T>,
    total: usize,
    as_of: String,
    source: String,
}

fn quote_time(rows: &[serde_json::Value]) -> String {
    rows.iter()
        .filter_map(|row| number_at(row, &["f124"]))
        .filter_map(|time| chrono::DateTime::from_timestamp(time as i64, 0))
        .max()
        .map(|time| {
            time.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "行情时间未返回".into())
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

#[cfg(test)]
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
        quoted_at_unix: number_at(row, &["f124"]).filter(|t|t.is_finite()).map(|t|t as i64),
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

async fn fetch_clist<T>(
    filter: &str,
    field: &str,
    fields: &str,
    page: u32,
    page_size: u32,
    parse: impl Fn(&serde_json::Value, usize) -> Option<T>,
) -> Result<QuotePage<T>, SectorError> {
    let params = [
        ("pn", page.to_string()),
        ("pz", page_size.to_string()),
        ("po", "1".to_owned()),
        ("np", "1".to_owned()),
        ("ut", UT_TOKEN.to_owned()),
        ("fltt", "2".to_owned()),
        ("invt", "2".to_owned()),
        ("fid", field.to_owned()),
        ("fs", filter.to_owned()),
        ("fields", format!("{fields},f124")),
    ];
    let mut last_error = SectorError::Empty;
    let hint = HOST_HINT.load(Ordering::Relaxed) % HOSTS.len();
    for round in 0..RETRY_ROUNDS {
        for offset in 0..HOSTS.len() {
            let index = (hint + offset) % HOSTS.len();
            let host = HOSTS[index];
            let url = format!("{host}/api/qt/clist/get");
            let result = async {
                let response = with_browser_headers(
                    universe_client().get(&url),
                    "https://quote.eastmoney.com/",
                )
                .query(&params)
                .timeout(REQUEST_TIMEOUT)
                .send()
                .await?;
                if !response.status().is_success() {
                    return Err(SectorError::Status(response.status().as_u16()));
                }
                let data = response
                    .json::<ClistResponse>()
                    .await?
                    .data
                    .ok_or(SectorError::Empty)?;
                let total = data.total.ok_or(SectorError::Empty)? as usize;
                let raw = values_from_diff(data.diff);
                let as_of = quote_time(&raw);
                let items: Vec<_> = raw
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| {
                        parse(
                            row,
                            (page.saturating_sub(1) as usize * page_size as usize) + i + 1,
                        )
                    })
                    .collect();
                if items.len() != raw.len()
                    || (items.is_empty()
                        && page.saturating_sub(1) as usize * (page_size as usize) < total)
                {
                    return Err(SectorError::Empty);
                }
                Ok(QuotePage {
                    items,
                    total,
                    as_of,
                    source: host.trim_start_matches("https://").to_owned(),
                })
            }
            .await;
            match result {
                Ok(value) => {
                    HOST_HINT.store(index, Ordering::Relaxed);
                    return Ok(value);
                }
                Err(error) => last_error = error,
            }
        }
        if round + 1 < RETRY_ROUNDS {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    Err(last_error)
}

async fn fetch_page_uncached(
    kind: SectorKind,
    page: u32,
    page_size: u32,
) -> Result<QuotePage<SectorSummary>, SectorError> {
    fetch_clist(kind.market_filter(), kind.rank_field(),
        "f1,f2,f3,f4,f5,f6,f8,f9,f12,f14,f20,f23,f62,f66,f72,f78,f84,f104,f105,f109,f127,f128,f136,f160,f184",
        page, page_size, |row, rank| parse_summary(kind, row, rank)).await
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
) -> Result<QuotePage<SectorMember>, SectorError> {
    fetch_clist(
        &format!("b:{sector_code} f:!50"),
        "f12",
        "f2,f3,f4,f6,f8,f9,f12,f14,f20,f23",
        page,
        page_size,
        |row, _| parse_member(row),
    )
    .await
}

fn require_complete(received: usize, total: usize) -> Result<(), SectorError> {
    if received == total && total > 0 {
        Ok(())
    } else {
        Err(SectorError::Incomplete { received, total })
    }
}

/// 网关可能将 pz=5000 截成 100 条；必须补齐后再筛选或统计。
async fn fetch_all_members(sector_code: &str) -> Result<QuotePage<SectorMember>, SectorError> {
    let mut result = fetch_member_page_uncached(sector_code, 1, 5_000).await?;
    let received = result.items.len();
    if received > 0 && received < result.total {
        for page in 2..=result.total.div_ceil(received).min(100) {
            let next =
                fetch_member_page_uncached(sector_code, page as u32, received as u32).await?;
            require_complete(next.total, result.total)?;
            result.items.extend(next.items);
        }
    }
    let mut seen = HashSet::new();
    result.items.retain(|item| seen.insert(item.code.clone()));
    require_complete(result.items.len(), result.total)?;
    Ok(result)
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

async fn fetch_catalog(kind: SectorKind) -> Result<QuotePage<SectorSummary>, SectorError> {
    let mut result = fetch_page_uncached(kind, 1, PAGE_SIZE).await?;
    let pages = (result.total as u32).div_ceil(PAGE_SIZE).min(MAX_PAGES);
    for page in 2..=pages {
        let next = fetch_page_uncached(kind, page, PAGE_SIZE).await?;
        require_complete(next.total, result.total)?;
        result.items.extend(next.items);
    }
    let mut seen = HashSet::new();
    result.items.retain(|row| seen.insert(row.code.clone()));
    require_complete(result.items.len(), result.total)?;
    sort_summaries(&mut result.items);
    for (i, row) in result.items.iter_mut().enumerate() {
        row.rank = i as u32 + 1;
    }
    Ok(result)
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
            .map(|mut result| {
                sort_summaries(&mut result.items);
                SectorSummaryPage {
                    kind,
                    page,
                    page_size,
                    total: result.total,
                    items: result.items,
                    as_of: result.as_of,
                    source: result.source,
                    stale: false,
                }
            })
    } else {
        fetch_catalog(kind).await.map(|result| {
            let filtered: Vec<_> = result
                .items
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
                as_of: result.as_of,
                source: result.source,
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
        .map(|result| SectorMemberPage {
            kind,
            sector_code: sector_code.clone(),
            page,
            page_size,
            total: result.total,
            items: result.items,
            as_of: result.as_of,
            source: result.source,
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
/// 与涨停统计共用完整成分分页校验，以 `MEMBER_TTL` 缓存完整代码集合。
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

    let result = fetch_all_members(&sector_code)
        .await
        .map_err(|error| error.to_string())?;
    let value: HashSet<String> = result.items.into_iter().map(|item| item.code).collect();
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

/// 主线研究必须拿全分页与行情时点，不能把单页或代码集合当完整覆盖。
pub async fn fetch_research_members(kind: SectorKind, sector_code: &str) -> Result<SectorMemberPage, String> {
    let code = sector_code.trim().to_uppercase();
    if !valid_sector_code(&code) { return Err("板块代码格式无效".into()); }
    let result = fetch_all_members(&code).await.map_err(|e| e.to_string())?;
    Ok(SectorMemberPage { kind, sector_code: code, page: 1, page_size: result.total as u32,
        total: result.total, items: result.items, as_of: result.as_of, source: result.source, stale: false })
}

/// 取齐全部成分后计算派生数量；缺页或重复时拒绝返回局部统计。
pub async fn fetch_limit_up_stats(sector_code: &str) -> Result<SectorLimitUpStats, String> {
    let sector_code = sector_code.trim().to_uppercase();
    if !valid_sector_code(&sector_code) {
        return Err("板块代码格式无效".into());
    }
    let result = fetch_all_members(&sector_code)
        .await
        .map_err(|error| error.to_string())?;
    let items = result.items;
    let limit_up_count = items.iter().filter(|item| member_is_limit_up(item)).count();
    Ok(SectorLimitUpStats {
        sector_code,
        limit_up_count,
        member_count: items.len(),
        as_of: result.as_of,
        source: format!("{} · derived", result.source),
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
    let bar=SectorKline {
        date: values[0].to_owned(),
        open: parse(1)?,
        close: parse(2)?,
        high: parse(3)?,
        low: parse(4)?,
        volume: parse(5)?,
        amount: parse(6)?,
        change_pct: parse(8),
        turnover_rate: parse(10),
    };
    if chrono::NaiveDate::parse_from_str(&bar.date,"%Y-%m-%d").is_err()
        || [bar.open,bar.high,bar.low,bar.close].iter().any(|v|!v.is_finite()||*v<=0.0)
        || bar.high<bar.open.max(bar.close)||bar.low>bar.open.min(bar.close)||bar.low>bar.high
        || !bar.volume.is_finite()||bar.volume<0.0||!bar.amount.is_finite()||bar.amount<0.0{return None;}
    Some(bar)
}

fn parse_history_payload(
    sector_code: &str,
    period: &str,
    host: &str,
    value: &serde_json::Value,
) -> Result<SectorHistory, String> {
    if value["data"].is_null() {
        return Err(format!("90.{sector_code} 官方没有返回原始日线（data=null）"));
    }
    if value["rc"].as_i64() != Some(0)
        || value["data"]["code"].as_str() != Some(sector_code)
        || value["data"]["market"].as_u64() != Some(90)
    {
        return Err(format!("板块 K 线未返回同一 90.{sector_code} 身份，不能替代或猜测映射"));
    }
    let rows = value.pointer("/data/klines").and_then(serde_json::Value::as_array)
        .filter(|rows| !rows.is_empty()).ok_or("板块 K 线接口没有返回原始行情")?;
    let items = rows.iter().map(|row| row.as_str().and_then(parse_kline)
        .ok_or("板块 K 线包含无效日期或 OHLC，拒绝丢行后继续计算".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    if items.windows(2).any(|r| r[0].date >= r[1].date) {
        return Err("板块 K 线包含重复或倒序日期".into());
    }
    Ok(SectorHistory {
        sector_code: sector_code.into(), period: period.into(),
        as_of: items.last().unwrap().date.clone(), items,
        source: format!("{} · 90.BK · unadjusted", host.trim_start_matches("https://")),
    })
}

fn validate_required_sessions(required_dates: &[String]) -> Result<(), String> {
    if required_dates.is_empty()
        || required_dates.iter().any(|date| date.len() != 10
            || chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err())
        || required_dates.windows(2).any(|dates| dates[0] >= dates[1])
    {
        return Err("必需交易日必须是非空、严格升序的 YYYY-MM-DD 真实指数日期".into());
    }
    Ok(())
}

fn missing_sessions<'a>(history: &SectorHistory, required_dates: &'a [String]) -> Vec<&'a str> {
    required_dates.iter().filter(|date| history.items.binary_search_by(|bar| bar.date.cmp(date)).is_err())
        .map(String::as_str).collect()
}

fn session_gap_error(history: &SectorHistory, required_dates: &[String], missing: &[&str]) -> String {
    let first = &history.items.first().unwrap().date;
    let short = if first > &required_dates[0] { "；当前官方可用历史短于必需窗口（short-history，未证实板块成立日）" } else { "" };
    format!("{} 缺少真实交易日 {} 等共 {} 日，覆盖 {}/{}（{}..{}）；{} 返回 {} 根原始行情（{}..{}）{}",
        history.sector_code, missing[0], missing.len(), required_dates.len() - missing.len(), required_dates.len(),
        required_dates.first().unwrap(), required_dates.last().unwrap(), history.source,
        history.items.len(), first, history.as_of, short)
}

fn complete_session_history(mut history: SectorHistory, required_dates: &[String]) -> Result<SectorHistory, String> {
    let missing = missing_sessions(&history, required_dates);
    if !missing.is_empty() { return Err(session_gap_error(&history, required_dates, &missing)); }
    // 换用单个完整窗口，避免把重叠价格不一致的源拼成一条历史。
    history.items.retain(|bar| bar.date >= required_dates[0] && bar.date <= *required_dates.last().unwrap());
    history.as_of = required_dates.last().unwrap().clone();
    Ok(history)
}

pub async fn fetch_history(sector_code: &str, period: &str) -> Result<SectorHistory, String> {
    fetch_history_inner(sector_code, period, &[]).await
}

/// 主线传入沪深300最后66根真实日线日期。缺交易日时继续官方网关和日期窗口；不填价、不映射SW。
/// 普通图表仍调用 fetch_history；本接口只返回覆盖完整必需日且不晚于截止日的原始BK日线。
pub async fn fetch_history_for_sessions(code: &str, required_dates: &[String]) -> Result<SectorHistory, String> {
    validate_required_sessions(required_dates)?;
    fetch_history_inner(code, "daily", required_dates).await
}

async fn request_history_window(
    sector_code: &str, period: &str, host: &str, window: usize,
    required_dates: &[String], timeout: Duration,
) -> Result<SectorHistory, (String, bool)> {
    let klt = match period { "daily" => "101", "weekly" => "102", _ => "103" };
    let end = if window == 0 { "20500101".into() } else { required_dates.last().unwrap().replace('-', "") };
    let lmt = if window == 2 { "1000" } else { "180" };
    let beg = if window == 1 { required_dates[0].replace('-', "") } else { "0".into() };
    let provenance = format!("{} · end={end} lmt={lmt} beg={beg}", host.trim_start_matches("https://"));
    if timeout.is_zero() { return Err((format!("{provenance}：官方兜底48秒预算用尽"), false)); }
    let mut query = vec![
        ("secid", format!("90.{sector_code}")), ("fields1", "f1,f2,f3,f4,f5,f6".into()),
        ("fields2", "f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61".into()),
        ("klt", klt.to_owned()), ("fqt", "0".into()), ("end", end), ("lmt", lmt.into()),
    ];
    if window > 0 {
        // 官方 quotekchart 的全量参数；1000根上限足以核验66日，避免无限请求。
        query.extend([("beg", beg), ("ut", "fa5fd1943c7b386f172d6893dbfba10b".into()), ("smplmt", "1000000".into())]);
    }
    // bool标记传输是否可用：断连/超时/HTTP失败不再对该主机重复参数窗口。
    let response = with_browser_headers(universe_client().get(format!("{host}/api/qt/stock/kline/get")),
        "https://quote.eastmoney.com/").query(&query).timeout(timeout).send().await
        .map_err(|error| (format!("{provenance}：板块 K 线请求失败：{error}"), false))?;
    let response = response.error_for_status()
        .map_err(|error| (format!("{provenance}：板块 K 线 HTTP失败：{error}"), false))?;
    let value: serde_json::Value = response.json().await
        .map_err(|error| (format!("{provenance}：板块 K 线解析失败：{error}"), !error.is_timeout()))?;
    let mut history = parse_history_payload(sector_code, period, host, &value)
        .map_err(|error| (format!("{provenance}：{error}"), true))?;
    if !required_dates.is_empty() { history.source = format!("{provenance} · 90.{sector_code} · unadjusted"); }
    Ok(history)
}

async fn fetch_history_inner(sector_code: &str, period: &str, required_dates: &[String]) -> Result<SectorHistory, String> {
    let sector_code = sector_code.trim().to_uppercase();
    if !valid_sector_code(&sector_code) { return Err("板块代码格式无效".into()); }
    if !matches!(period, "daily" | "weekly" | "monthly") {
        return Err("板块 K 线周期必须是 daily / weekly / monthly".into());
    }
    let mut last_error = "板块 K 线接口没有返回有效数据".to_owned();
    if required_dates.is_empty() {
        // 普通日/周/月图表保留原网关顺序、180根窗口及成功即返回。
        for host in KLINE_HOSTS {
            match request_history_window(&sector_code, period, host, 0, &[], REQUEST_TIMEOUT).await {
                Ok(history) => return Ok(history), Err((error, _)) => last_error = error,
            }
        }
        return Err(last_error);
    }
    // 先用本机实测可连通的官方网关；首请求完整时无额外HTTP。
    // 缺口后最多4条并发，每主机至多3窗口；48秒内归还具体诊断给命令层50秒总timeout。
    let deadline = Instant::now() + Duration::from_secs(48);
    let fetch = |index: usize, window: usize| {
        let code = sector_code.clone(); let dates = required_dates.to_vec(); let period = period.to_owned();
        async move {
            let timeout = deadline.saturating_duration_since(Instant::now()).min(REQUEST_TIMEOUT);
            let result = request_history_window(&code, &period, KLINE_HOSTS[index], window, &dates, timeout).await;
            (index, window, result)
        }
    };
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(fetch(1, 0));
    let mut attempts = vec![format!("{}#0", KLINE_HOSTS[1])];
    let mut fallback_started = false;
    let mut best_gap: Option<(usize, String)> = None;
    let mut data_error = None;
    while !tasks.is_empty() {
        let result = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), tasks.join_next()).await;
        let result = match result {
            Ok(Some(result)) => result.map_err(|error| format!("板块日线兜底任务失败：{error}"))?,
            Ok(None) => break,
            Err(_) => { last_error = "官方兜底48秒预算用尽，剩余请求已取消".into(); break; }
        };
        let (index, window, result) = result;
        let reachable = match result {
            Ok(history) => {
                let missing_count = missing_sessions(&history, required_dates).len();
                match complete_session_history(history, required_dates) {
                    Ok(history) => return Ok(history),
                    Err(error) => {
                        if best_gap.as_ref().is_none_or(|(count, _)| missing_count < *count) { best_gap = Some((missing_count, error)); }
                    }
                }
                true
            }
            Err((error, reachable)) => {
                if reachable { data_error = Some(error.clone()); }
                last_error = error; reachable
            }
        };
        if !fallback_started {
            for other in [0, 2, 3] {
                tasks.spawn(fetch(other, 0)); attempts.push(format!("{}#0", KLINE_HOSTS[other]));
            }
            fallback_started = true;
        }
        if reachable && window < 2 {
            tasks.spawn(fetch(index, window + 1)); attempts.push(format!("{}#{}", KLINE_HOSTS[index], window + 1));
        }
    }
    // JoinSet在返回/外层取消时中止剩余HTTP，不在后台继续探测或启动任何服务。
    Err(format!("板块 {sector_code} 官方日线兜底失败：{}；已尝试 {}（#0原180根/#1限定日期/#2宽1000根）；末次接口错误：{last_error}",
        best_gap.map(|(_, error)| error).or(data_error).unwrap_or_else(|| format!("没有获得同一BK身份的原始OHLC，无法核验 {} 个真实交易日", required_dates.len())), attempts.join(" / ")))
}

async fn fetch_rotation_kind(kind: SectorKind) -> Result<QuotePage<SectorSummary>, String> {
    // 东财单页最多约 100 条；复用排行目录的分页和多域名重试。
    let mut result = tokio::time::timeout(Duration::from_secs(30), fetch_catalog(kind))
        .await
        .map_err(|_| "板块目录请求超时".to_owned())?
        .map_err(|error| error.to_string())?;
    result.items.sort_by(|a, b| {
        b.main_net_inflow
            .unwrap_or(f64::NEG_INFINITY)
            .partial_cmp(&a.main_net_inflow.unwrap_or(f64::NEG_INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(result)
}

pub async fn fetch_rotation() -> SectorRotation {
    let (industry, concept) = tokio::join!(
        fetch_rotation_kind(SectorKind::Industry),
        fetch_rotation_kind(SectorKind::Concept)
    );
    let mut items = Vec::new();
    let mut statuses = Vec::with_capacity(2);
    let mut sources = Vec::new();
    for (kind, result) in [
        (SectorKind::Industry, industry),
        (SectorKind::Concept, concept),
    ] {
        match result {
            Ok(mut result) => {
                items.append(&mut result.items);
                sources.push(result.source);
                statuses.push(RotationStatus {
                    kind,
                    ok: true,
                    error: None,
                    as_of: result.as_of,
                });
            }
            Err(error) => statuses.push(RotationStatus {
                kind,
                ok: false,
                error: Some(error),
                as_of: String::new(),
            }),
        }
    }
    SectorRotation {
        items,
        statuses,
        source: {
            sources.sort();
            sources.dedup();
            sources.join(" / ")
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_partial_members_and_uses_quote_timestamp() {
        assert!(require_complete(100, 3874).is_err());
        assert!(require_complete(0, 0).is_err());
        assert!(require_complete(3874, 3874).is_ok());
        let rows = vec![json!({"f124": 1790235570_i64})];
        assert_eq!(
            quote_time(&rows),
            "2026-09-24 15:39:30"
        );
        assert_eq!(quote_time(&[json!({"f124": "-"})]), "行情时间未返回");
    }

    #[tokio::test]
    #[ignore = "需要东方财富实时网络；覆盖排行、成分、筛选代码、轮动与三种K线"]
    async fn live_sector_flow_smoke() {
        for kind in [SectorKind::Industry, SectorKind::Concept] {
            let page = fetch_summaries(kind, 1, 50, "", true).await.unwrap();
            assert_eq!(page.items.len(), 50);
            assert!(page.total > 100);
            assert_ne!(page.as_of, "行情时间未返回");
            eprintln!(
                "{kind:?}: {} sectors / {} / {}",
                page.total, page.source, page.as_of
            );
            let next = fetch_summaries(kind, 2, 50, "", true).await.unwrap();
            assert!(page
                .items
                .iter()
                .all(|row| next.items.iter().all(|other| other.code != row.code)));
        }
        let members = fetch_members(SectorKind::Industry, "BK0421", 1, 50, true)
            .await
            .unwrap();
        assert!(!members.items.is_empty());
        let codes = fetch_member_codes(SectorKind::Concept, "BK0596")
            .await
            .unwrap();
        assert!(codes.len() > 100, "大板块不能被截成第一页");
        eprintln!(
            "BK0421 members: {}; BK0596 complete codes: {}",
            members.total,
            codes.len()
        );
        let stats = fetch_limit_up_stats("BK0421").await.unwrap();
        assert_eq!(stats.member_count, members.total);
        let rotation = fetch_rotation().await;
        assert!(
            rotation.statuses.iter().all(|status| status.ok),
            "{:?}",
            rotation.statuses
        );
        assert!(rotation
            .items
            .iter()
            .any(|row| row.main_net_inflow.is_some()));
        eprintln!("rotation: {} / {}", rotation.items.len(), rotation.source);
        for period in ["daily", "weekly", "monthly"] {
            let history = fetch_history("BK0421", period).await.unwrap();
            assert!(!history.items.is_empty());
            eprintln!(
                "{period}: {} bars through {} / {}",
                history.items.len(),
                history.as_of,
                history.source
            );
        }
    }

    #[test]
    fn filter_catalog_keeps_industries_and_concepts_without_quotes() {
        let rows = parse_filter_catalog(&json!({"bklist": [
            {"type":1,"code":"BK0169","name":"四川板块"},
            {"type":2,"code":"BK0421","name":"铁路公路"},
            {"type":3,"code":"BK0955","name":"C2M概念"},
            {"type":2,"code":"BK0421","name":"铁路公路"},
            {"type":2,"code":"600000","name":"股票"},
            {"type":3,"code":"BK0493","name":" "}
        ]}));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, SectorKind::Industry);
        assert_eq!(rows[0].code, "BK0421");
        assert_eq!(rows[1].kind, SectorKind::Concept);
        assert_eq!(rows[1].name, "C2M概念");
    }

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
    fn daily_session_fallback_rejects_gap_stale_identity_and_broken_rows() {
        let dates = ["2026-06-30", "2026-07-01", "2026-07-02"].map(str::to_owned);
        assert!(validate_required_sessions(&dates).is_ok());
        for invalid in [vec![], vec!["2026-6-30".into()], vec!["2026-02-30".into()],
            vec![dates[1].clone(), dates[0].clone()], vec![dates[0].clone(); 2]] {
            assert!(validate_required_sessions(&invalid).is_err());
        }
        let row = |date: &str| format!("{date},100,102,103,99,10,1000,4,2,2,1");
        let payload = |days: &[&str]| json!({"rc":0,"data":{"code":"BK1305","market":90,
            "klines":days.iter().map(|date| row(date)).collect::<Vec<_>>()}});
        let parse = |value: &serde_json::Value| parse_history_payload("BK1305", "daily", "https://push2test.eastmoney.com", value);
        let gap = parse(&payload(&["2026-06-30", "2026-07-02"])).unwrap();
        assert_eq!(missing_sessions(&gap, &dates), ["2026-07-01"]);
        let error = complete_session_history(gap.clone(), &dates).unwrap_err();
        assert!(error.contains("2026-07-01") && error.contains("2/3"));
        let stale = parse(&payload(&["2026-06-30", "2026-07-01"])).unwrap();
        assert!(complete_session_history(stale.clone(), &dates).unwrap_err().contains("2026-07-02"));
        let full_payload = payload(&["2026-06-29", "2026-06-30", "2026-07-01", "2026-07-02", "2026-07-03"]);
        let full = parse(&full_payload).unwrap();
        let accepted = [gap, stale, full.clone()].into_iter()
            .find_map(|history| complete_session_history(history, &dates).ok()).unwrap();
        assert_eq!(accepted.items.iter().map(|bar| &bar.date).collect::<Vec<_>>(), dates.iter().collect::<Vec<_>>());
        assert_eq!(accepted.as_of, "2026-07-02");
        for bar in &accepted.items {
            let raw = full.items.iter().find(|raw| raw.date == bar.date).unwrap();
            assert_eq!([bar.open, bar.close, bar.high, bar.low, bar.volume, bar.amount],
                [raw.open, raw.close, raw.high, raw.low, raw.volume, raw.amount]);
        }
        for (field, value) in [("code", json!("BK1656")), ("code", json!("SW801150")), ("market", json!(1))] {
            let mut wrong = full_payload.clone(); wrong["data"][field] = value;
            assert!(parse(&wrong).is_err());
        }
        for rows in [vec![row("2026-07-01"), row("2026-06-30")], vec![row("2026-06-30"); 2],
            vec!["2026-06-30,100,102,101,99,10,1000".into()], vec!["2026-06-30,NaN,102,103,99,10,1000".into()]] {
            let mut bad = full_payload.clone(); bad["data"]["klines"] = json!(rows);
            assert!(parse(&bad).is_err());
        }
        let short = parse(&payload(&["2026-07-01", "2026-07-02"])).unwrap();
        assert!(complete_session_history(short, &dates).unwrap_err().contains("short-history"));
        // 普通周/月线仍保留完整原始窗口，不裁成主线日期集合。
        for period in ["weekly", "monthly"] {
            let history = parse_history_payload("BK1305", period, "https://push2test.eastmoney.com", &full_payload).unwrap();
            assert_eq!(history.period, period); assert_eq!(history.items.len(), 5); assert_eq!(history.as_of, "2026-07-03");
        }
    }

    #[tokio::test]
    #[ignore = "read-only official HTTP; requires the saved 17-failure discovery under target/mainline-check/sector-gap"]
    async fn live_daily_session_fallback_checks_all_17_failed_boards() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/mainline-check/sector-gap");
        let baseline: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("baseline-discovery.json")).unwrap()).unwrap();
        let day = baseline["as_of"].as_str().unwrap();
        let response = with_browser_headers(universe_client().get("https://web.ifzq.gtimg.cn/appstock/app/fqkline/get"), "https://gu.qq.com")
            .query(&[("param", format!("sh000300,day,,{day},100,qfq"))]).timeout(REQUEST_TIMEOUT)
            .send().await.unwrap().error_for_status().unwrap();
        let benchmark: serde_json::Value = response.json().await.unwrap();
        assert_eq!(benchmark["code"], 0);
        let index_rows = benchmark.pointer("/data/sh000300/day").and_then(serde_json::Value::as_array).unwrap();
        let mut dates = Vec::new();
        for row in index_rows {
            let date = row[0].as_str().unwrap();
            let prices: Vec<f64> = (1..5).map(|i| row[i].as_str().unwrap().parse().unwrap()).collect();
            assert!(prices.iter().all(|v| v.is_finite() && *v > 0.0));
            assert!(prices[2] >= prices[0].max(prices[1]) && prices[3] <= prices[0].min(prices[1]));
            if date <= day { dates.push(date.to_owned()); }
        }
        assert!(dates.len() >= 66); assert_eq!(dates.last().unwrap(), day);
        validate_required_sessions(&dates).unwrap();
        let required = dates[dates.len()-66..].to_vec();
        let saved: Vec<String> = serde_json::from_slice(&std::fs::read(root.join("required-dates.json")).unwrap()).unwrap();
        assert_eq!(required, saved, "live HS300 sessions must equal the saved real 66-day window");
        std::fs::write(root.join("live-benchmark.json"), serde_json::to_vec_pretty(&benchmark).unwrap()).unwrap();
        let control_before = fetch_history("BK1340", "daily").await.unwrap();
        let control = fetch_history_for_sessions("BK1340", &required).await.unwrap();
        assert!(missing_sessions(&control, &required).is_empty());
        for bar in &control.items {
            let raw = control_before.items.iter().find(|raw| raw.date == bar.date).unwrap();
            assert_eq!([bar.open, bar.close, bar.high, bar.low, bar.volume, bar.amount],
                [raw.open, raw.close, raw.high, raw.low, raw.volume, raw.amount]);
        }
        let mut ui_periods = Vec::new();
        for (period, klt) in [("weekly", "102"), ("monthly", "103")] {
            let history = fetch_history("BK1340", period).await.unwrap();
            let host = history.source.split(" · ").next().unwrap();
            let raw: serde_json::Value = with_browser_headers(universe_client().get(format!("https://{host}/api/qt/stock/kline/get")), "https://quote.eastmoney.com/")
                .query(&[("secid", "90.BK1340"), ("fields1", "f1,f2,f3,f4,f5,f6"),
                    ("fields2", "f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61"), ("klt", klt), ("fqt", "0"), ("end", "20500101"), ("lmt", "180")])
                .timeout(REQUEST_TIMEOUT).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
            let legacy_items: Vec<_> = raw["data"]["klines"].as_array().unwrap().iter()
                .filter_map(serde_json::Value::as_str).filter_map(parse_kline).collect();
            assert_eq!(serde_json::to_value(&history.items).unwrap(), serde_json::to_value(legacy_items).unwrap());
            std::fs::write(root.join(format!("live-{period}-raw.json")), serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
            ui_periods.push(history);
        }
        let failed = baseline["failed"].as_array().unwrap(); assert_eq!(failed.len(), 17);
        let mut tasks = tokio::task::JoinSet::new();
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
        for row in failed {
            let row = row.clone(); let required = required.clone(); let gate = gate.clone();
            tasks.spawn(async move {
                let _permit = gate.acquire_owned().await.unwrap();
                let code = row["code"].as_str().unwrap();
                let before = match fetch_history(code, "daily").await {
                    Ok(history) => json!({"history":history,"missing_dates":missing_sessions(&history, &required)}),
                    Err(error) => json!({"error":error}),
                };
                let after = match fetch_history_for_sessions(code, &required).await {
                    Ok(history) => {
                        assert_eq!(history.sector_code, code); assert_eq!(history.as_of, *required.last().unwrap());
                        assert!(missing_sessions(&history, &required).is_empty());
                        json!({"ok":true,"history":history})
                    },
                    Err(error) => json!({"ok":false,"error":error}),
                };
                json!({"code":code,"name":row["name"],"original_reason":row["reason"],"before":before,"after":after})
            });
        }
        let mut results = Vec::new();
        while let Some(result) = tasks.join_next().await { results.push(result.unwrap()); }
        results.sort_by(|a,b| a["code"].as_str().cmp(&b["code"].as_str()));
        let recovered = results.iter().filter(|row| row["after"]["ok"] == true).count();
        let report = json!({"observed_at":chrono::Utc::now().to_rfc3339(),"source":"read-only official Eastmoney raw BK OHLC; actual Tencent sh000300 daily sessions",
            "required_dates":required,"required_count":66,"recovered":recovered,"unresolved":17-recovered,"control":control,"ui_periods":ui_periods,"results":results});
        std::fs::write(root.join("live-17-results.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("official BK gap check: recovered {recovered}/17; control BK1340 covers all 66 real sessions; {}..{}; report={}",
            required[0], required.last().unwrap(), root.join("live-17-results.json").display());
        // Unavailable true prices are an expected live observation, not permission to lower the strategy window.
        assert_eq!(report["results"].as_array().unwrap().len(), 17);
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
