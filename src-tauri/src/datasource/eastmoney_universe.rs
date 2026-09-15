//! 东方财富 · 全市场快照适配器（漏斗 L0 层）
//!
//! # 为什么不用 akshare-rs 的 `stock_zh_a_spot_em()`
//!
//! 实测（2026-09-12）该函数只能返回 **100 条**：
//! 它内部向 `clist_spot_fetch` 传了 `pz="5000"`，但 `util::eastmoney_clist_params`
//! 把 `pn` 硬编码为 `"1"`（只取第一页），而东财服务端又把单页上限锁死在 100 条。
//! 结果就是拿到降序排列的第一页（全是北交所 920xxx），无法用于全市场扫描。
//!
//! # 接口实测结论
//!
//! * `data.total` = 5913（全市场 A 股总数，含北交所）
//! * 单页硬上限 100 条（`pz=500` / `pz=1000` 均只回 100）
//! * 分页参数 `pn=1..60` 正常工作，最后一页 13 条
//! * 因此全市场快照 = 60 次请求；并发 6 路后耗时约 1-2 秒
//!
//! 本模块：分页拉取 → 解析 → 基础分类（板块 / ST / 退市 / 停牌嫌疑 / 一字板）
//!
//! # ⚠️ 调用方注意：必须绕开系统代理
//!
//! 实测踩坑（2026-09-12）：开发机存在系统代理（`HTTPS_PROXY` 环境变量）时，
//! `reqwest` 默认会把请求也送进代理，导致东财连接被中间层切断，报错：
//!
//! ```text
//! peer closed connection without sending TLS close_notify
//! ```
//!
//! 东财是国内数据源，**务必在构造 Client 时调用 `.no_proxy()`**：
//!
//! ```no_run
//! let client = reqwest::Client::builder().no_proxy().build()?;
//! ```
//!
//! 本项目 `datasource::shared_client()` 目前**没有**设置 `no_proxy()`，
//! 接入时需确认该共享 Client 是否会被代理影响，或为本模块单独建一个 Client。

use serde::Deserialize;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// 东财 clist 接口的可用域名。
///
/// 实测东财存在 **IP 级限流**（连续约 15 次请求后主域名返回 HTTP 000），
/// 且主域名偶发不可达。备用域名在实测中仍可通，因此必须轮换，
/// 否则「第 1 页就失败」会让整个全市场功能直接不可用。
const CLIST_HOSTS: [&str; 3] = [
    "https://push2.eastmoney.com",
    "https://82.push2.eastmoney.com",
    "https://push2delay.eastmoney.com",
];

/// 每个域名的重试轮数（一轮 = 依次试完 3 个域名）
const RETRY_ROUNDS: usize = 2;

/// 上一次成功使用的域名下标。命中后优先复用，避免每次都从被限流的域名开始试。
static HOST_HINT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 东财公开的 ut 校验位，长期固定
const UT_TOKEN: &str = "bd1d9ddb04089700cf9c27f6f7426281";

/// 单页硬上限（实测 pz 超过 100 无效）
const PAGE_SIZE: u32 = 100;

/// 分页安全上限，防止 total 异常时失控（5913 / 100 = 60 页）
const MAX_PAGES: u32 = 80;

/// 并发路数。不要调太高，避免触发风控
const CONCURRENCY: usize = 6;

/// 全 A 股市场过滤串（沪主板 + 科创 + 深主板 + 创业 + 北交所）
/// 与 akshare-rs 内部使用的过滤串一致，已实测返回 total=5913
const FS_ALL_A: &str =
    "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23,m:0+t:81+s:2048";

/// 请求字段。字段含义见模块末尾 `FIELD_MEANING` 注释
const FIELDS: &str = "f12,f14,f2,f3,f4,f5,f6,f7,f8,f9,f10,f11,f15,f16,f17,f18,f20,f21,f22,f23,f24,f25";

/// 单页请求超时
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);

// ── 错误 ──

#[derive(Debug, thiserror::Error)]
pub enum EmError {
    #[error("HTTP 请求失败: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("东财接口返回 data 为空（可能被风控或参数变更）")]
    EmptyPayload,
    #[error("分页请求失败: 第 {page} 页 -> {source}")]
    Page { page: u32, source: reqwest::Error },
    #[error("数据源触发风控（HTTP {0}），本机 IP 已被临时限制，请稍后再试")]
    RateLimited(u16),
}

// ── 分类 ──

/// A 股板块。判定优先使用代码前缀。
///
/// 注意：北交所代码段历史上调整过（新增 920 段），
/// 因此这里把已知段全部列出，无法归类的落到 [`Board::Other`] 以便监控漏网。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Board {
    /// 沪市主板（600/601/603/605）
    ShMain,
    /// 深市主板（000/001/002/003，含原中小板）
    SzMain,
    /// 创业板（300/301）
    ChiNext,
    /// 科创板（688/689）
    Star,
    /// 北交所（43x/83x/87x/920）
    Bse,
    /// B 股（200/900），默认应排除
    BShare,
    /// 未能归类
    Other,
}

impl Board {
    pub fn from_code(code: &str) -> Self {
        let c = code.trim();
        if c.starts_with("688") || c.starts_with("689") {
            return Self::Star;
        }
        if c.starts_with("300") || c.starts_with("301") {
            return Self::ChiNext;
        }
        if c.starts_with("600")
            || c.starts_with("601")
            || c.starts_with("603")
            || c.starts_with("605")
        {
            return Self::ShMain;
        }
        if c.starts_with("000")
            || c.starts_with("001")
            || c.starts_with("002")
            || c.starts_with("003")
        {
            return Self::SzMain;
        }
        if c.starts_with("200") || c.starts_with("900") {
            return Self::BShare;
        }
        // 北交所：43x（原新三板精选层）、83x、87x、920（新代码段）
        if c.starts_with("43")
            || c.starts_with("83")
            || c.starts_with("87")
            || c.starts_with("920")
        {
            return Self::Bse;
        }
        Self::Other
    }

    /// 中文名，用于 UI 与日志
    pub fn label(self) -> &'static str {
        match self {
            Self::ShMain => "沪市主板",
            Self::SzMain => "深市主板",
            Self::ChiNext => "创业板",
            Self::Star => "科创板",
            Self::Bse => "北交所",
            Self::BShare => "B股",
            Self::Other => "未识别",
        }
    }

    /// 是否属于「大 A 常用选股范围」（排除 B 股与未识别）
    pub fn is_tradable_a(self) -> bool {
        !matches!(self, Self::BShare | Self::Other)
    }
}

// ── L0 行 ──

/// 全市场快照的一行。字段名与东财 f-code 对应关系见 `FIELD_MEANING`。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SnapshotRow {
    /// 6 位代码（如 "600519"）
    pub code: String,
    /// 名称
    pub name: String,
    /// 最新价（元）
    pub price: f64,
    /// 涨跌幅 %
    pub change_pct: f64,
    /// 涨跌额
    pub change_amount: f64,
    /// 成交量（手）
    pub volume: f64,
    /// 成交额（元）
    pub amount: f64,
    /// 振幅 %
    pub amplitude_pct: f64,
    /// 换手率 %
    pub turnover_rate: f64,
    /// 市盈率（动态）
    pub pe: f64,
    /// 量比
    pub volume_ratio: f64,
    /// 5 分钟涨跌 %
    pub change_5min: f64,
    /// 最高
    pub high: f64,
    /// 最低
    pub low: f64,
    /// 今开
    pub open: f64,
    /// 昨收
    pub prev_close: f64,
    /// 总市值（元）
    pub total_market_cap: f64,
    /// 流通市值（元）
    pub circulating_market_cap: f64,
    /// 涨速 %
    pub speed: f64,
    /// 市净率
    pub pb: f64,
    /// 60 日涨跌幅 %
    pub change_60d: f64,
    /// 年初至今涨跌幅 %
    pub change_ytd: f64,

    // ── 派生分类（解析时一并算出，避免后续重复计算） ──
    /// 板块
    pub board: Board,
    /// 名称含 ST（含 *ST）
    pub is_st: bool,
    /// 名称含「退」（退市整理 / 退市）
    pub is_delisting: bool,
    /// 成交量为 0 或价格无效 → 停牌嫌疑
    pub suspected_suspended: bool,
    /// 一字板：涨跌幅接近 ±10% 且振幅极小
    pub is_limit_locked: bool,
}

impl SnapshotRow {
    /// 总市值（亿元），UI 友好
    pub fn market_cap_yi(&self) -> f64 {
        self.total_market_cap / 1e8
    }

    /// 是否通过「基础可用性」硬门槛：有成交、价格有效、非退市
    pub fn is_basically_tradable(&self) -> bool {
        self.price > 0.0
            && self.volume > 0.0
            && !self.is_delisting
            && self.board.is_tradable_a()
    }
}

// ── 原始响应结构 ──

#[derive(Debug, Deserialize)]
struct ClistResp {
    data: Option<ClistData>,
}

#[derive(Debug, Deserialize)]
struct ClistData {
    total: u32,
    /// 空结果时东财返回 `{}` 而非 `[]`，故用 Value 兜底
    diff: Option<serde_json::Value>,
}

// ── 解析辅助 ──

/// 东财在停牌 / 无数据时返回字符串 "-"，需按 0 处理
fn f64_of(v: &serde_json::Value, key: &str) -> f64 {
    match v.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(s)) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// 判断是否「一字板」——开盘即封板、全天几乎没有波动，属于**买不进**的票。
///
/// 涨跌幅限制按板块不同，必须分开算：
/// - 主板（沪/深）±10%，其中 **ST 股只有 ±5%**
/// - 创业板 / 科创板 ±20%
/// - 北交所 ±30%
///
/// 早期版本用统一的 9.8% 阈值，会把 ST 的 5% 一字板漏掉（4.98% < 9.8%）。
/// 两个数据源（东财 / 新浪）共用本函数，保证口径一致。
pub fn is_limit_locked(change_pct: f64, amplitude_pct: f64, board: Board, is_st: bool) -> bool {
    let limit = match board {
        Board::ChiNext | Board::Star => 20.0,
        Board::Bse => 30.0,
        _ if is_st => 5.0,
        _ => 10.0,
    };
    change_pct.abs() >= limit - 0.2 && amplitude_pct <= 1.0
}

fn str_of(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string().trim_matches('"').to_owned(),
        None => String::new(),
    }
}

fn parse_row(v: &serde_json::Value) -> Option<SnapshotRow> {
    let code = str_of(v, "f12");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let name = str_of(v, "f14");
    if name.is_empty() {
        return None;
    }

    let price = f64_of(v, "f2");
    let volume = f64_of(v, "f5");
    let amplitude_pct = f64_of(v, "f7");
    let change_pct = f64_of(v, "f3");

    let board = Board::from_code(&code);
    let upper_name = name.to_uppercase();
    let is_st = upper_name.contains("ST");

    Some(SnapshotRow {
        code,
        name,
        price,
        change_pct,
        change_amount: f64_of(v, "f4"),
        volume,
        amount: f64_of(v, "f6"),
        amplitude_pct,
        turnover_rate: f64_of(v, "f8"),
        pe: f64_of(v, "f9"),
        volume_ratio: f64_of(v, "f10"),
        change_5min: f64_of(v, "f11"),
        high: f64_of(v, "f15"),
        low: f64_of(v, "f16"),
        open: f64_of(v, "f17"),
        prev_close: f64_of(v, "f18"),
        total_market_cap: f64_of(v, "f20"),
        circulating_market_cap: f64_of(v, "f21"),
        speed: f64_of(v, "f22"),
        pb: f64_of(v, "f23"),
        change_60d: f64_of(v, "f24"),
        change_ytd: f64_of(v, "f25"),
        board,
        is_st,
        is_delisting: upper_name.contains("退"),
        suspected_suspended: volume <= 0.0 || price <= 0.0,
        is_limit_locked: is_limit_locked(change_pct, amplitude_pct, board, is_st),
    })
}

// ── 分页拉取 ──

/// 排序字段（东财 `fid`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    /// f12 = 代码，升序。分页最稳定，用于全市场遍历
    Code,
    /// f6 = 成交额，**降序**。用于「只要最活跃的一批」的轻量扫描
    AmountDesc,
}

impl SortBy {
    fn fid(self) -> &'static str {
        match self {
            SortBy::Code => "f12",
            SortBy::AmountDesc => "f6",
        }
    }

    /// `po`：0 = 升序，1 = 降序
    fn po(self) -> &'static str {
        match self {
            SortBy::Code => "0",
            SortBy::AmountDesc => "1",
        }
    }
}

fn page_params(page: u32, sort: SortBy) -> Vec<(&'static str, String)> {
    vec![
        ("pn", page.to_string()),
        ("pz", PAGE_SIZE.to_string()),
        ("po", sort.po().to_owned()),
        ("np", "1".to_owned()),
        ("ut", UT_TOKEN.to_owned()),
        ("fltt", "2".to_owned()),
        ("invt", "2".to_owned()),
        ("fid", sort.fid().to_owned()),
        ("fs", FS_ALL_A.to_owned()),
        ("fields", FIELDS.to_owned()),
    ]
}

/// 从 `data.diff` 里解析出行。空页返回 `{}`/`null` 时视为空而非错误。
fn rows_of(data: &ClistData) -> Vec<SnapshotRow> {
    match &data.diff {
        Some(serde_json::Value::Array(items)) => items.iter().filter_map(parse_row).collect(),
        _ => Vec::new(),
    }
}

/// 拉单页原始结构：**域名轮换 + 重试**。
///
/// 只要任一域名在任一轮成功即返回；全部失败才返回最后一个错误。
/// 这是全市场功能抗限流 / 抗单点网络故障的关键。
async fn request_clist(
    client: &reqwest::Client,
    page: u32,
    sort: SortBy,
) -> Result<ClistData, EmError> {
    let start = HOST_HINT.load(std::sync::atomic::Ordering::Relaxed) % CLIST_HOSTS.len();
    let mut last: Option<EmError> = None;

    for round in 0..RETRY_ROUNDS {
        for offset in 0..CLIST_HOSTS.len() {
            let index = (start + offset) % CLIST_HOSTS.len();
            let url = format!("{}/api/qt/clist/get", CLIST_HOSTS[index]);
            let params = page_params(page, sort);

            let result = async {
                let resp = client
                    .get(&url)
                    .header("Referer", "https://quote.eastmoney.com/")
                    .query(&params)
                    .timeout(PAGE_TIMEOUT)
                    .send()
                    .await
                    .map_err(|source| EmError::Page { page, source })?;
                let body: ClistResp = resp.json().await?;
                body.data.ok_or(EmError::EmptyPayload)
            }
            .await;

            match result {
                Ok(data) => {
                    HOST_HINT.store(index, std::sync::atomic::Ordering::Relaxed);
                    return Ok(data);
                }
                Err(error) => {
                    log::warn!(
                        "东财 {}/ 第 {} 页请求失败（第 {} 轮）：{}",
                        CLIST_HOSTS[index],
                        page,
                        round + 1,
                        error
                    );
                    last = Some(error);
                }
            }
        }
        // 整轮都失败说明大概率是限流，退避一下再试下一轮
        if round + 1 < RETRY_ROUNDS {
            tokio::time::sleep(Duration::from_millis(300 * (round as u64 + 1))).await;
        }
    }

    Err(last.unwrap_or(EmError::EmptyPayload))
}

/// 并发拉取第 `from..=to` 页，返回按页号升序排好的结果。
/// 单页失败不会中断其它页，由调用方决定容忍度。
async fn fetch_page_range(
    client: &reqwest::Client,
    from: u32,
    to: u32,
    sort: SortBy,
) -> Vec<(u32, Result<Vec<SnapshotRow>, EmError>)> {
    if from > to {
        return Vec::new();
    }
    let semaphore = Arc::new(Semaphore::new(CONCURRENCY));
    let client = Arc::new(client.clone());
    let mut join_set: JoinSet<(u32, Result<Vec<SnapshotRow>, EmError>)> = JoinSet::new();

    for page in from..=to {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore 不会关闭");
        let client = client.clone();
        join_set.spawn(async move {
            let result = match request_clist(&client, page, sort).await {
                Ok(data) => Ok(rows_of(&data)),
                Err(error) => Err(error),
            };
            drop(permit);
            (page, result)
        });
    }

    let mut pages: Vec<(u32, Result<Vec<SnapshotRow>, EmError>)> = Vec::new();
    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(item) => pages.push(item),
            Err(join_error) => {
                log::warn!("快照分页任务 panic: {join_error}");
            }
        }
    }
    // 按页号归位，保证顺序稳定
    pages.sort_by_key(|(page, _)| *page);
    pages
}

/// 拉取全市场快照（自动分页 + 有界并发）。
///
/// 先请求第 1 页取得 `total`，据此算出总页数，再并发拉取剩余页。
/// 返回顺序按页码拼接，保证结果稳定。
pub async fn fetch_market_snapshot(
    client: &reqwest::Client,
) -> Result<Vec<SnapshotRow>, EmError> {
    // 1. 第一页 —— 同时拿到 total（失败会自动轮换域名重试）
    let first = request_clist(client, 1, SortBy::Code).await?;
    let total = first.total;
    let first_rows = rows_of(&first);

    if total == 0 {
        return Ok(first_rows);
    }

    // 2. 计算总页数（向上取整），并夹在安全上限内
    let total_pages = ((total + PAGE_SIZE - 1) / PAGE_SIZE).min(MAX_PAGES);
    if total_pages <= 1 {
        return Ok(first_rows);
    }

    // 3. 并发拉取第 2..=total_pages 页
    let pages = fetch_page_range(client, 2, total_pages, SortBy::Code).await;

    let mut all = first_rows;
    let mut errors = Vec::new();
    for (page, result) in pages {
        match result {
            Ok(mut rows) => all.append(&mut rows),
            Err(error) => errors.push((page, error)),
        }
    }

    // 部分页失败不致命，但必须可见 —— 数据不全时上层不应误判
    if !errors.is_empty() {
        let pages_desc: Vec<String> = errors
            .iter()
            .map(|(page, error)| format!("第{page}页:{error}"))
            .collect();
        log::warn!(
            "快照有 {} 页失败（共 {} 页），返回 {} 条：{}",
            errors.len(),
            total_pages,
            all.len(),
            pages_desc.join("; ")
        );
    }

    Ok(all)
}

/// 只拉「成交额最高的前 `max_pages` 页」（每页 100 只）。
///
/// 推荐榜、快捷扫描这类场景只需要最活跃的一批股票：全市场 60 页里，
/// 成交额排前面的几十只几乎必然落在前几页。用它把 60 次请求压到几次，
/// 既快，又极大降低触发东财 IP 限流的概率（限流是全市场功能最主要的失败原因）。
///
/// 返回值**已按成交额降序**（服务端排序），调用方无需再排一次。
pub async fn fetch_top_active(
    client: &reqwest::Client,
    max_pages: u32,
) -> Result<Vec<SnapshotRow>, EmError> {
    let max_pages = max_pages.clamp(1, MAX_PAGES);

    let first = request_clist(client, 1, SortBy::AmountDesc).await?;
    let mut all = rows_of(&first);
    let total = first.total;
    if total == 0 {
        return Ok(all);
    }

    let total_pages = ((total + PAGE_SIZE - 1) / PAGE_SIZE)
        .min(MAX_PAGES)
        .min(max_pages);
    if total_pages <= 1 {
        return Ok(all);
    }

    for (page, result) in fetch_page_range(client, 2, total_pages, SortBy::AmountDesc).await {
        match result {
            Ok(mut rows) => all.append(&mut rows),
            Err(error) => log::warn!("活跃股扫描第 {page} 页失败（已跳过）：{error}"),
        }
    }

    // 兜底：并发归位后顺序是对的，但跳过失败页可能乱序，这里再排一次保证语义
    all.sort_by(|a, b| b.amount.partial_cmp(&a.amount).unwrap_or(std::cmp::Ordering::Equal));
    Ok(all)
}

/// 按板块统计，用于设置页展示与自检
pub fn count_by_board(rows: &[SnapshotRow]) -> Vec<(Board, usize)> {
    let mut counts: std::collections::HashMap<Board, usize> = std::collections::HashMap::new();
    for row in rows {
        *counts.entry(row.board).or_insert(0) += 1;
    }
    let mut list: Vec<(Board, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1));
    list
}

// ── 专用 HTTP Client（必须不走代理） ──

static UNIVERSE_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// 全市场快照专用 Client。
///
/// 与 `datasource::shared_client()` 的关键区别：**显式 `.no_proxy()`**。
/// 开发机若存在系统代理（`HTTPS_PROXY`），共享 Client 会把东财请求也送进代理，
/// 导致 TLS 被中间层切断（实测报 `peer closed connection without sending TLS close_notify`）。
/// 东财是国内源，直连才是正确姿势。
pub fn universe_client() -> &'static reqwest::Client {
    UNIVERSE_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .no_proxy()
            .pool_max_idle_per_host(10)
            .build()
            .expect("构建全市场快照 Client 失败")
    })
}

// ── 通道选择：新浪 / 东财（互为兜底） ──

/// 全市场快照的取数通道。
///
/// 两源的字段能力不同，必须让上层知道数据来自哪里：
/// - 东财 `clist` 字段最全（含量比、涨速），能扛并发
/// - 新浪 `Market_Center` 覆盖沪深 A + 北交所，但**没有量比**，且**按 IP 限流**
///   （高频拉取会收到 HTTP 456「拒绝访问」）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    Sina,
    Eastmoney,
}

impl SnapshotSource {
    /// 该通道是否提供「量比」
    pub fn has_volume_ratio(self) -> bool {
        matches!(self, SnapshotSource::Eastmoney)
    }

    /// 该通道是否提供「60 日涨跌幅 / 年初至今涨跌幅」。
    ///
    /// 只有东财的 clist 带 f24 / f25；新浪列表接口不提供，解析时记 0。
    /// 依赖这两项的策略（如反转、趋势确认）在新浪通道上必须**跳过该条件并明示用户**，
    /// 否则会拿 0 去比较，把结果筛成空集。
    pub fn has_change_60d(self) -> bool {
        matches!(self, SnapshotSource::Eastmoney)
    }

    pub fn label(self) -> &'static str {
        match self {
            SnapshotSource::Sina => "新浪财经",
            SnapshotSource::Eastmoney => "东方财富",
        }
    }

    fn other(self) -> Self {
        match self {
            SnapshotSource::Sina => SnapshotSource::Eastmoney,
            SnapshotSource::Eastmoney => SnapshotSource::Sina,
        }
    }
}

/// 解析设置项 `universe_source`（auto / sina / eastmoney）。
///
/// `auto` 表示「东财优先，新浪兜底」——东财字段最全且能扛并发；
/// 新浪列表接口按 IP 限流，长时间高频拉取会被封。
pub fn preferred_source(setting: Option<&str>) -> Option<SnapshotSource> {
    match setting.map(str::trim) {
        Some("sina") => Some(SnapshotSource::Sina),
        Some("eastmoney") => Some(SnapshotSource::Eastmoney),
        _ => None, // auto
    }
}

/// 按指定通道拉全市场快照
pub async fn fetch_snapshot_from(
    source: SnapshotSource,
) -> Result<Vec<SnapshotRow>, EmError> {
    match source {
        SnapshotSource::Sina => {
            crate::datasource::sina_universe::fetch_market_snapshot(
                crate::datasource::sina_universe::client(),
            )
            .await
        }
        SnapshotSource::Eastmoney => fetch_market_snapshot(universe_client()).await,
    }
}

/// 带通道回退的全市场快照。
///
/// 依次尝试：指定通道（若有）→ 另一通道。任一成功即返回，并告知数据来源。
///
/// **默认顺序是「东财优先，新浪兜底」**，这是实测结论而非偏好：
/// - 东财 `clist` 字段最全（含量比、涨速），且能承受 6 路并发
/// - 新浪列表接口**按 IP 限流**：用 4 路并发拉 56 页后本机 IP 会收到 HTTP 456「拒绝访问」，
///   连 curl 也会一起被封一段时间。只在东财不可用时才用它
///   （新浪的**日K**接口在另一个域名上，没有这个问题）
pub async fn fetch_snapshot_auto(
    preferred: Option<SnapshotSource>,
) -> Result<(Vec<SnapshotRow>, SnapshotSource), EmError> {
    let order: [SnapshotSource; 2] = match preferred {
        Some(source) => [source, source.other()],
        None => [SnapshotSource::Eastmoney, SnapshotSource::Sina],
    };

    let mut last: Option<EmError> = None;
    for source in order {
        match fetch_snapshot_from(source).await {
            Ok(rows) if !rows.is_empty() => return Ok((rows, source)),
            Ok(_) => {
                last = Some(EmError::EmptyPayload);
                log::warn!("{} 返回空快照，尝试下一个通道", source.label());
            }
            Err(error) => {
                log::warn!("{} 快照失败：{error}", source.label());
                last = Some(error);
            }
        }
    }
    Err(last.unwrap_or(EmError::EmptyPayload))
}

/// 带通道回退的「最活跃一批」拉取（推荐榜用）。
///
/// 只要几页而不是全市场，失败成本低，因此不做缓存。顺序同 [`fetch_snapshot_auto`]。
pub async fn fetch_top_active_auto(
    pages: u32,
    preferred: Option<SnapshotSource>,
) -> Result<(Vec<SnapshotRow>, SnapshotSource), EmError> {
    let order: [SnapshotSource; 2] = match preferred {
        Some(source) => [source, source.other()],
        None => [SnapshotSource::Eastmoney, SnapshotSource::Sina],
    };

    let mut last: Option<EmError> = None;
    for source in order {
        let result = match source {
            SnapshotSource::Sina => {
                crate::datasource::sina_universe::fetch_top_active(
                    crate::datasource::sina_universe::client(),
                    pages,
                )
                .await
            }
            SnapshotSource::Eastmoney => fetch_top_active(universe_client(), pages).await,
        };
        match result {
            Ok(rows) if !rows.is_empty() => return Ok((rows, source)),
            Ok(_) => last = Some(EmError::EmptyPayload),
            Err(error) => {
                log::warn!("{} 活跃股扫描失败：{error}", source.label());
                last = Some(error);
            }
        }
    }
    Err(last.unwrap_or(EmError::EmptyPayload))
}

// ── 带 TTL 的快照缓存 ──

struct CachedSnapshot {
    rows: Arc<Vec<SnapshotRow>>,
    fetched_at: Instant,
    source: SnapshotSource,
}

static SNAPSHOT_CACHE: OnceLock<tokio::sync::RwLock<Option<CachedSnapshot>>> = OnceLock::new();

/// 快照默认缓存时长。全市场一轮是几十次请求，**必须缓存**，否则极易触发风控。
pub const DEFAULT_SNAPSHOT_TTL: Duration = Duration::from_secs(60);

/// 取全市场快照（带 TTL 缓存 + 请求去重）。
///
/// 缓存过期后会有且仅有一次真实拉取：写锁串行化避免了「多个调用方同时各拉几十页」，
/// 这是防止把数据源打爆的关键。
pub async fn market_snapshot_cached(
    ttl: Duration,
    preferred: Option<SnapshotSource>,
) -> Result<(Arc<Vec<SnapshotRow>>, SnapshotSource), EmError> {
    let cache = SNAPSHOT_CACHE.get_or_init(|| tokio::sync::RwLock::new(None));

    {
        let guard = cache.read().await;
        if let Some(cached) = guard.as_ref() {
            if cached.fetched_at.elapsed() < ttl {
                return Ok((cached.rows.clone(), cached.source));
            }
        }
    }

    let mut guard = cache.write().await;
    // 双重检查：可能已有其他任务在我们等锁期间刷新过
    if let Some(cached) = guard.as_ref() {
        if cached.fetched_at.elapsed() < ttl {
            return Ok((cached.rows.clone(), cached.source));
        }
    }

    let (rows, source) = fetch_snapshot_auto(preferred).await?;
    let rows = Arc::new(rows);
    *guard = Some(CachedSnapshot {
        rows: rows.clone(),
        fetched_at: Instant::now(),
        source,
    });
    Ok((rows, source))
}

/// 快照获取结果：数据 + 是否为「陈旧的兜底数据」+ 数据通道
pub struct SnapshotOutcome {
    pub rows: Arc<Vec<SnapshotRow>>,
    /// true 表示本次刷新失败（被限流或网络不通），返回的是上一次的旧数据
    pub stale: bool,
    pub source: SnapshotSource,
}

/// 带陈旧兜底的快照获取。
///
/// 数据源存在限流与网络波动，因此**刷新失败时不应让功能整体失效**：
/// 有旧缓存就返回旧数据并标记 `stale`，让 UI 能明确提示「数据陈旧」，
/// 这比直接报错、界面空白要好得多。
pub async fn market_snapshot_with_fallback(
    ttl: Duration,
    preferred: Option<SnapshotSource>,
) -> Result<SnapshotOutcome, EmError> {
    match market_snapshot_cached(ttl, preferred).await {
        Ok((rows, source)) => Ok(SnapshotOutcome {
            rows,
            stale: false,
            source,
        }),
        Err(error) => {
            let cache = SNAPSHOT_CACHE.get_or_init(|| tokio::sync::RwLock::new(None));
            let guard = cache.read().await;
            if let Some(cached) = guard.as_ref() {
                log::warn!("全市场快照刷新失败，回退到旧缓存（stale）: {error}");
                return Ok(SnapshotOutcome {
                    rows: cached.rows.clone(),
                    stale: true,
                    source: cached.source,
                });
            }
            Err(error)
        }
    }
}

// ── L1 筛选引擎 ──

/// 用户可配置的筛选条件（对应设置页）。
///
/// 设计原则：**所有区间都是 `Option`，`None` 表示不限制**，
/// 这样前端只需要传用户真正设置过的字段。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MarketFilter {
    /// 允许的板块。空 Vec 表示「全部允许」
    pub boards: Vec<Board>,
    /// 排除 ST / *ST
    pub exclude_st: bool,
    /// 排除退市 / 退市整理
    pub exclude_delisting: bool,
    /// 排除停牌（成交量为 0 或价格无效）
    pub exclude_suspended: bool,
    /// 排除一字板（买不进）
    pub exclude_limit_locked: bool,

    pub price_min: Option<f64>,
    pub price_max: Option<f64>,
    /// 总市值下限（亿元）
    pub market_cap_min_yi: Option<f64>,
    pub market_cap_max_yi: Option<f64>,
    /// 换手率 % 区间
    pub turnover_min: Option<f64>,
    pub turnover_max: Option<f64>,
    /// 量比下限
    pub volume_ratio_min: Option<f64>,
    /// 当日涨跌幅 % 区间
    pub change_pct_min: Option<f64>,
    pub change_pct_max: Option<f64>,
    /// 成交额下限（万元）
    pub amount_min_wan: Option<f64>,
    /// 60 日涨跌幅 % 区间。**趋势 / 反转类策略的主要依据**，仅东财通道提供。
    ///
    /// 为什么重要：此前预设全用当日字段，导致「稳健趋势」没有任何趋势判定、
    /// 「超跌反弹」只判当日下跌。60 日涨跌幅是日线快照里唯一的中期维度，
    /// 接上它这两条预设才名副其实。
    pub change_60d_min: Option<f64>,
    pub change_60d_max: Option<f64>,
    /// 市盈率（动态）区间。想只保留盈利股时把 min 设成 0.01 即可 ——
    /// 亏损股该字段为 0 或负值，会被自然排除。
    pub pe_min: Option<f64>,
    pub pe_max: Option<f64>,
    /// 市净率区间。A 股实证里 PB 是最稳健的估值因子（低 PB 长期胜率高于低 PE）。
    pub pb_min: Option<f64>,
    pub pb_max: Option<f64>,
    /// 振幅上限 %。**当日振幅，仅作波动率代理**，与 60 日波动率不是一回事。
    pub amplitude_max: Option<f64>,
}

impl Default for MarketFilter {
    /// 面向 A 股实盘的保守默认值：
    /// 排除 B 股与未识别板块、排除 ST/退市/停牌/一字板，但**不限制任何数值区间**
    /// —— 数值限制交给用户或预设方案，默认全放开，避免「一上来就筛没了」。
    fn default() -> Self {
        Self {
            boards: vec![
                Board::ShMain,
                Board::SzMain,
                Board::ChiNext,
                Board::Star,
                Board::Bse,
            ],
            exclude_st: true,
            exclude_delisting: true,
            exclude_suspended: true,
            exclude_limit_locked: true,
            price_min: None,
            price_max: None,
            market_cap_min_yi: None,
            market_cap_max_yi: None,
            turnover_min: None,
            turnover_max: None,
            volume_ratio_min: None,
            change_pct_min: None,
            change_pct_max: None,
            amount_min_wan: None,
            change_60d_min: None,
            change_60d_max: None,
            pe_min: None,
            pe_max: None,
            pb_min: None,
            pb_max: None,
            amplitude_max: None,
        }
    }
}

/// 筛选时可用的字段能力。
///
/// **为什么必须有这个**：不同数据源的字段覆盖不同 —— 新浪列表接口**不提供「量比」**
/// （所有行的 `volume_ratio` 都是 0）。如果照常比较 `row.volume_ratio < min`，
/// 那么内置预设里 4 套带量比条件的方案（稳健趋势 / 强势突破 / 短线活跃 / 超跌反弹）
/// 会**全部筛出 0 只** —— 用户看到的是「点了筛选什么都没有」，却完全不知道为什么。
///
/// 正确做法是：字段缺失时**跳过该条条件**，并把这个事实回传给 UI 明确提示。
#[derive(Debug, Clone, Copy)]
pub struct FilterCapabilities {
    /// 数据源是否提供「量比」
    pub volume_ratio: bool,
    /// 数据源是否提供「60 日涨跌幅」（趋势 / 反转类策略的依据）
    pub change_60d: bool,
}

impl Default for FilterCapabilities {
    /// 默认假设字段齐全 —— 只在「明知道来自哪个通道」的场合才用 `for_source` 收紧。
    fn default() -> Self {
        Self {
            volume_ratio: true,
            change_60d: true,
        }
    }
}

impl FilterCapabilities {
    /// 由快照通道推导出可用字段
    pub fn for_source(source: SnapshotSource) -> Self {
        Self {
            volume_ratio: source.has_volume_ratio(),
            change_60d: source.has_change_60d(),
        }
    }
}

impl MarketFilter {
    fn within(value: f64, min: Option<f64>, max: Option<f64>) -> bool {
        if let Some(min) = min {
            if value < min {
                return false;
            }
        }
        if let Some(max) = max {
            if value > max {
                return false;
            }
        }
        true
    }

    /// 修正「最小值大于最大值」的区间写法。
    ///
    /// 手输区间很容易反着填（如价格 1000 ~ 100），而反着填的结果**恒为空集** ——
    /// 界面上只表现为「命中 0 只」，用户完全看不出是自己填反了。
    /// 保存策略时统一交换一次，避免把一个永远筛不出东西的策略存下来。
    pub fn normalize_ranges(&mut self) {
        fn fix(min: &mut Option<f64>, max: &mut Option<f64>) {
            if let (Some(lo), Some(hi)) = (*min, *max) {
                if lo > hi {
                    *min = Some(hi);
                    *max = Some(lo);
                }
            }
        }
        fix(&mut self.price_min, &mut self.price_max);
        fix(&mut self.market_cap_min_yi, &mut self.market_cap_max_yi);
        fix(&mut self.turnover_min, &mut self.turnover_max);
        fix(&mut self.change_pct_min, &mut self.change_pct_max);
        fix(&mut self.change_60d_min, &mut self.change_60d_max);
        fix(&mut self.pe_min, &mut self.pe_max);
        fix(&mut self.pb_min, &mut self.pb_max);
    }

    /// 因数据源不支持而被忽略的条件名（用于给用户明确提示）
    pub fn skipped_conditions(&self, caps: FilterCapabilities) -> Vec<String> {
        let mut skipped = Vec::new();
        if !caps.volume_ratio && self.volume_ratio_min.is_some() {
            skipped.push("量比".to_owned());
        }
        // 60 日涨跌幅只有东财提供；新浪通道下必须明示，否则用户会以为策略失效了
        if !caps.change_60d && (self.change_60d_min.is_some() || self.change_60d_max.is_some()) {
            skipped.push("60 日涨跌幅".to_owned());
        }
        skipped
    }

    /// 单个标的是否通过筛选（默认假设所有字段可用）
    pub fn accepts(&self, row: &SnapshotRow) -> bool {
        self.accepts_with(row, FilterCapabilities::default())
    }

    /// 单个标的是否通过筛选，按 `caps` 决定哪些条件跳过
    pub fn accepts_with(&self, row: &SnapshotRow, caps: FilterCapabilities) -> bool {
        if !self.boards.is_empty() && !self.boards.contains(&row.board) {
            return false;
        }
        if self.exclude_st && row.is_st {
            return false;
        }
        if self.exclude_delisting && row.is_delisting {
            return false;
        }
        if self.exclude_suspended && row.suspected_suspended {
            return false;
        }
        if self.exclude_limit_locked && row.is_limit_locked {
            return false;
        }
        if !Self::within(row.price, self.price_min, self.price_max) {
            return false;
        }
        if !Self::within(
            row.market_cap_yi(),
            self.market_cap_min_yi,
            self.market_cap_max_yi,
        ) {
            return false;
        }
        if !Self::within(row.turnover_rate, self.turnover_min, self.turnover_max) {
            return false;
        }
        // 字段不可用时跳过，而不是拿 0 去比较
        if caps.volume_ratio {
            if let Some(min) = self.volume_ratio_min {
                if row.volume_ratio < min {
                    return false;
                }
            }
        }
        if !Self::within(row.change_pct, self.change_pct_min, self.change_pct_max) {
            return false;
        }
        if let Some(min_wan) = self.amount_min_wan {
            if row.amount / 10_000.0 < min_wan {
                return false;
            }
        }
        // 60 日涨跌幅：趋势 / 反转类策略的依据，仅东财通道有值
        if caps.change_60d
            && !Self::within(row.change_60d, self.change_60d_min, self.change_60d_max)
        {
            return false;
        }
        // 市盈率 / 市净率：两个通道都提供（新浪 per / pb，东财 f9 / f23）
        if !Self::within(row.pe, self.pe_min, self.pe_max) {
            return false;
        }
        if !Self::within(row.pb, self.pb_min, self.pb_max) {
            return false;
        }
        if let Some(max_amplitude) = self.amplitude_max {
            if row.amplitude_pct > max_amplitude {
                return false;
            }
        }
        true
    }

    /// 对全市场执行筛选（默认假设所有字段可用）。内存筛选在毫秒级，**零网络请求**。
    pub fn apply(&self, rows: &[SnapshotRow]) -> Vec<SnapshotRow> {
        self.apply_with(rows, FilterCapabilities::default())
    }

    /// 对全市场执行筛选，按 `caps` 跳过数据源不支持的字段
    pub fn apply_with(&self, rows: &[SnapshotRow], caps: FilterCapabilities) -> Vec<SnapshotRow> {
        rows.iter()
            .filter(|row| self.accepts_with(row, caps))
            .cloned()
            .collect()
    }
}

// ── 内置预设方案 ──

/// 一键切换的筛选预设。
///
/// 设计理由：让用户每次手调十几个字段是不现实的。
/// 内置方案分两类，**描述里必须说清楚是哪一类**，不要让用户误以为都能赚钱：
///
/// 1. **有公开实证支撑的因子类** —— 趋势确认、短期反转、低估值、低波动。
///    这类策略的方向和大致阈值来自公开的 A 股因子研究，注释里都标了出处与量级。
/// 2. **当日量价的行为类** —— 强势突破、短线活跃。它们只描述"今天盘面在发生什么"，
///    是选股池而不是收益预期，描述里明确写出来。
///
/// 刻意**没有**内置小市值策略：2024 年退市新规后，连续 20 日市值低于 5 亿元会直接退市，
/// 小盘组合的下行风险已经不是收益因子能覆盖的了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterPreset {
    /// 全部：只做基础排除，不限数值
    All,
    /// 稳健趋势：中大盘、中期不弱、温和放量上涨
    SteadyTrend,
    /// 强势突破：当日明显放量、涨幅靠前
    StrongBreakout,
    /// 短线活跃：中小市值、高换手
    ShortTermActive,
    /// 超跌反弹：中期跌幅足够大，当日仍弱
    OversoldRebound,
    /// 低估值价值：低市净率 + 正盈利
    LowValuation,
    /// 低波动稳健：低振幅 + 中大盘
    LowVolatility,
}

impl FilterPreset {
    /// 全部预设，顺序即 UI 展示顺序
    pub const ALL: [FilterPreset; 7] = [
        Self::All,
        Self::SteadyTrend,
        Self::StrongBreakout,
        Self::ShortTermActive,
        Self::OversoldRebound,
        Self::LowValuation,
        Self::LowVolatility,
    ];

    /// 稳定标识，用于前端持久化选中项
    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::SteadyTrend => "steady_trend",
            Self::StrongBreakout => "strong_breakout",
            Self::ShortTermActive => "short_term_active",
            Self::OversoldRebound => "oversold_rebound",
            Self::LowValuation => "low_valuation",
            Self::LowVolatility => "low_volatility",
        }
    }

    /// 该 id 是否属于内置预设。
    ///
    /// 用户自建策略的 id 一律带 `custom_` 前缀，这里再兜一道：
    /// 内置 id 绝不允许被自定义策略占用（否则「重置/切换」会指向错误的条件）。
    pub fn is_builtin_id(id: &str) -> bool {
        Self::ALL.iter().any(|preset| preset.id() == id)
    }

    /// 这条预设配套的交易规则。
    ///
    /// 映射依据是**预设想要的形态**与**规则的性格**匹配：
    /// - 反转类（超跌反弹）→ 均值回归：公开测算里胜率能过六成的那一类
    /// - 突破类（强势突破、短线活跃）→ 放量突破：靠量能确认把假信号率压下去
    /// - 趋势 / 价值 / 低波 → 趋势跟随：持有周期长，靠止损截断亏损、让利润跑
    ///
    /// 规则只在**个股分析**里生效（筛选器用的是单日快照，没有均线和 ATR）。
    pub fn rule(self) -> crate::quant::playbook::TradeRule {
        use crate::quant::playbook::TradeRule;
        match self {
            Self::OversoldRebound => TradeRule::MeanReversion,
            Self::StrongBreakout | Self::ShortTermActive => TradeRule::Breakout,
            Self::All | Self::SteadyTrend | Self::LowValuation | Self::LowVolatility => {
                TradeRule::TrendFollow
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::SteadyTrend => "稳健趋势",
            Self::StrongBreakout => "强势突破",
            Self::ShortTermActive => "短线活跃",
            Self::OversoldRebound => "超跌反弹",
            Self::LowValuation => "低估值价值",
            Self::LowVolatility => "低波动稳健",
        }
    }

    /// 一句话说明这条预设想选什么样的股票，直接展示给用户。
    /// **必须诚实**：是收益因子就说因子，是当日盘面就说盘面，避免用户误读。
    pub fn description(self) -> &'static str {
        match self {
            Self::All => "仅做基础排除（ST/退市/停牌/一字板/B股），不限数值区间",
            Self::SteadyTrend => "中大盘 · 近 60 日不弱 · 温和放量上涨 —— 趋势确认，不追高",
            Self::StrongBreakout => "当日明显放量且涨幅靠前 —— 资金驱动型选池，注意追高风险",
            Self::ShortTermActive => "中小市值 · 高换手 —— 活跃度筛选，用于短线选池，非收益预期",
            Self::OversoldRebound => "近 60 日跌超 15% 且今日仍弱 —— A 股短期反转效应，跌多的更易反弹",
            Self::LowValuation => "低市净率 + 正盈利 —— 价值因子，低 PB 是 A 股最稳健的估值因子",
            Self::LowVolatility => "低振幅 · 中大盘 —— 低波动因子，波动大的股票长期收益反而更差",
        }
    }

    /// 展开成具体的筛选条件。
    ///
    /// 所有预设都继承 [`MarketFilter::default`] 的基础排除规则，
    /// 只在此基础上叠加数值区间 —— 避免出现「某个预设忘了排除 ST」这类漏洞。
    ///
    /// 阈值取值说明见各分支注释：因子类方案的**方向**来自公开实证，
    /// 具体数值是在「不至于筛出空集」和「保持区分度」之间取的折中，
    /// 属于**未回测的手工阈值** —— 用户可在此基础上微调，也可以另存为自己的策略。
    pub fn build(self) -> MarketFilter {
        let mut filter = MarketFilter::default();
        match self {
            Self::All => {}
            Self::SteadyTrend => {
                // 中大盘：100 亿以下流动性差，3000 亿以上弹性不足
                filter.market_cap_min_yi = Some(100.0);
                filter.market_cap_max_yi = Some(3000.0);
                filter.turnover_min = Some(1.0);
                filter.turnover_max = Some(8.0);
                filter.volume_ratio_min = Some(1.0);
                // 涨跌幅温和，滤掉已经异动的
                filter.change_pct_min = Some(-2.0);
                filter.change_pct_max = Some(5.0);
                filter.amount_min_wan = Some(10_000.0);
                filter.price_min = Some(5.0);
                // 趋势确认的关键一条：近 60 日不能是下跌趋势。
                // 只要求「不弱」而不是「大涨」—— 追高在 A 股是负期望（见 OversoldRebound 注释）。
                filter.change_60d_min = Some(0.0);
            }
            Self::StrongBreakout => {
                filter.volume_ratio_min = Some(2.0);
                filter.change_pct_min = Some(3.0);
                // 上游是一字板已被排除，这里再卡一道 9%
                filter.change_pct_max = Some(9.0);
                filter.turnover_min = Some(3.0);
                filter.turnover_max = Some(20.0);
                filter.amount_min_wan = Some(20_000.0);
            }
            Self::ShortTermActive => {
                filter.market_cap_min_yi = Some(20.0);
                filter.market_cap_max_yi = Some(300.0);
                filter.turnover_min = Some(5.0);
                filter.turnover_max = Some(25.0);
                filter.volume_ratio_min = Some(1.5);
                filter.change_pct_min = Some(-3.0);
                filter.change_pct_max = Some(9.0);
                filter.amount_min_wan = Some(8_000.0);
            }
            Self::OversoldRebound => {
                // 核心是这条「中期跌够多」。此前只判当日跌幅，
                // 结果「跌 1% 的高位股」也会入选，而真正腰斩的票只要当天平盘就落选 —— 名不副实。
                //
                // 方向依据：A 股 2015—2025 全样本的价格动量因子 IC 为**负**
                // （3 个月回看、不跳过近期，IC≈-0.032，t≈-2.66），即短期显著**反转**；
                // 3 个月回看是效应最强的窗口，正好对应这里的 60 日。
                filter.change_60d_max = Some(-15.0);
                // 今日仍弱：不在放量拉升时分批接，避免追在半山腰
                filter.change_pct_min = Some(-9.0);
                filter.change_pct_max = Some(0.0);
                filter.turnover_min = Some(2.0);
                // 有资金介入迹象（放量止跌比缩量阴跌更值得看）
                filter.volume_ratio_min = Some(1.0);
                filter.amount_min_wan = Some(5_000.0);
                // 1 元附近的仙股不碰
                filter.price_min = Some(3.0);
            }
            Self::LowValuation => {
                // 价值因子的核心：A 股 2004—2024 二十年，低 PB 组合跑赢胜率约 55%，
                // 且 2020—2024 更突出（破净股胜率均值约 70%）；多项研究认为
                // **PB 比 PE 更稳健**，所以这里以 PB 为主约束、PE 只用来排除亏损股。
                filter.pb_min = Some(0.01);
                filter.pb_max = Some(2.0);
                // pe 下限设成正数 = 只要盈利股（亏损股该字段为 0 或负）
                filter.pe_min = Some(0.01);
                filter.pe_max = Some(30.0);
                // 50 亿以下避开退市新规的市值红线区域，也不至于买不到量
                filter.market_cap_min_yi = Some(50.0);
                filter.amount_min_wan = Some(5_000.0);
                filter.price_min = Some(3.0);
            }
            Self::LowVolatility => {
                // 低波动因子的方向在 A 股是稳的（多份券商测算显示低波组合的
                // 风险调整后收益明显优于高波），但**这里只能用当日振幅近似**：
                // 快照没有 60 日波动率，日线数据要逐只拉 K 线，不适合放进全市场粗筛。
                // 所以这条是「低波动」的代理，不是严格的波动率因子。
                filter.amplitude_max = Some(4.0);
                filter.market_cap_min_yi = Some(100.0);
                filter.turnover_min = Some(0.5);
                filter.turnover_max = Some(5.0);
                filter.amount_min_wan = Some(5_000.0);
                filter.price_min = Some(5.0);
            }
        }
        filter
    }
}

/// 预设的描述信息，供前端一次性拉取渲染切换条。
///
/// 内置预设与用户自建策略共用这个结构：`builtin` 决定前端是否给出「改名 / 删除」入口。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresetInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    pub filter: MarketFilter,
    /// 配套的交易规则 id（`trend_follow` / `mean_reversion` / `breakout`）。
    ///
    /// 策略只负责**粗筛**（单日快照能表达的字段），精确买点/止损/止盈要靠日 K 算，
    /// 这个字段就是两者的纽带：打开个股分析时用它决定用哪套规则。
    /// 反序列化用户旧数据时缺省为空串，前端会安全回落到趋势跟随。
    #[serde(default)]
    pub rule: String,
    /// 是否内置。内置策略随版本更新，**不允许改名或删除**，只能「另存为」出自己的副本；
    /// 从 settings 反序列化用户数据时该项缺省为 false。
    #[serde(default)]
    pub builtin: bool,
}

/// 列出全部内置预设（含展开后的筛选条件，方便前端直接预览）
pub fn preset_infos() -> Vec<PresetInfo> {
    FilterPreset::ALL
        .iter()
        .map(|preset| PresetInfo {
            id: preset.id().to_owned(),
            label: preset.label().to_owned(),
            description: preset.description().to_owned(),
            filter: preset.build(),
            rule: preset.rule().id().to_owned(),
            builtin: true,
        })
        .collect()
}

/// 按 id 取预设，未知 id 回退到 [`FilterPreset::All`]
pub fn preset_by_id(id: &str) -> FilterPreset {
    FilterPreset::ALL
        .iter()
        .copied()
        .find(|preset| preset.id() == id)
        .unwrap_or(FilterPreset::All)
}

// 东财 field 含义速查（f12/f14/... 的可读对照）：
// f2=最新价 f3=涨跌幅 f4=涨跌额 f5=成交量(手) f6=成交额(元) f7=振幅
// f8=换手率 f9=市盈率(动) f10=量比 f11=5分钟涨跌 f12=代码 f14=名称
// f15=最高 f16=最低 f17=今开 f18=昨收 f20=总市值 f21=流通市值
// f22=涨速 f23=市净率 f24=60日涨跌幅 f25=年初至今涨跌幅

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_classification_covers_all_segments() {
        assert_eq!(Board::from_code("600519"), Board::ShMain);
        assert_eq!(Board::from_code("601398"), Board::ShMain);
        assert_eq!(Board::from_code("603259"), Board::ShMain);
        assert_eq!(Board::from_code("605499"), Board::ShMain);

        assert_eq!(Board::from_code("000001"), Board::SzMain);
        assert_eq!(Board::from_code("002594"), Board::SzMain);
        assert_eq!(Board::from_code("003816"), Board::SzMain);

        assert_eq!(Board::from_code("300750"), Board::ChiNext);
        assert_eq!(Board::from_code("301236"), Board::ChiNext);

        assert_eq!(Board::from_code("688981"), Board::Star);
        assert_eq!(Board::from_code("689009"), Board::Star);

        assert_eq!(Board::from_code("920992"), Board::Bse);
        assert_eq!(Board::from_code("430047"), Board::Bse);
        assert_eq!(Board::from_code("832000"), Board::Bse);
        assert_eq!(Board::from_code("871981"), Board::Bse);

        assert_eq!(Board::from_code("200011"), Board::BShare);
        assert_eq!(Board::from_code("900901"), Board::BShare);
    }

    #[test]
    fn b_share_and_unknown_are_not_tradable() {
        assert!(!Board::BShare.is_tradable_a());
        assert!(!Board::Other.is_tradable_a());
        assert!(Board::Star.is_tradable_a());
        assert!(Board::Bse.is_tradable_a());
    }

    #[test]
    fn parses_row_and_derives_flags() {
        let raw = serde_json::json!({
            "f12": "600519", "f14": "*ST某某", "f2": 1500.0, "f3": 3.2,
            "f4": 46.0, "f5": 12345.0, "f6": 1.8e9, "f7": 2.5, "f8": 0.9,
            "f9": 22.5, "f10": 1.4, "f11": 0.1, "f15": 1510.0, "f16": 1470.0,
            "f17": 1480.0, "f18": 1454.0, "f20": 1.9e12, "f21": 1.9e12,
            "f22": 0.05, "f23": 8.1, "f24": 12.0, "f25": 6.0
        });
        let row = parse_row(&raw).expect("应能解析");
        assert_eq!(row.code, "600519");
        assert_eq!(row.board, Board::ShMain);
        assert!(row.is_st, "名称含 ST 应被识别");
        assert!(!row.is_delisting);
        assert!(!row.suspected_suspended);
        assert!(row.is_basically_tradable());
        assert!((row.market_cap_yi() - 19000.0).abs() < 1.0);
    }

    #[test]
    fn dash_string_is_treated_as_zero() {
        let raw = serde_json::json!({ "f12": "600000", "f14": "停牌股", "f2": "-", "f5": 0.0 });
        let row = parse_row(&raw).expect("应能解析");
        assert_eq!(row.price, 0.0);
        assert!(row.suspected_suspended);
        assert!(!row.is_basically_tradable());
    }

    #[test]
    fn rejects_malformed_code_or_empty_name() {
        assert!(parse_row(&serde_json::json!({ "f12": "abc", "f14": "X" })).is_none());
        assert!(parse_row(&serde_json::json!({ "f12": "600519", "f14": "" })).is_none());
        assert!(parse_row(&serde_json::json!({ "f12": "60051", "f14": "X" })).is_none());
    }

    #[test]
    fn detects_limit_locked_and_delisting() {
        // 主板 10% 封板 + 振幅 0.2% → 一字板
        let main = parse_row(&serde_json::json!({
            "f12": "600001", "f14": "某某股份", "f2": 11.0, "f3": 10.0, "f7": 0.2, "f5": 100.0
        }))
        .expect("应能解析");
        assert!(main.is_limit_locked);

        // 创业板涨跌幅限制是 ±20%，10% 不算一字板（旧版用统一 9.8% 阈值会误判）
        let raw = serde_json::json!({
            "f12": "300001", "f14": "退市某某", "f2": 1.0, "f3": 10.0, "f7": 0.2, "f5": 100.0
        });
        let row = parse_row(&raw).expect("应能解析");
        assert!(row.is_delisting);
        assert_eq!(row.board, Board::ChiNext);
        assert!(!row.is_limit_locked, "创业板 10% 未到 20% 限制，不应判为一字板");
    }

    #[test]
    fn limit_threshold_follows_board_and_st_rules() {
        // ST 主板只有 ±5%，4.98% 就该算一字板
        assert!(is_limit_locked(4.98, 0.1, Board::SzMain, true));
        assert!(!is_limit_locked(4.98, 0.1, Board::SzMain, false));
        // 普通主板 ±10%
        assert!(is_limit_locked(10.0, 0.0, Board::ShMain, false));
        // 创业板/科创板 ±20%
        assert!(!is_limit_locked(10.0, 0.0, Board::ChiNext, false));
        assert!(is_limit_locked(20.0, 0.0, Board::Star, false));
        // 北交所 ±30%
        assert!(is_limit_locked(30.0, 0.5, Board::Bse, false));
        assert!(!is_limit_locked(20.0, 0.5, Board::Bse, false));
        // 振幅大说明不是一字板
        assert!(!is_limit_locked(10.0, 5.0, Board::ShMain, false));
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    /// 构造一只「正常可交易」的样本股，便于单独改变某个字段做隔离测试
    fn row(code: &str, name: &str) -> SnapshotRow {
        let raw = serde_json::json!({
            "f12": code, "f14": name,
            "f2": 20.0,        // 价格
            "f3": 2.0,         // 涨跌幅
            "f5": 100000.0,    // 成交量（手）
            "f6": 2.0e8,       // 成交额（元）= 20000 万
            "f7": 3.0,         // 振幅
            "f8": 5.0,         // 换手率
            "f10": 1.5,        // 量比
            "f20": 5.0e9       // 总市值（元）= 50 亿
        });
        parse_row(&raw).expect("样本应能解析")
    }

    #[test]
    fn default_filter_excludes_b_share_and_st() {
        let filter = MarketFilter::default();
        assert!(filter.accepts(&row("600519", "贵州茅台")));
        assert!(!filter.accepts(&row("200011", "某B股")), "B股默认应被排除");
        assert!(!filter.accepts(&row("600001", "ST某某")), "ST 默认应被排除");
        assert!(
            !filter.accepts(&row("600002", "退市某某")),
            "退市默认应被排除"
        );
    }

    #[test]
    fn board_whitelist_is_respected() {
        let mut filter = MarketFilter::default();
        filter.boards = vec![Board::ChiNext];
        assert!(filter.accepts(&row("300750", "宁德时代")));
        assert!(!filter.accepts(&row("600519", "贵州茅台")), "非白名单板块应被排除");

        // 空 Vec 表示放开全部板块
        filter.boards = vec![];
        assert!(filter.accepts(&row("600519", "贵州茅台")));
    }

    #[test]
    fn numeric_ranges_are_inclusive_and_optional() {
        let mut filter = MarketFilter::default();
        // 样本价格固定为 20.0
        filter.price_min = Some(15.0);
        filter.price_max = Some(25.0);
        assert!(filter.accepts(&row("600519", "区间内")));

        // 闭区间：值恰好等于边界时应通过
        filter.price_min = Some(20.0);
        filter.price_max = Some(20.0);
        assert!(
            filter.accepts(&row("600519", "恰好等于上下界")),
            "区间应为闭区间"
        );
        filter.price_min = Some(20.01);
        assert!(!filter.accepts(&row("600519", "低于下界应排除")));
        filter.price_min = None;
        filter.price_max = Some(19.99);
        assert!(!filter.accepts(&row("600519", "高于上界应排除")));

        // None 表示不限制
        filter.price_min = None;
        filter.price_max = None;
        assert!(filter.accepts(&row("600519", "不限制价格")));
    }

    #[test]
    fn market_cap_filter_uses_yi_yuan_unit() {
        let mut filter = MarketFilter::default();
        // 样本总市值 = 50 亿
        filter.market_cap_min_yi = Some(40.0);
        filter.market_cap_max_yi = Some(60.0);
        assert!(filter.accepts(&row("600519", "50亿在区间内")));

        filter.market_cap_min_yi = Some(60.0);
        assert!(!filter.accepts(&row("600519", "50亿低于下限")));
    }

    #[test]
    fn volume_ratio_and_amount_thresholds_work() {
        let mut filter = MarketFilter::default();
        filter.volume_ratio_min = Some(1.5);
        assert!(filter.accepts(&row("600519", "量比1.5达标")));
        filter.volume_ratio_min = Some(1.51);
        assert!(!filter.accepts(&row("600519", "量比不足")));

        filter.volume_ratio_min = None;
        // 样本成交额 = 2e8 元 = 20000 万
        filter.amount_min_wan = Some(15000.0);
        assert!(filter.accepts(&row("600519", "成交额达标")));
        filter.amount_min_wan = Some(30000.0);
        assert!(!filter.accepts(&row("600519", "成交额不足")));
    }

    #[test]
    fn apply_filters_the_whole_slice() {
        let rows = vec![
            row("600519", "贵州茅台"),
            row("300750", "宁德时代"),
            row("600001", "ST某某"),
            row("200011", "某B股"),
        ];
        let filter = MarketFilter::default();
        let kept = filter.apply(&rows);
        let codes: Vec<&str> = kept.iter().map(|r| r.code.as_str()).collect();
        assert_eq!(codes.len(), 2, "应只留下两只正常股票，实际: {codes:?}");
        assert!(codes.contains(&"600519"));
        assert!(codes.contains(&"300750"));
    }

    #[test]
    fn suspend_and_limit_locked_can_be_kept_on_demand() {
        let mut filter = MarketFilter::default();

        let suspended = parse_row(&serde_json::json!({
            "f12": "600003", "f14": "停牌股", "f2": 10.0, "f5": 0.0
        }))
        .expect("应能解析");
        assert!(!filter.accepts(&suspended), "停牌默认排除");
        filter.exclude_suspended = false;
        assert!(filter.accepts(&suspended), "关掉开关后应保留");

        let locked = parse_row(&serde_json::json!({
            "f12": "600004", "f14": "涨停板", "f2": 11.0, "f3": 9.99, "f7": 0.1, "f5": 1000.0
        }))
        .expect("应能解析");
        filter.exclude_suspended = true;
        assert!(!filter.accepts(&locked), "一字板默认排除");
        filter.exclude_limit_locked = false;
        assert!(filter.accepts(&locked), "关掉开关后应保留");
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    /// 构造一只可调字段的样本股，用于验证预设的区分度
    fn sample(
        code: &str,
        name: &str,
        change_pct: f64,
        volume_ratio: f64,
        turnover: f64,
        amount: f64,
    ) -> SnapshotRow {
        let raw = serde_json::json!({
            "f12": code, "f14": name,
            "f2": 20.0, "f3": change_pct, "f5": 100000.0, "f6": amount,
            "f7": 3.0, "f8": turnover, "f10": volume_ratio, "f20": 5.0e9
        });
        parse_row(&raw).expect("应能解析")
    }

    #[test]
    fn every_preset_keeps_base_exclusions() {
        for preset in FilterPreset::ALL {
            let filter = preset.build();
            let id = preset.id();
            assert!(filter.exclude_st, "预设 {id} 不应关闭 ST 排除");
            assert!(filter.exclude_delisting, "预设 {id} 不应关闭退市排除");
            assert!(filter.exclude_suspended, "预设 {id} 不应关闭停牌排除");
            assert!(filter.exclude_limit_locked, "预设 {id} 不应关闭一字板排除");
            assert!(
                !filter.boards.is_empty(),
                "预设 {id} 不应放宽板块白名单（会混入 B 股）"
            );
        }
    }

    #[test]
    fn preset_ids_are_unique_and_resolvable() {
        let mut ids: Vec<&str> = FilterPreset::ALL.iter().map(|preset| preset.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "预设 id 必须唯一");

        for preset in FilterPreset::ALL {
            assert_eq!(preset_by_id(preset.id()), preset);
        }
        // 未知 id 应安全回退，而不是 panic
        assert_eq!(preset_by_id("不存在的预设"), FilterPreset::All);
    }

    #[test]
    fn preset_infos_are_complete_and_differentiated() {
        let infos = preset_infos();
        assert_eq!(infos.len(), FilterPreset::ALL.len());

        let all = infos.iter().find(|info| info.id == "all").expect("应含 all");
        assert!(all.filter.market_cap_min_yi.is_none(), "全部预设不应限制市值");
        assert!(all.filter.change_pct_min.is_none(), "全部预设不应限制涨幅");

        // 除「全部」外的预设必须有区分度条件，否则选出来和全部一样
        for info in infos.iter().filter(|info| info.id != "all") {
            let f = &info.filter;
            let has_narrowing = f.volume_ratio_min.is_some()
                || f.turnover_min.is_some()
                || f.change_60d_min.is_some()
                || f.change_60d_max.is_some()
                || f.pe_min.is_some()
                || f.pe_max.is_some()
                || f.pb_min.is_some()
                || f.pb_max.is_some()
                || f.amplitude_max.is_some();
            assert!(has_narrowing, "预设 {} 缺少区分度条件", info.id);
            assert!(!info.label.is_empty(), "预设 {} 缺少中文名", info.id);
            assert!(!info.description.is_empty(), "预设 {} 缺少说明", info.id);
        }
    }

    /// 回归守卫：**趋势 / 反转类预设必须真的用上中期维度**。
    ///
    /// 曾经的实现里「稳健趋势」全都用当日字段，「超跌反弹」只判当日跌幅 ——
    /// 名字承诺了趋势和超跌，条件里却一个字都没提。任何把 `change_60d` 摘掉的
    /// 改动都应该在这里被拦住。
    #[test]
    fn trend_and_reversal_presets_use_mid_term_return() {
        let steady = FilterPreset::SteadyTrend.build();
        assert!(
            steady.change_60d_min.is_some(),
            "「稳健趋势」必须带 60 日涨跌幅下限，否则谈不上趋势"
        );

        let oversold = FilterPreset::OversoldRebound.build();
        assert!(
            oversold.change_60d_max.is_some(),
            "「超跌反弹」必须带 60 日涨跌幅上限，否则「超跌」无从体现"
        );
    }

    /// 每条内置策略都必须带上一条可解析的交易规则 —— 否则「打开分析」拿不到买卖点。
    ///
    /// 同时锁住「形态 ↔ 规则」的匹配关系：超跌反弹必须走均值回归（公开测算里
    /// 胜率能过六成的那一类），突破类必须走放量突破。这层映射被改错的话，
    /// 用户会看到一条名叫「超跌反弹」却在用趋势规则算买点的策略。
    #[test]
    fn every_preset_carries_a_valid_trade_rule() {
        use crate::quant::playbook::TradeRule;

        for preset in FilterPreset::ALL {
            let rule = preset.rule();
            assert_eq!(TradeRule::from_id(rule.id()), rule, "规则 id 必须能往返解析");
            assert!(!rule.label().is_empty(), "规则 {} 缺中文名", rule.id());
            assert!(
                !rule.profile().is_empty(),
                "规则 {} 必须说明自己的胜率 / 盈亏比性格",
                rule.id()
            );
        }

        assert_eq!(FilterPreset::OversoldRebound.rule(), TradeRule::MeanReversion);
        assert_eq!(FilterPreset::StrongBreakout.rule(), TradeRule::Breakout);
        assert_eq!(FilterPreset::ShortTermActive.rule(), TradeRule::Breakout);
        assert_eq!(FilterPreset::SteadyTrend.rule(), TradeRule::TrendFollow);
        assert_eq!(FilterPreset::LowValuation.rule(), TradeRule::TrendFollow);
    }

    /// 规则必须随预设一起下发给前端，否则前端不知道按哪套规则算操作计划
    #[test]
    fn preset_infos_expose_the_rule_id() {
        let infos = preset_infos();
        assert!(!infos.is_empty());
        for info in &infos {
            assert!(!info.rule.is_empty(), "预设 {} 缺少规则 id", info.id);
            assert!(info.builtin, "内置预设 {} 未标记 builtin", info.id);
        }
    }

    /// 建一只可指定估值 / 60 日涨跌幅 / 振幅的样本股
    fn sample_full(        change_pct: f64,
        volume_ratio: f64,
        turnover: f64,
        pe: f64,
        pb: f64,
        change_60d: f64,
        amplitude: f64,
        market_cap: f64,
    ) -> SnapshotRow {
        let raw = serde_json::json!({
            "f12": "600519", "f14": "样本股",
            "f2": 20.0, "f3": change_pct, "f5": 100000.0, "f6": 1.0e8,
            "f7": amplitude, "f8": turnover, "f9": pe, "f10": volume_ratio,
            "f20": market_cap, "f23": pb, "f24": change_60d
        });
        parse_row(&raw).expect("应能解析")
    }

    /// 超跌反弹的核心是「中期跌够了」，而不是「今天跌了」——
    /// 旧实现只判当日涨跌幅，会把高位回调 1% 的股票也放进来。
    #[test]
    fn oversold_rebound_requires_real_drawdown_not_just_a_red_day() {
        let filter = FilterPreset::OversoldRebound.build();

        // 60 日跌 30%、今日 -3%、放量 → 符合
        assert!(filter.accepts(&sample_full(-3.0, 1.5, 5.0, 20.0, 2.0, -30.0, 5.0, 1.0e10)));
        // 60 日基本没跌，只是今天绿了 → 不算超跌，必须被拒
        assert!(
            !filter.accepts(&sample_full(-3.0, 1.5, 5.0, 20.0, 2.0, 0.0, 5.0, 1.0e10)),
            "没跌够就不该叫超跌反弹"
        );
        // 跌够了但今天在拉升 → 不追，拒掉
        assert!(
            !filter.accepts(&sample_full(2.0, 1.5, 5.0, 20.0, 2.0, -30.0, 5.0, 1.0e10)),
            "今日走强时不接"
        );
    }

    /// 价值因子以 PB 为主约束，同时要求正盈利（亏损股 pe 为 0）
    #[test]
    fn low_valuation_needs_low_pb_and_positive_earnings() {
        let filter = FilterPreset::LowValuation.build();

        // PE 12 / PB 1.2 / 市值 100 亿 → 符合
        assert!(filter.accepts(&sample_full(1.0, 1.2, 3.0, 12.0, 1.2, 5.0, 3.0, 1.0e10)));
        // PB 5.0 已经不算低估 → 拒
        assert!(
            !filter.accepts(&sample_full(1.0, 1.2, 3.0, 12.0, 5.0, 5.0, 3.0, 1.0e10)),
            "高市净率不应被当成低估值"
        );
        // 亏损股（pe = 0）即使 PB 很低也要拒
        assert!(
            !filter.accepts(&sample_full(1.0, 1.2, 3.0, 0.0, 1.0, 5.0, 3.0, 1.0e10)),
            "亏损股不该进低估值策略"
        );
    }

    /// 低波动用的是当日振幅代理，振幅过大直接排除
    #[test]
    fn low_volatility_rejects_wide_amplitude() {
        let filter = FilterPreset::LowVolatility.build();

        assert!(filter.accepts(&sample_full(0.5, 1.1, 3.0, 15.0, 1.5, 3.0, 2.0, 2.0e10)));
        assert!(
            !filter.accepts(&sample_full(0.5, 1.1, 3.0, 15.0, 1.5, 3.0, 9.0, 2.0e10)),
            "振幅 9% 不是低波动"
        );
    }

    /// 新浪通道不提供 60 日涨跌幅：依赖它的条件必须被跳过并明确告知，
    /// 否则会拿 0 去比较，把结果筛成空集。
    #[test]
    fn mid_term_condition_is_skipped_on_sources_without_the_field() {
        let filter = FilterPreset::OversoldRebound.build();

        let sina = FilterCapabilities::for_source(SnapshotSource::Sina);
        assert!(!sina.change_60d);
        assert!(
            filter
                .skipped_conditions(sina)
                .iter()
                .any(|name| name == "60 日涨跌幅"),
            "新浪通道下必须提示 60 日涨跌幅被忽略"
        );

        let eastmoney = FilterCapabilities::for_source(SnapshotSource::Eastmoney);
        assert!(eastmoney.change_60d);
        assert!(
            !filter
                .skipped_conditions(eastmoney)
                .iter()
                .any(|name| name == "60 日涨跌幅"),
            "东财通道下不应提示该条件被忽略"
        );
    }

    #[test]
    fn strong_breakout_rejects_weak_and_accepts_strong() {
        let filter = FilterPreset::StrongBreakout.build();
        // 温和上涨 + 量比不足 → 不符合「强势突破」
        assert!(!filter.accepts(&sample("600519", "温和股", 2.0, 1.5, 8.0, 3.0e8)));
        // 涨幅 5% + 量比 2.5 + 换手 8% + 成交额 3 亿 → 符合
        assert!(filter.accepts(&sample("600519", "突破股", 5.0, 2.5, 8.0, 3.0e8)));
    }

    #[test]
    fn short_term_active_rejects_large_caps() {
        let filter = FilterPreset::ShortTermActive.build();
        // 市值 50 亿（样本即 5e9）→ 符合
        assert!(filter.accepts(&sample("300001", "小盘股", 3.0, 2.0, 10.0, 1.0e8)));

        // 市值 5000 亿 → 超上限
        let big = parse_row(&serde_json::json!({
            "f12": "600519", "f14": "大盘股", "f2": 20.0, "f3": 3.0,
            "f5": 100000.0, "f6": 1.0e9, "f7": 3.0, "f8": 10.0, "f10": 2.0, "f20": 5.0e11
        }))
        .expect("应能解析");
        assert!(!filter.accepts(&big), "5000 亿市值应被短线预设排除");
    }

    /// 区间填反了（min > max）结果恒为空集，保存策略前必须纠正 ——
    /// 否则用户会存下一个「永远筛不出任何股票」的策略却毫无察觉。
    #[test]
    fn normalize_ranges_swaps_reversed_bounds() {
        let mut reversed = MarketFilter {
            price_min: Some(100.0),
            price_max: Some(10.0),
            turnover_min: Some(8.0),
            turnover_max: Some(1.0),
            ..MarketFilter::default()
        };
        reversed.normalize_ranges();
        assert_eq!(reversed.price_min, Some(10.0));
        assert_eq!(reversed.price_max, Some(100.0));
        assert_eq!(reversed.turnover_min, Some(1.0));
        assert_eq!(reversed.turnover_max, Some(8.0));

        // 只填了一边：不动，保持「不限另一端」的语义
        let mut single = MarketFilter {
            price_min: Some(10.0),
            ..MarketFilter::default()
        };
        single.normalize_ranges();
        assert_eq!(single.price_min, Some(10.0));
        assert_eq!(single.price_max, None);

        // 顺序本来就对：原样保留（含负数区间，不能因为「负数比正数小」就乱换）
        let mut ordered = MarketFilter {
            change_pct_min: Some(-9.0),
            change_pct_max: Some(0.0),
            ..MarketFilter::default()
        };
        ordered.normalize_ranges();
        assert_eq!(ordered.change_pct_min, Some(-9.0));
        assert_eq!(ordered.change_pct_max, Some(0.0));
    }

    /// 内置预设的 id 不允许被自建策略占用：`is_builtin_id` 是保存/删除时的守门人。
    #[test]
    fn is_builtin_id_recognizes_all_and_only_builtins() {
        for preset in FilterPreset::ALL {
            assert!(FilterPreset::is_builtin_id(preset.id()));
        }
        assert!(!FilterPreset::is_builtin_id("custom_1234567890"));
        assert!(!FilterPreset::is_builtin_id(""));
        assert!(!FilterPreset::is_builtin_id("custom"));
    }
}
