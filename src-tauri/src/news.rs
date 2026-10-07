//! 全市场资讯：独立轮询、事件白名单、自选标记与持久化去重。

use crate::db::Database;
use chrono::{NaiveDateTime, TimeZone, Timelike, Utc};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

const FAST_URL: &str = "https://np-weblist.eastmoney.com/comm/web/getFastNewsList";
const ANNOUNCEMENT_URL: &str = "https://np-anotice-stock.eastmoney.com/api/security/ann";
const POLL_SECONDS: u64 = 60;
const QUIET_POLL_SECONDS: u64 = 300;

static NEWS_AI_RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct AiRunGuard(String);
impl AiRunGuard {
    fn claim(id: String) -> Result<Self, String> {
        let mut running = NEWS_AI_RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
            .lock().unwrap_or_else(|error| error.into_inner());
        if !running.insert(id.clone()) { return Err("这条资讯正在 AI 解读，请稍后查看".into()); }
        Ok(Self(id))
    }
}
impl Drop for AiRunGuard {
    fn drop(&mut self) {
        NEWS_AI_RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
            .lock().unwrap_or_else(|error| error.into_inner()).remove(&self.0);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RawNews {
    source: &'static str,
    source_label: &'static str,
    source_id: String,
    title: String,
    body: String,
    codes: Vec<String>,
    published_at: Option<String>,
    published_date: Option<String>,
    publication_precision: &'static str,
    published_at_source: &'static str,
    url: Option<String>,
    received_at: String,
    source_index_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Route {
    kind: &'static str,
    popup: bool,
}

struct PreparedNews {
    item: RawNews,
    matches: Vec<(String, String)>,
    route: Route,
    key: String,
    fallback_body: String,
    related_mainlines: Vec<Value>,
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(12))
            .user_agent("Mozilla/5.0 Bull-Arrives/1.5")
            .build()
            .expect("构建资讯客户端失败")
    })
}

fn string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn normalize_code(raw: &str) -> Option<String> {
    let raw = raw.trim().to_ascii_lowercase();
    let tail = raw
        .strip_prefix("sh")
        .or_else(|| raw.strip_prefix("sz"))
        .or_else(|| raw.strip_prefix("bj"))
        .or_else(|| raw.rsplit_once('.').map(|(_, code)| code))
        .unwrap_or(&raw);
    (tail.len() == 6 && tail.bytes().all(|byte| byte.is_ascii_digit())).then(|| tail.to_owned())
}

// 两个已实取的接口均返回北京时间；公告 display_time 的毫秒以冒号分隔。
fn publication_time(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let normalized = if raw.len() > 19 && raw.as_bytes().get(19) == Some(&b':') {
        format!("{}.{}", &raw[..19], &raw[20..])
    } else { raw.to_owned() };
    let parsed = NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S%.f").ok()?;
    chrono::FixedOffset::east_opt(8 * 3600)?.from_local_datetime(&parsed).single().map(|t| t.to_rfc3339())
}

fn announcement_url(code: &str, art_code: &str) -> Option<String> {
    // URL 结构已用实际公告 AN202610011830059731 验证；不把 sort_date 当发布时间。
    (normalize_code(code).is_some() && art_code.starts_with("AN")
        && art_code.len() > 2 && art_code[2..].bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("https://data.eastmoney.com/notices/detail/{code}/{art_code}.html"))
}

fn parse_fast(value: &Value) -> Result<Vec<RawNews>, String> {
    if value.get("code").and_then(Value::as_str) != Some("1") {
        return Err(format!(
            "快讯接口返回失败：{}",
            string(value.get("message"))
        ));
    }
    let list = value
        .pointer("/data/fastNewsList")
        .and_then(Value::as_array)
        .ok_or_else(|| "快讯响应缺少 data.fastNewsList".to_string())?;
    Ok(list
        .iter()
        .filter_map(|item| {
            let title = string(item.get("title"));
            let body = string(item.get("summary"));
            if title.is_empty() && body.is_empty() {
                return None;
            }
            let codes = item
                .get("stockList")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(normalize_code)
                .collect();
            let published_at = publication_time(&string(item.get("showTime")));
            Some(RawNews {
                source: "eastmoney_flash",
                source_label: "东方财富财经 / 上市公司快讯",
                source_id: string(item.get("code")),
                title,
                body,
                codes,
                published_date: published_at.as_ref().map(|t| t[..10].to_owned()),
                publication_precision: if published_at.is_some() { "second" } else { "unknown" },
                published_at_source: if published_at.is_some() { "showTime" } else { "unknown" },
                published_at,
                // 快讯列表没有逐条 URL 字段，不能按 ID 猜造文章链接。
                url: None,
                received_at: Utc::now().to_rfc3339(),
                source_index_only: false,
            })
        })
        .collect())
}

fn parse_announcements(value: &Value) -> Result<Vec<RawNews>, String> {
    if value.get("success").and_then(Value::as_i64) != Some(1) {
        return Err(format!("公告接口返回失败：{}", string(value.get("error"))));
    }
    let list = value
        .pointer("/data/list")
        .and_then(Value::as_array)
        .ok_or_else(|| "公告响应缺少 data.list".to_string())?;
    Ok(list
        .iter()
        .filter_map(|item| {
            let title = string(item.get("title_ch"));
            if title.is_empty() {
                return None;
            }
            let codes: Vec<String> = item
                .get("codes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|code| code.get("stock_code").and_then(Value::as_str))
                .filter_map(normalize_code)
                .collect();
            let source_id = string(item.get("art_code"));
            let display_time = publication_time(&string(item.get("display_time")));
            let (published_at, published_at_source) = if display_time.is_some() {
                (display_time, "display_time")
            } else { (publication_time(&string(item.get("eiTime"))), "eiTime") };
            let notice_date = string(item.get("notice_date"));
            let published_date = published_at.as_ref().map(|t| t[..10].to_owned()).or_else(|| {
                chrono::NaiveDate::parse_from_str(notice_date.get(..10).unwrap_or(""), "%Y-%m-%d")
                    .ok().map(|d| d.to_string())
            });
            let publication_precision = if published_at.is_some() { "millisecond" }
                else if published_date.is_some() { "date" } else { "unknown" };
            let url = codes.first().and_then(|code| announcement_url(code, &source_id));
            Some(RawNews {
                source: "eastmoney_announcement",
                source_label: "东方财富公司公告",
                source_id,
                body: String::new(),
                title,
                codes,
                published_at,
                published_date,
                publication_precision,
                published_at_source: if publication_precision == "date" { "notice_date" }
                    else if publication_precision == "unknown" { "unknown" } else { published_at_source },
                url,
                received_at: Utc::now().to_rfc3339(),
                source_index_only: true,
            })
        })
        .collect())
}

async fn fetch_fast() -> Result<Vec<RawNews>, String> {
    let trace = Utc::now().timestamp_millis().to_string();
    let mut items = Vec::new();
    for column in ["102", "103"] {
        let value: Value = client()
        .get(FAST_URL)
        .query(&[
            ("client", "web"),
            ("biz", "web_724"),
            ("fastColumn", column),
            ("sortEnd", ""),
            ("pageSize", "100"),
            ("req_trace", trace.as_str()),
        ])
        .header("Referer", "https://kuaixun.eastmoney.com/")
        .send()
        .await
        .map_err(|error| format!("快讯请求失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("快讯 HTTP 错误：{error}"))?
        .json()
        .await
        .map_err(|error| format!("快讯 JSON 解析失败：{error}"))?;
        items.extend(parse_fast(&value)?);
    }
    Ok(items)
}

async fn fetch_announcements() -> Result<Vec<RawNews>, String> {
    let mut items = Vec::new();
    // 接口单页最多返回 100 条；多取几页，覆盖集中发布公告的时段。
    for page in 1..=3 {
        let page_index = page.to_string();
        let value: Value = client()
            .get(ANNOUNCEMENT_URL)
            .query(&[
                ("sr", "-1"),
                ("page_size", "100"),
                ("page_index", page_index.as_str()),
                ("ann_type", "A"),
                ("stock_list", ""),
            ])
            .send()
            .await
            .map_err(|error| format!("公告请求失败：{error}"))?
            .error_for_status()
            .map_err(|error| format!("公告 HTTP 错误：{error}"))?
            .json()
            .await
            .map_err(|error| format!("公告 JSON 解析失败：{error}"))?;
        let page_items = parse_announcements(&value)?;
        let count = page_items.len();
        items.extend(page_items);
        if count < 100 {
            break;
        }
    }
    Ok(items)
}

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| text.contains(word))
}

fn classify(text: &str) -> Option<Route> {
    if contains_any(text, &["投资者关系", "一般决议", "公司章程", "法律意见书", "审计报告", "回购进展", "回购股份进展", "日常关联交易", "股票交易风险提示", "股票交易异常波动"]) {
        return None;
    }
    if contains_any(
        text,
        &[
            "立案",
            "处罚",
            "退市",
            "风险提示",
            "债务违约",
            "重大诉讼",
            "控制权变更",
        ],
    ) {
        return Some(Route {
            kind: "风险",
            popup: true,
        });
    }
    if contains_any(
        text,
        &[
            "停牌",
            "复牌",
            "重大资产重组",
            "发行股份购买资产",
            "并购重组",
        ],
    ) {
        return Some(Route {
            kind: "重大事项",
            popup: true,
        });
    }
    if contains_any(text, &["预亏", "首亏", "大幅下降", "业绩变脸"]) {
        return Some(Route {
            kind: "业绩风险",
            popup: true,
        });
    }
    if contains_any(
        text,
        &["业绩预告", "业绩快报", "预增", "预减", "利润增长", "净利润同比"],
    ) {
        return Some(Route {
            kind: "业绩",
            popup: false,
        });
    }
    if contains_any(
        text,
        &["权益分派", "分红", "回购", "增持", "减持", "限售股解禁"],
    ) {
        return Some(Route {
            kind: "资本事项",
            popup: false,
        });
    }
    if contains_any(
        text,
        &[
            "重大合同",
            "中标",
            "股权激励",
            "收购",
            "股权质押",
        ],
    ) {
        return Some(Route {
            kind: "公司事项",
            popup: false,
        });
    }
    if contains_any(text, &["降准", "降息", "出口管制", "关税", "行业补贴", "产业政策", "停产", "减产", "价格上调", "价格下调"]) {
        return Some(Route { kind: "政策行业", popup: true });
    }
    None
}

fn relevant_event(item: &RawNews, route: Route) -> bool {
    if item.source != "eastmoney_flash" || !item.codes.is_empty() {
        return true;
    }
    let text = format!("{} {}", item.title, item.body);
    // 无 A 股代码的财经快讯需要额外证据，避免体育、科技八卦等标题仅凭“处罚/收购”误入。
    (route.kind == "政策行业"
        && contains_any(&text, &["中国", "我国", "国内", "A股", "央行", "发改委", "工信部", "财政部", "国务院", "人民币", "沪深", "证监会", "关税", "出口管制"]))
        || contains_any(&text, &["A股", "上市公司", "沪深", "证监会", "上交所", "深交所", "北交所", "央企"])
}

fn target_pool(db: &Database) -> Result<HashMap<String, String>, String> {
    let mut targets = HashMap::new();
    for item in db.get_watchlist().map_err(|error| error.to_string())? {
        if let Some(code) = normalize_code(&item.code) {
            targets.entry(code).or_insert(item.name);
        }
    }
    Ok(targets)
}

fn related(item: &RawNews, targets: &HashMap<String, String>) -> Vec<(String, String)> {
    let text = format!("{} {}", item.title, item.body);
    let mut rows: Vec<_> = targets
        .iter()
        .filter(|(code, name)| {
            item.codes.contains(code)
                || (item.codes.is_empty() && !name.is_empty() && text.contains(name.as_str()))
        })
        .map(|(code, name)| (code.clone(), name.clone()))
        .collect();
    rows.sort();
    rows.dedup();
    rows
}

fn related_mainlines(item: &RawNews, targets: &[Value]) -> Vec<Value> {
    let text = format!("{} {}", item.title, item.body);
    targets.iter().filter_map(|target| {
        if target["snapshot_current"] != true || target["as_of"].as_str().unwrap_or_default().is_empty()
            || target["fingerprint"].as_str().unwrap_or_default().is_empty() { return None; }
        let symbols: Vec<String> = target["symbols"].as_array().into_iter().flatten()
            .filter_map(Value::as_str).filter_map(normalize_code).collect();
        let matched_symbols: Vec<_> = symbols.iter().filter(|s| item.codes.contains(s)).cloned().collect();
        let name_match = item.codes.is_empty() && target["names"].as_array().into_iter().flatten()
            .filter_map(Value::as_str).any(|name| name.chars().count() >= 2 && text.contains(name));
        let sector_name = target["sector_name"].as_str().unwrap_or_default();
        let theme_match = sector_name.chars().count() >= 2 && text.contains(sector_name);
        let basis = if !matched_symbols.is_empty() { "structured_symbol" } else if name_match { "name" }
            else if theme_match { "theme" } else { return None; };
        Some(serde_json::json!({"kind":target["kind"],"sector_code":target["sector_code"],
            "sector_name":sector_name,"as_of":target["as_of"],"fingerprint":target["fingerprint"],
            "match_basis":basis,"matched_symbols":matched_symbols,"price_effect_verified":false}))
    }).collect()
}

fn news_codes(news: &PreparedNews) -> Vec<String> {
    if news.item.codes.is_empty() {
        news.matches.iter().map(|(code, _)| code.clone()).collect()
    } else {
        news.item.codes.clone()
    }
}

fn stable_hash(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn dedupe_key(item: &RawNews) -> String {
    if item.source_id.is_empty() {
        format!(
            "{}:body:{}",
            item.source,
            stable_hash(&format!("{}\n{}", item.title, item.body))
        )
    } else {
        format!("{}:id:{}", item.source, item.source_id)
    }
}

#[cfg(test)]
fn prepare_source(
    db: &Database,
    targets: &HashMap<String, String>,
    init_key: &str,
    result: Result<Vec<RawNews>, String>,
) -> Vec<PreparedNews> {
    prepare_source_with_mainlines(db, targets, &[], init_key, result)
}

fn prepare_source_with_mainlines(
    db: &Database,
    targets: &HashMap<String, String>,
    mainlines: &[Value],
    init_key: &str,
    result: Result<Vec<RawNews>, String>,
) -> Vec<PreparedNews> {
    let items = match result {
        Ok(items) => items,
        Err(error) => {
            log::warn!(target: "automation::news", "资讯源检查失败：{error}");
            return Vec::new();
        }
    };
    let deliver = db.get_setting(init_key).ok().flatten().as_deref() == Some("1");
    let now = Utc::now().to_rfc3339();
    let mut prepared = Vec::new();
    for item in items {
        if item.published_at.as_deref().and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .is_some_and(|published| published.with_timezone(&Utc) > Utc::now()) { continue; }
        if item.published_date.as_deref().and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
            .is_some_and(|published| published > Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8*3600).unwrap()).date_naive()) { continue; }
        let matches = related(&item, targets);
        let related_mainlines = related_mainlines(&item, mainlines);
        let associated = !related_mainlines.is_empty();
        let Some(route) = classify(&format!("{} {}", item.title, item.body)).or_else(|| {
            associated.then_some(Route { kind: "主线资讯", popup: false })
        }) else {
            continue;
        };
        if !associated && !relevant_event(&item, route) {
            continue;
        }
        let key = dedupe_key(&item);
        match db.claim_news(&key, item.source, &now) {
            Ok(true) => {
                let fallback_body = if item.body.is_empty() {
                    item.title.clone()
                } else {
                    item.body.chars().take(240).collect()
                };
                let news = PreparedNews {
                    item,
                    matches,
                    route,
                    key,
                    fallback_body,
                    related_mainlines,
                };
                if deliver { prepared.push(news); }
                else if let Err(error) = db.archive_news(&news_payload(news)) {
                    log::warn!("[news] 初始化原文归档失败：{error}");
                }
            }
            Ok(_) => {}
            Err(error) => log::warn!("[news] 去重记录失败：{error}"),
        }
    }
    if !deliver {
        if let Err(error) = db.set_setting(init_key, "1") {
            log::warn!("[news] 初始化水位保存失败：{error}");
        }
    }
    prepared
}

fn news_poll_interval(now: chrono::DateTime<Utc>) -> Duration {
    let offset = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 is valid");
    let local = now.with_timezone(&offset);
    Duration::from_secs(if (7..23).contains(&local.hour()) {
        POLL_SECONDS
    } else {
        QUIET_POLL_SECONDS
    })
}

async fn poll_once(db: &Arc<Database>, app: &tauri::AppHandle) {
    // 老版本只采自选：升级后先重建两路水位，避免把当前全市场列表当新消息推送。
    if db.get_setting("news_scope_version").ok().flatten().as_deref() != Some("2") {
        for key in ["news_flash_initialized", "news_announcement_initialized"] {
            if let Err(error) = db.set_setting(key, "0") {
                log::warn!("[news] 重建全市场水位失败：{error}");
                return;
            }
        }
        if let Err(error) = db.set_setting("news_scope_version", "2") {
            log::warn!("[news] 保存全市场版本失败：{error}");
            return;
        }
    }
    let targets = match target_pool(db) {
        Ok(targets) => targets,
        Err(error) => {
            log::warn!("[news] 读取关注池失败：{error}");
            return;
        }
    };
    let mainlines = match crate::commands::mainline::news_targets(db) {
        Ok(rows) => rows,
        Err(error) => { log::warn!("[news] 主线资讯目标暂不可用：{error}"); Vec::new() }
    };
    let cutoff = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    if let Err(error) = db.purge_news_before(&cutoff) {
        log::warn!("[news] 清理过期去重记录失败：{error}");
    }
    let (fast, announcements) = tokio::join!(fetch_fast(), fetch_announcements());
    let mut prepared = prepare_source_with_mainlines(db, &targets, &mainlines, "news_flash_initialized", fast);
    prepared.extend(prepare_source_with_mainlines(
        db,
        &targets,
        &mainlines,
        "news_announcement_initialized",
        announcements,
    ));
    if prepared.is_empty() {
        return;
    }
    log::info!(target: "automation::news", "本轮收到 {} 条新资讯；仅通知本次启动后发布且时间有效的内容",prepared.len());
    let db = Arc::clone(db);
    let app = app.clone();
    // 通知逐条投递，资讯抓取继续按分钟运行。自动流程只发布原文。
    tauri::async_runtime::spawn(async move {
        deliver_prepared_news(&db, &app, prepared).await;
    });
}

fn news_payload(news: PreparedNews) -> Value {
    let codes = news_codes(&news);
    let received_at = if news.item.received_at.is_empty() { Utc::now().to_rfc3339() } else { news.item.received_at.clone() };
    let names = news
        .matches
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join("、");
    let title_subject = if names.is_empty() {
        news.item.title.chars().take(35).collect::<String>()
    } else {
        format!("★自选 {} · {}", names, news.item.title.chars().take(25).collect::<String>())
    };
    serde_json::json!({
        "signal_id": format!("news:{}", stable_hash(&news.key)),
        "signal_tag": news.route.kind,
        "signal_kind": "news",
        "occurred_at": Utc::now().to_rfc3339(),
        "title": format!("资讯 · {} · {}", news.route.kind, title_subject),
        "body": news.fallback_body,
        "alert_type": "news",
        "news_source": news.item.source_label,
        "news_source_id": news.item.source_id,
        "news_source_kind": news.item.source,
        "url": news.item.url,
        "source_url": if news.item.source == "eastmoney_announcement" { ANNOUNCEMENT_URL } else { FAST_URL },
        "published_at": news.item.published_at,
        "published_date": news.item.published_date,
        "publication_precision": news.item.publication_precision,
        "published_at_source": news.item.published_at_source,
        "received_at": received_at,
        "source_received_at": received_at,
        "source_index_only": news.item.source_index_only,
        "source_coverage": if news.item.source_index_only { "公告标题索引；未采集公告正文" } else { "快讯列表提供的标题及摘要；非完整报道" },
        "original_title": news.item.title,
        "original_body": news.item.body,
        "severity": if news.route.popup { "high" } else { "record" },
        "symbols": codes,
        "source_symbols": news.item.codes,
        "watchlist_names": names,
        "watchlist_match": !news.matches.is_empty(),
        "related_mainlines": news.related_mainlines,
        "research_target_match": !news.related_mainlines.is_empty(),
        "news_mode": "direct",
        "agent_summary": false,
    })
}

fn archived_ai_input(payload: &Value) -> Result<crate::agent::NewsAgentItem, String> {
    if payload["signal_kind"] != "news" { return Err("只能解读资讯记录".into()); }
    let id = payload["signal_id"].as_str().ok_or("资讯记录缺少 ID")?;
    let title = payload["original_title"].as_str().ok_or("资讯缺少原始标题")?;
    let body = payload["original_body"].as_str().unwrap_or_default();
    let symbols = payload["symbols"].as_array().into_iter().flatten()
        .filter_map(Value::as_str).filter_map(normalize_code).collect();
    Ok(crate::agent::NewsAgentItem {
        id: id.into(),
        source: payload["news_source"].as_str().unwrap_or("资讯").into(),
        title: title.chars().take(240).collect(),
        body: body.chars().take(500).collect(),
        symbols,
        rule_kind: payload["signal_tag"].as_str().unwrap_or("资讯").into(),
        severity: payload["severity"].as_str().unwrap_or("record").into(),
    })
}

fn apply_news_summary(payload: &mut Value, summary: &crate::agent::NewsSummaryItem) {
    let body = format!("事实：{}\n观点：{}\n方向：{}，影响：{}\n行业：{}；股票：{}\n依据：{}",
        summary.summary,
        summary.viewpoint,
        match summary.sentiment.as_str() { "positive" => "利好", "negative" => "利空", "neutral" => "中性", _ => "不确定" },
        match summary.impact_level.as_str() { "high" => "高", "medium" => "中", "low" => "低", _ => "不确定" },
        if summary.industries.is_empty() { "未确认".into() } else { format!("{}{}", summary.industries.join("、"), if summary.industry_basis == "inferred" { "（推测）" } else { "" }) },
        if summary.stocks.is_empty() && summary.stock_names.is_empty() { "未明确".into() } else { summary.stock_names.iter().chain(summary.stocks.iter()).cloned().collect::<Vec<_>>().join("、") },
        summary.evidence);
    payload["body"] = body.into();
    payload["agent_summary"] = true.into();
    payload["sentiment"] = summary.sentiment.clone().into();
    payload["impact_level"] = summary.impact_level.clone().into();
    payload["industries"] = serde_json::json!(summary.industries);
    payload["industry_basis"] = summary.industry_basis.clone().into();
    payload["related_stocks"] = serde_json::json!(summary.stocks);
    payload["related_stock_names"] = serde_json::json!(summary.stock_names);
    payload["confidence"] = summary.confidence.into();
}

#[tauri::command]
pub async fn analyze_archived_news(
    db: tauri::State<'_, Arc<Database>>,
    app: tauri::AppHandle,
    signal_id: String,
) -> Result<Value, String> {
    if !signal_id.starts_with("news:") || signal_id.len() > 80 { return Err("资讯 ID 无效".into()); }
    let mut payload = db.news_archive_item(&signal_id)?.ok_or("资讯记录不存在")?;
    if payload["agent_summary"] == true { return Ok(payload); }
    let _guard = AiRunGuard::claim(signal_id.clone())?;
    let input = archived_ai_input(&payload)?;
    let summary = crate::agent::summarize_news(&db, &[input]).await?
        .into_iter().next().ok_or("AI 未返回资讯解读")?;
    apply_news_summary(&mut payload, &summary);
    db.update_news_analysis(&signal_id, &payload)?;
    crate::notifications::news_analysis_updated(&app, &payload, false);
    Ok(payload)
}

async fn deliver_prepared_news(
    db: &Database,
    app: &tauri::AppHandle,
    prepared: Vec<PreparedNews>,
) {
    for news in prepared {
        if db.get_setting("news_notifications_enabled").ok().flatten().as_deref() != Some("1") { return; }
        crate::notifications::publish(app, news_payload(news));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tauri::command]
pub fn get_news_archive(
    db: tauri::State<'_, Arc<Database>>,
) -> Result<Vec<serde_json::Value>, String> {
    db.news_archive()
}

pub fn spawn(db: Arc<Database>, app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let enabled = db
                .get_setting("news_notifications_enabled")
                .ok()
                .flatten()
                .as_deref()
                == Some("1");
            if enabled {
                poll_once(&db, &app).await;
            }
            if let Err(error) = crate::daily_brief::ensure_due_brief(&db, Utc::now()) {
                log::warn!("[news] 每日简报生成失败：{error}");
            }
            tokio::time::sleep(if enabled { news_poll_interval(Utc::now()) } else { Duration::from_secs(POLL_SECONDS) }).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[tokio::test]
    #[ignore = "requires live Eastmoney news endpoints"]
    async fn live_sources_return_parseable_news() {
        let fast = fetch_fast().await.unwrap();
        let announcements = fetch_announcements().await.unwrap();
        assert!(!fast.is_empty());
        assert!(!announcements.is_empty());
        assert!(announcements.iter().any(|item| !item.codes.is_empty()));
    }

    #[test]
    fn whitelist_and_target_matching_reject_unrelated_news() {
        assert!(classify("董事会一般决议").is_none());
        assert!(classify("投资者关系活动记录").is_none());
        assert!(classify("年度报告摘要").is_none());
        assert!(classify("回购股份进展公告").is_none());
        assert!(classify("股票交易风险提示公告").is_none());
        assert_eq!(
            classify("收到立案告知书"),
            Some(Route {
                kind: "风险",
                popup: true
            })
        );
        let targets = HashMap::from([("600519".into(), "贵州茅台".into())]);
        let mut item = RawNews {
            source: "test",
            source_label: "测试",
            source_id: "1".into(),
            title: "别家公司年度报告".into(),
            body: String::new(),
            codes: vec!["000001".into()],
            ..Default::default()
        };
        assert!(related(&item, &targets).is_empty());
        item.codes = vec!["600519".into()];
        assert_eq!(related(&item, &targets)[0].0, "600519");
        item.codes = vec!["000001".into()];
        item.title = "贵州茅台同行年度报告".into();
        assert!(related(&item, &targets).is_empty(), "明确代码不是自选时不应靠名称误标");
        item.codes.clear();
        assert_eq!(related(&item, &targets)[0].0, "600519");
    }

    #[test]
    fn code_free_financial_flash_needs_a_share_context() {
        let mut item = RawNews {
            source: "eastmoney_flash", source_label: "快讯", source_id: "1".into(),
            title: "英超世纪财务案：曼城面临处罚".into(), body: String::new(), codes: vec![],
            ..Default::default()
        };
        assert!(!relevant_event(&item, classify(&item.title).unwrap()));
        item.title = "央行宣布降准，支持国内实体经济".into();
        assert!(relevant_event(&item, classify(&item.title).unwrap()));
        item.title = "包钢股份中标某央企海工采购项目".into();
        assert!(relevant_event(&item, classify(&item.title).unwrap()));
        item.codes.push("600010".into());
        item.title = "公司收购新业务".into();
        assert!(relevant_event(&item, classify(&item.title).unwrap()));
    }

    #[test]
    fn automatic_delivery_keeps_raw_payload_for_major_and_watchlist_news() {
        for (popup, matches) in [(true, vec![]), (false, vec![("600000".into(), "自选".into())])] {
            let payload = news_payload(PreparedNews {
                item: RawNews { source: "test", source_label: "测试", source_id: "raw".into(), title: "重大风险原文".into(), body: "公告正文".into(), codes: vec!["600000".into()], ..Default::default() },
                matches, route: Route { kind: "风险", popup }, key: "raw".into(), fallback_body: "公告正文".into(),
                related_mainlines: vec![],
            });
            assert_eq!(payload["news_mode"], "direct");
            assert_eq!(payload["agent_summary"], false);
            assert_eq!(payload["original_body"], "公告正文");
            assert_eq!(payload["body"], "公告正文");
            assert!(archived_ai_input(&payload).is_ok(), "manual source remains available");
        }
    }

    #[test]
    fn manual_analysis_uses_only_archived_source_and_updates_same_record() {
        let dir = std::env::temp_dir().join(format!("bull-news-manual-{}-{}", std::process::id(), Utc::now().timestamp_micros()));
        let db = Database::open(dir.clone()).unwrap();
        let id = "news:manual-check";
        let mut payload = serde_json::json!({
            "signal_id": id, "signal_kind": "news", "signal_tag": "公司事项",
            "title": "资讯 · 中标", "original_title": "甲公司中标行业项目",
            "original_body": "甲公司公告中标", "symbols": ["600001"],
            "body": "甲公司公告中标", "agent_summary": false,
        });
        db.archive_news(&payload).unwrap();
        let input = archived_ai_input(&db.news_archive_item(id).unwrap().unwrap()).unwrap();
        assert_eq!(input.title, "甲公司中标行业项目");
        assert_eq!(input.symbols, vec!["600001"]);
        assert!(archived_ai_input(&serde_json::json!({"signal_kind":"timeline","signal_id":id})).is_err());
        let summary = crate::agent::NewsSummaryItem {
            id: id.into(), summary: "甲公司中标".into(), viewpoint: "合同金额未知，影响待核实".into(),
            sentiment: "positive".into(), impact_level: "uncertain".into(), industries: vec![],
            industry_basis: "unknown".into(), stocks: vec!["600001".into()], stock_names: vec!["甲公司".into()],
            confidence: 60, evidence: "中标".into(),
        };
        apply_news_summary(&mut payload, &summary);
        db.update_news_analysis(id, &payload).unwrap();
        let rows = db.news_archive().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["signal_id"], id);
        assert_eq!(rows[0]["agent_summary"], true);
        assert!(rows[0]["body"].as_str().unwrap().contains("观点："));
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn same_story_cannot_start_two_claude_runs_at_once() {
        let guard = AiRunGuard::claim("news:guard-check".into()).unwrap();
        assert!(AiRunGuard::claim("news:guard-check".into()).is_err());
        drop(guard);
        assert!(AiRunGuard::claim("news:guard-check".into()).is_ok());
    }

    #[test]
    fn all_market_news_works_without_watchlist_and_keeps_first_poll_quiet() {
        let dir = std::env::temp_dir().join(format!(
            "bull-arrives-news-all-market-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        let db = Database::open(dir.clone()).unwrap();
        let item = RawNews {
            source: "test",
            source_label: "测试",
            source_id: "old".into(),
            title: "非自选公司收到立案告知书".into(),
            body: String::new(),
            codes: vec!["600001".into()],
            ..Default::default()
        };
        let targets = HashMap::new();
        assert!(prepare_source(&db, &targets, "news_flash_initialized", Ok(vec![item.clone()])).is_empty());
        let mut fresh = item;
        fresh.source_id = "new".into();
        let rows = prepare_source(&db, &targets, "news_flash_initialized", Ok(vec![fresh.clone()]));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].matches.is_empty());
        let payload = news_payload(rows.into_iter().next().unwrap());
        assert_eq!(payload["news_mode"], "direct");
        assert_eq!(payload["agent_summary"], false);
        assert_eq!(payload["watchlist_match"], false);
        assert_eq!(payload["symbols"][0], "600001");
        db.archive_news(&payload).unwrap();
        assert_eq!(db.news_archive().unwrap()[0]["news_mode"], "direct");
        assert!(prepare_source(&db, &targets, "news_flash_initialized", Ok(vec![fresh])).is_empty());
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ai_notification_displays_impact_industry_stocks_and_marks_watchlist() {
        let news = PreparedNews {
            item: RawNews { source: "test", source_label: "测试", source_id: "1".into(), title: "公司利润预亏".into(), body: String::new(), codes: vec!["600001".into()], ..Default::default() },
            matches: vec![("600001".into(), "测试自选".into())],
            route: classify("利润预亏").unwrap(),
            key: "test:1".into(),
            fallback_body: "公司利润预亏".into(),
            related_mainlines: vec![],
        };
        let summary = crate::agent::NewsSummaryItem {
            id: "test:1".into(), summary: "公司预亏".into(), viewpoint: "盈利压力可能偏利空，影响仍需核实".into(), sentiment: "negative".into(), impact_level: "medium".into(), industries: vec!["制造业".into()], industry_basis: "inferred".into(), stocks: vec!["600001".into()], stock_names: vec![], confidence: 60, evidence: "利润预亏".into(),
        };
        let mut payload = news_payload(news);
        apply_news_summary(&mut payload, &summary);
        assert!(payload["title"].as_str().unwrap().contains("★自选 测试自选"));
        let body = payload["body"].as_str().unwrap();
        for text in ["事实：", "观点：", "利空", "影响：中", "制造业（推测）", "600001", "依据：利润预亏"] {
            assert!(body.contains(text), "{text} must appear in the notification");
        }
        assert_eq!(payload["watchlist_match"], true);
        assert_eq!(payload["agent_summary"], true);
    }

    #[test]
    fn news_polls_after_close_and_overnight() {
        let utc = |hour, minute| {
            Utc.with_ymd_and_hms(2026, 9, 18, hour, minute, 0)
                .single()
                .unwrap()
        };
        assert_eq!(news_poll_interval(utc(0, 0)), Duration::from_secs(60)); // 北京 08:00
        assert_eq!(news_poll_interval(utc(8, 0)), Duration::from_secs(60)); // 北京 16:00
        assert_eq!(news_poll_interval(utc(16, 0)), Duration::from_secs(300)); // 北京 00:00

    }

    #[test]
    fn body_hash_is_stable_when_source_has_no_id() {
        let item = RawNews {
            source: "test",
            source_label: "测试",
            source_id: String::new(),
            title: "标题".into(),
            body: "正文".into(),
            codes: Vec::new(),
            ..Default::default()
        };
        assert_eq!(dedupe_key(&item), dedupe_key(&item));
        assert!(dedupe_key(&item).starts_with("test:body:"));
    }

    #[test]
    fn verified_publication_metadata_and_mainline_original_news_keep_time_boundaries() {
        let dir=std::env::temp_dir().join(format!("bull-mainline-news-{}-{}",std::process::id(),Utc::now().timestamp_micros()));
        let db=Database::open(dir.clone()).unwrap();
        db.set_setting("news_flash_initialized","1").unwrap();
        let now=Utc::now()-chrono::Duration::seconds(2);
        let local=now.with_timezone(&chrono::FixedOffset::east_opt(8*3600).unwrap());
        let fast=parse_fast(&serde_json::json!({"code":"1","data":{"fastNewsList":[{
            "code":"real-format-fixture","title":"恒瑞医药研发项目进度","summary":"源接口摘要", "stockList":["1.600276"],
            "showTime":local.format("%Y-%m-%d %H:%M:%S").to_string()}]}})).unwrap();
        assert_eq!(fast[0].published_at_source,"showTime");
        assert!(fast[0].published_at.as_ref().unwrap().ends_with("+08:00"));
        assert!(fast[0].url.is_none(),"列表没有链接字段，不能猜造快讯 URL");
        let ann=parse_announcements(&serde_json::json!({"success":1,"data":{"list":[{
            "art_code":"AN202610011830059731","title_ch":"贝泰妮投资者关系活动记录表",
            "codes":[{"stock_code":"300957"}],"display_time":"2026-10-01 00:37:09:681",
            "notice_date":"2026-10-01 00:00:00","sort_date":"2099-01-01 12:00:00"}]}})).unwrap();
        assert_eq!(ann[0].published_at.as_deref(),Some("2026-10-01T00:37:09.681+08:00"));
        assert_eq!(ann[0].published_at_source,"display_time");
        assert!(ann[0].source_index_only && ann[0].body.is_empty());
        assert_eq!(ann[0].url.as_deref(),Some("https://data.eastmoney.com/notices/detail/300957/AN202610011830059731.html"));
        let targets=vec![serde_json::json!({"kind":"industry","sector_code":"801150","sector_name":"医药生物",
            "as_of":"2026-09-30","fingerprint":"fixture","snapshot_current":true,
            "symbols":["sh600276"],"names":["恒瑞医药"]}),
            serde_json::json!({"kind":"concept","sector_code":"stale","sector_name":"研发",
                "as_of":"2026-09-29","fingerprint":"stale","snapshot_current":false,"symbols":["sh600276"]})];
        let prepared=prepare_source_with_mainlines(&db,&HashMap::new(),&targets,"news_flash_initialized",Ok(fast.clone()));
        assert_eq!(prepared.len(),1,"主线原文无需事件白名单中的关键词");
        let payload=news_payload(prepared.into_iter().next().unwrap());
        assert_eq!(payload["related_mainlines"].as_array().unwrap().len(),1);
        assert_eq!(payload["related_mainlines"][0]["match_basis"],"structured_symbol");
        assert_eq!(payload["watchlist_match"],false);
        assert_eq!(payload["agent_summary"],false);
        db.archive_news(&payload).unwrap();
        assert!(prepare_source_with_mainlines(&db,&HashMap::new(),&targets,"news_flash_initialized",Ok(fast)).is_empty());
        let mut future=payload.clone();future["signal_id"]="news:future-received".into();
        future["source_received_at"]=(Utc::now()+chrono::Duration::days(1)).to_rfc3339().into();
        db.archive_news(&future).unwrap();
        let mut future=payload.clone();future["signal_id"]="news:future-publication".into();
        future["published_at"]=(Utc::now()+chrono::Duration::days(1)).to_rfc3339().into();
        db.archive_news(&future).unwrap();
        let mut manual=payload.clone();manual["body"]="后来手动 AI 观点".into();manual["agent_summary"]=true.into();
        db.update_news_analysis(payload["signal_id"].as_str().unwrap(),&manual).unwrap();
        let as_of=(local.date_naive()-chrono::Duration::days(1)).to_string();
        let rows=db.recent_research_news(&[],&["sh600276".into()],&[],&as_of).unwrap();
        assert_eq!(rows.len(),1,"未来采集/发布时间不能进入近期研究证据");
        assert_eq!(rows[0]["body"],"源接口摘要","只引用原文，不把后续 AI 观点当资讯事实");
        assert_eq!(rows[0]["available_for_market_asof"],false);
        assert_eq!(rows[0]["coverage_complete"],false);
        assert_eq!(rows[0]["match_basis"],"structured_symbol");
        let inferred=news_payload(PreparedNews {
            item:RawNews {source:"test",source_label:"测试",source_id:"name-only".into(),title:"恒瑞医药研发进度".into(),
                body:"名称关联摘要".into(),received_at:Utc::now().to_rfc3339(),..Default::default()},
            matches:vec![("600276".into(),"恒瑞医药".into())],route:Route{kind:"公司事项",popup:false},key:"name-only".into(),
            fallback_body:"名称关联摘要".into(),related_mainlines:vec![],
        });
        assert!(inferred["source_symbols"].as_array().unwrap().is_empty());
        assert_eq!(inferred["symbols"][0],"600276");
        db.archive_news(&inferred).unwrap();
        let rows=db.recent_research_news(&[],&["sh600276".into()],&["恒瑞医药".into()],&as_of).unwrap();
        let row=rows.iter().find(|r|r["source_id"]=="name-only").unwrap();
        assert_eq!(row["match_basis"],"name");
        assert_ne!(row["match_basis"],"structured_symbol","名称推断不得改称源结构化代码");
        drop(db);std::fs::remove_dir_all(dir).unwrap();
    }
}
