// src-tauri/src/datasource/sina_universe.rs
//! 新浪全市场快照适配器（东财 clist 的替代通道）。
//!
//! # 为什么需要它
//!
//! 实测发现东财 `push2.eastmoney.com/api/qt/clist/get` 在部分网络下会被**针对性阻断**
//! （同域的 `ulist.np` / `stock/get` 正常，只有 clist 路径在 TLS 重协商后断开连接），
//! 而 `clist` 正是全市场快照的唯一入口。一旦被阻断，筛选器与推荐榜会整体不可用。
//!
//! 新浪 `Market_Center.getHQNodeData` 提供了几乎同构的全市场列表能力（实测 5561 只，
//! 覆盖沪深 A 股 + 北交所），因此作为**首选的等价通道**。
//!
//! # 已实测确认的接口行为
//!
//! - 单页硬上限 100 条（`num=500/1000` 均只回 100）
//! - `node=hs_a` → 全市场 5561 只；`hs_bjs` = 北交所 343；`hs_b` = B 股 79
//! - **必须带 `Referer: https://finance.sina.com.cn/`**，否则 HTTP 200 但响应体为空
//! - 排序：`sort=amount&asc=0` 即按成交额降序（推荐榜用）
//! - 字段齐全度：现价 / 涨跌幅 / 成交额 / 总市值 / 流通市值 / 换手率 / PE / PB 都有；
//!   **缺「量比」与「涨速」**，本模块将它们置 0 并由上层标记为「当前数据源不支持」

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::eastmoney_universe::{Board, EmError, SnapshotRow};

/// 新浪行情列表接口基址
const SINA_BASE: &str = "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php";

/// 全市场节点：沪深 A 股 + 北交所
const SINA_NODE: &str = "hs_a";

/// 单页硬上限（实测）
const PAGE_SIZE: u32 = 100;

/// 分页安全上限
const MAX_PAGES: u32 = 80;

/// 并发路数。**必须保持很低**：新浪列表接口按 IP 限流，
/// 实测 4 路并发拉满 56 页后，本机 IP 会收到 HTTP 456「拒绝访问」（连 curl 一并被封）。
const CONCURRENCY: usize = 2;

/// 单页请求超时
const TIMEOUT: Duration = Duration::from_secs(20);

/// 每个请求的重试轮数
const RETRY_ROUNDS: usize = 2;

/// 新浪的风控状态码：返回 456 表示「拒绝访问」，此时**必须立刻停止后续请求**，
/// 否则会把 IP 封得更久。
const SINA_BLOCKED_STATUS: u16 = 456;

/// 判断响应是否为风控拦截
fn is_blocked_status(status: u16) -> bool {
    status == SINA_BLOCKED_STATUS
}

/// 排序字段
#[derive(Debug, Clone, Copy)]
pub enum SinaSort {
    /// 代码升序 —— 逐页遍历全市场，不重不漏
    Symbol,
    /// 成交额降序 —— 只取最活跃的一批
    AmountDesc,
}

impl SinaSort {
    fn query(self) -> (&'static str, &'static str) {
        match self {
            SinaSort::Symbol => ("symbol", "1"),
            SinaSort::AmountDesc => ("amount", "0"),
        }
    }
}

/// 新浪返回的一行。数字字段有时是字符串（`"13.550"`）有时是数字（`-2.448`），
/// 因此统一用 `Value` 接收再手工转换。
type RawRow = serde_json::Value;

fn str_field(v: &RawRow, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string().trim_matches('"').to_owned(),
    }
}

fn f64_field(v: &RawRow, key: &str) -> f64 {
    match v.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn parse_row(v: &RawRow) -> Option<SnapshotRow> {
    // symbol 形如 "sh600519" / "sz000001" / "bj920000"
    let symbol = str_field(v, "symbol").to_lowercase();
    let code = str_field(v, "code");
    let code = if code.len() == 6 {
        code
    } else if symbol.len() >= 8 {
        symbol[symbol.len() - 6..].to_owned()
    } else {
        return None;
    };
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let name = str_field(v, "name");
    if name.is_empty() {
        return None;
    }

    let price = f64_field(v, "trade");
    let prev_close = f64_field(v, "settlement");
    // volume 单位是「股」，东财 f5 是「手」，统一成手以免两源混用后口径不一致
    let volume = f64_field(v, "volume") / 100.0;
    let change_pct = f64_field(v, "changepercent");
    let high = f64_field(v, "high");
    let low = f64_field(v, "low");

    // 新浪不提供振幅，用 (高-低)/昨收 自行计算
    let amplitude_pct = if prev_close > 0.0 {
        (high - low) / prev_close * 100.0
    } else {
        0.0
    };

    let upper_name = name.to_uppercase();
    let board = Board::from_code(&code);
    let is_st = upper_name.contains("ST");
    let is_cdr = code.starts_with("689");

    Some(SnapshotRow {
        code,
        name,
        price,
        change_pct,
        change_amount: f64_field(v, "pricechange"),
        volume,
        // amount 单位已是「元」
        amount: f64_field(v, "amount"),
        amplitude_pct,
        turnover_rate: f64_field(v, "turnoverratio"),
        pe: f64_field(v, "per"),
        // 新浪列表接口不提供量比 / 涨速 / 5 分钟涨跌 / 60 日 / 年初至今
        volume_ratio: 0.0,
        change_5min: 0.0,
        high,
        low,
        open: f64_field(v, "open"),
        prev_close,
        // mktcap / nmc 单位是「万元」→ 元
        total_market_cap: f64_field(v, "mktcap") * 10_000.0,
        circulating_market_cap: f64_field(v, "nmc") * 10_000.0,
        speed: 0.0,
        pb: f64_field(v, "pb"),
        change_60d: 0.0,
        change_ytd: 0.0,
        listing_date: None,
        listed_days: None,
        // 新浪列表接口不提供行业 / 概念（东财 clist 的 f100 / f103）。
        // 前端据此显示 `--`，并由 `sector_supported` 明确提示「换个通道才有」。
        industry: String::new(),
        concepts: Vec::new(),
        board,
        is_st,
        is_delisting: upper_name.contains("退"),
        suspected_suspended: volume <= 0.0 || price <= 0.0,
        // 与东财通道共用同一套判定（阈值统一来自 market_rules，含主板 ST 的 10%）
        is_limit_locked: super::eastmoney_universe::is_limit_locked(
            change_pct,
            amplitude_pct,
            board,
            is_st,
        ),
        is_cdr,
    })
}

/// 专用 Client：与东财快照一样必须显式 `.no_proxy()`，
/// 否则系统代理会切断国内源连接。
fn sina_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .no_proxy()
        .pool_max_idle_per_host(8)
        .build()
        .expect("构建新浪快照 Client 失败")
}

/// 全市场股票总数
async fn fetch_count(client: &reqwest::Client) -> Result<u32, EmError> {
    let url = format!("{SINA_BASE}/Market_Center.getHQNodeStockCount?node={SINA_NODE}");
    let resp = client
        .get(&url)
        .header("Referer", "https://finance.sina.com.cn/")
        .send()
        .await?;

    let status = resp.status().as_u16();
    if is_blocked_status(status) {
        return Err(EmError::RateLimited(status));
    }

    let text = resp.text().await?;
    text.trim()
        .trim_matches('"')
        .parse::<u32>()
        .map_err(|_| EmError::EmptyPayload)
}

/// 拉取单页。空页返回空 Vec 而不是错误。
async fn fetch_page(
    client: &reqwest::Client,
    page: u32,
    sort: SinaSort,
) -> Result<Vec<SnapshotRow>, EmError> {
    let (sort_key, asc) = sort.query();
    let url = format!(
        "{SINA_BASE}/Market_Center.getHQNodeData?page={page}&num={PAGE_SIZE}\
         &sort={sort_key}&asc={asc}&node={SINA_NODE}"
    );

    let mut last: Option<EmError> = None;
    for round in 0..RETRY_ROUNDS {
        let attempt = async {
            let resp = client
                .get(&url)
                .header("Referer", "https://finance.sina.com.cn/")
                .send()
                .await
                .map_err(|source| EmError::Page { page, source })?;

            // 风控直接返回，不做重试 —— 重试只会加重封禁
            let status = resp.status().as_u16();
            if is_blocked_status(status) {
                return Err(EmError::RateLimited(status));
            }

            let text = resp
                .text()
                .await
                .map_err(|source| EmError::Page { page, source })?;
            // 空页 / 越界页返回空体或 `null`，都视为空而不是解析错误
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed == "null" {
                return Ok(Vec::new());
            }
            let rows: Vec<RawRow> = serde_json::from_str(trimmed)?;
            Ok(rows.iter().filter_map(parse_row).collect())
        }
        .await;

        match attempt {
            Ok(rows) => return Ok(rows),
            Err(error @ EmError::RateLimited(_)) => return Err(error),
            Err(error) => {
                log::warn!("新浪第 {page} 页失败（第 {} 轮）：{error}", round + 1);
                last = Some(error);
                if round + 1 < RETRY_ROUNDS {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }
    Err(last.unwrap_or(EmError::EmptyPayload))
}

/// 并发拉取第 `from..=to` 页并按页号归位。
///
/// **遇到风控立刻停止派发新页**：`abort` 标志一旦被任一页置位，
/// 余下的页就不再发起请求，避免把已受限的 IP 打得更死。
async fn fetch_page_range(
    client: &reqwest::Client,
    from: u32,
    to: u32,
    sort: SinaSort,
) -> Vec<(u32, Result<Vec<SnapshotRow>, EmError>)> {
    if from > to {
        return Vec::new();
    }
    let semaphore = Arc::new(Semaphore::new(CONCURRENCY));
    let http = Arc::new(client.clone());
    let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut set: JoinSet<(u32, Result<Vec<SnapshotRow>, EmError>)> = JoinSet::new();
    let mut skipped = 0usize;

    for page in from..=to {
        if abort.load(std::sync::atomic::Ordering::Relaxed) {
            skipped += 1;
            continue;
        }
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore 不会关闭");
        // 拿到许可后再检查一次，把等待期间新出现的风控也覆盖到
        if abort.load(std::sync::atomic::Ordering::Relaxed) {
            skipped += 1;
            drop(permit);
            continue;
        }
        let http = http.clone();
        let abort = abort.clone();
        set.spawn(async move {
            let result = fetch_page(&http, page, sort).await;
            if matches!(result, Err(EmError::RateLimited(_))) {
                abort.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            drop(permit);
            (page, result)
        });
    }

    if skipped > 0 {
        log::warn!("新浪快照被风控，跳过剩余 {skipped} 页");
    }

    let mut pages = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(item) => pages.push(item),
            Err(error) => log::warn!("新浪分页任务 panic: {error}"),
        }
    }
    pages.sort_by_key(|(page, _)| *page);
    pages
}

/// 拉取全市场快照（自动分页 + 有界并发）。
pub async fn fetch_market_snapshot(client: &reqwest::Client) -> Result<Vec<SnapshotRow>, EmError> {
    let total = fetch_count(client).await?;
    if total == 0 {
        return Err(EmError::EmptyPayload);
    }

    let total_pages = ((total + PAGE_SIZE - 1) / PAGE_SIZE).min(MAX_PAGES);
    let pages = fetch_page_range(client, 1, total_pages, SinaSort::Symbol).await;

    let mut all = Vec::new();
    let mut failed = 0usize;
    let mut blocked = false;
    for (page, result) in pages {
        match result {
            Ok(mut rows) => all.append(&mut rows),
            Err(EmError::RateLimited(status)) => {
                blocked = true;
                log::warn!("新浪快照第 {page} 页触发风控（HTTP {status}）");
            }
            Err(error) => {
                failed += 1;
                log::warn!("新浪快照第 {page} 页失败（已跳过）：{error}");
            }
        }
    }

    // 触发风控：立刻上报，让上层切换到东财通道，而不是继续用残缺数据
    if blocked {
        return Err(EmError::RateLimited(SINA_BLOCKED_STATUS));
    }

    if all.is_empty() {
        return Err(EmError::EmptyPayload);
    }

    // 失败页过多时数据不可信：宁可报错回退到别的通道，也不要给出残缺榜单
    if failed * 100 / (total_pages.max(1) as usize) > 20 {
        log::warn!("新浪快照失败页 {failed}/{total_pages}，超过 20%，放弃本次结果");
        return Err(EmError::EmptyPayload);
    }

    Ok(all)
}

/// 只拉「成交额最高的前 `max_pages` 页」。返回值已按成交额降序。
///
/// 推荐榜这类只要最活跃一批的场景用它，把 56 次请求压到几次。
pub async fn fetch_top_active(
    client: &reqwest::Client,
    max_pages: u32,
) -> Result<Vec<SnapshotRow>, EmError> {
    let max_pages = max_pages.clamp(1, MAX_PAGES);
    let pages = fetch_page_range(client, 1, max_pages, SinaSort::AmountDesc).await;

    let mut all = Vec::new();
    let mut blocked = false;
    for (page, result) in pages {
        match result {
            Ok(mut rows) => all.append(&mut rows),
            Err(EmError::RateLimited(_)) => blocked = true,
            Err(error) => log::warn!("新浪活跃股第 {page} 页失败（已跳过）：{error}"),
        }
    }
    if blocked {
        return Err(EmError::RateLimited(SINA_BLOCKED_STATUS));
    }
    if all.is_empty() {
        return Err(EmError::EmptyPayload);
    }

    // 失败页会破坏服务端排序，这里重排一次保证语义正确
    all.sort_by(|a, b| {
        b.amount
            .partial_cmp(&a.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(all)
}

/// 新浪通道使用的 Client（进程内复用连接池）
pub fn client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(sina_client)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(json: &str) -> Option<SnapshotRow> {
        let v: RawRow = serde_json::from_str(json).unwrap();
        parse_row(&v)
    }

    #[test]
    fn parses_sina_row_with_string_and_numeric_fields() {
        // trade 是字符串、changepercent 是数字 —— 两种都要能解析
        let r = row(
            r#"{"symbol":"sh600519","code":"600519","name":"贵州茅台","trade":"1500.00",
                "pricechange":12.5,"changepercent":0.84,"settlement":1487.5,
                "open":1490,"high":1510,"low":1470,"volume":1234500,"amount":1850000000,
                "per":30.2,"pb":9.1,"mktcap":188000000.0,"nmc":188000000.0,
                "turnoverratio":0.66}"#,
        )
        .unwrap();

        assert_eq!(r.code, "600519");
        assert_eq!(r.name, "贵州茅台");
        assert!((r.price - 1500.0).abs() < 1e-9);
        assert!((r.change_pct - 0.84).abs() < 1e-9);
        // 股 → 手
        assert!((r.volume - 12345.0).abs() < 1e-9);
        // 万元 → 元
        assert!((r.total_market_cap - 1.88e12).abs() < 1.0);
        assert_eq!(r.board, Board::ShMain);
        assert!(!r.is_st);
    }

    #[test]
    fn derives_board_and_flags_from_bj_and_st_names() {
        let bj = row(
            r#"{"symbol":"bj920000","code":"920000","name":"安徽凤凰","trade":"13.55",
                "changepercent":-2.45,"settlement":13.89,"volume":542801,"amount":7393476,
                "high":13.96,"low":13.49}"#,
        )
        .unwrap();
        assert_eq!(bj.board, Board::Bse);

        let st = row(
            r#"{"symbol":"sz000004","code":"000004","name":"ST国华","trade":"10.37",
                "changepercent":9.97,"settlement":9.43,"volume":100,"amount":1000,
                "high":10.37,"low":10.37}"#,
        )
        .unwrap();
        assert!(st.is_st);
        // 主板 ST 自 2026-07-06 起为 ±10%，9.97% 封板 + 振幅 0 → 一字板
        assert!(st.is_limit_locked);
        assert_eq!(st.board, Board::SzMain);

        // 同一只 ST 在旧口径（5%）下会被误判为封板，现在不算
        let st_flat = row(
            r#"{"symbol":"sz000004","code":"000004","name":"ST国华","trade":"9.90",
                "changepercent":4.98,"settlement":9.43,"volume":100,"amount":1000,
                "high":9.90,"low":9.90}"#,
        )
        .unwrap();
        assert!(
            !st_flat.is_limit_locked,
            "主板 ST 已并轨 10%，4.98% 不应判为一字板"
        );

        // 创业板 ±20%，10% 不算一字板
        let cn = row(
            r#"{"symbol":"sz300001","code":"300001","name":"特锐德","trade":"22.00",
                "changepercent":10.0,"settlement":20.0,"volume":100,"amount":1000,
                "high":22.0,"low":21.9}"#,
        )
        .unwrap();
        assert_eq!(cn.board, Board::ChiNext);
        assert!(!cn.is_limit_locked);
    }

    #[test]
    fn rejects_malformed_symbol() {
        assert!(row(r#"{"symbol":"xx","code":"12","name":"x"}"#).is_none());
        assert!(row(r#"{"symbol":"sh600519","code":"600519","name":""}"#).is_none());
    }

    #[test]
    fn amplitude_is_derived_from_prev_close() {
        let r = row(
            r#"{"symbol":"sz000001","code":"000001","name":"平安银行","trade":"10.00",
                "changepercent":0.0,"settlement":10.0,"high":10.5,"low":9.5,
                "volume":100,"amount":1000}"#,
        )
        .unwrap();
        // (10.5 - 9.5) / 10.0 * 100 = 10%
        assert!((r.amplitude_pct - 10.0).abs() < 1e-9);
    }
}
