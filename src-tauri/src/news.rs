//! 全市场资讯：独立轮询、事件白名单、自选标记与持久化去重。

use crate::db::Database;
use chrono::{Timelike, Utc};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawNews {
    source: &'static str,
    source_label: &'static str,
    source_id: String,
    title: String,
    body: String,
    codes: Vec<String>,
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
}

fn auto_ai_budget(db: &Database) -> u32 {
    db.get_setting("news_ai_daily_limit").ok().flatten()
        .and_then(|value| value.parse::<u32>().ok()).unwrap_or(3).min(20)
}

fn choose_auto_ai(db: &Database, prepared: &[PreparedNews], mode: &str) -> HashSet<String> {
    let mut selected = HashSet::new();
    if mode != "hybrid" || db.get_setting("ai_enabled").ok().flatten().as_deref() == Some("0") {
        return selected;
    }
    let keywords_setting = db.get_setting("news_ai_keywords").ok().flatten().unwrap_or_default();
    let keywords = keywords_setting.split([',', '，']).map(str::trim).filter(|word| !word.is_empty()).collect::<Vec<_>>();
    let limit = auto_ai_budget(db);
    let mut eligible = prepared.iter().filter(|news| ai_trigger(news, &keywords).is_some()).collect::<Vec<_>>();
    eligible.sort_by_key(|news| if news.route.popup { 0 } else if !news.matches.is_empty() { 1 } else { 2 });
    let day = local_day();
    for news in eligible {
        match db.claim_news_auto_ai(&day, &news.key, limit) {
            Ok(true) => { selected.insert(news.key.clone()); }
            Ok(false) => {}
            Err(error) => log::warn!("[news] 自动 AI 名额登记失败，改为直接通知：{error}"),
        }
    }
    selected
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
            Some(RawNews {
                source: "eastmoney_flash",
                source_label: "东方财富财经 / 上市公司快讯",
                source_id: string(item.get("code")),
                title,
                body,
                codes,
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
            let codes = item
                .get("codes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|code| code.get("stock_code").and_then(Value::as_str))
                .filter_map(normalize_code)
                .collect();
            Some(RawNews {
                source: "eastmoney_announcement",
                source_label: "东方财富公司公告",
                source_id: string(item.get("art_code")),
                body: String::new(),
                title,
                codes,
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

fn news_codes(news: &PreparedNews) -> Vec<String> {
    if news.item.codes.is_empty() {
        news.matches.iter().map(|(code, _)| code.clone()).collect()
    } else {
        news.item.codes.clone()
    }
}

fn ai_trigger(news: &PreparedNews, keywords: &[&str]) -> Option<&'static str> {
    if news.route.popup { return Some("重大事件"); }
    if !news.matches.is_empty() { return Some("自选股"); }
    let text = format!("{} {}", news.item.title, news.item.body);
    keywords.iter().any(|word| text.contains(word)).then_some("关注词")
}

fn local_day() -> String {
    Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8"))
        .format("%Y-%m-%d")
        .to_string()
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

fn prepare_source(
    db: &Database,
    targets: &HashMap<String, String>,
    init_key: &str,
    result: Result<Vec<RawNews>, String>,
) -> Vec<PreparedNews> {
    let items = match result {
        Ok(items) => items,
        Err(error) => {
            log::warn!("[news] {error}");
            return Vec::new();
        }
    };
    let deliver = db.get_setting(init_key).ok().flatten().as_deref() == Some("1");
    let now = Utc::now().to_rfc3339();
    let mut prepared = Vec::new();
    for item in items {
        let matches = related(&item, targets);
        let Some(route) = classify(&format!("{} {}", item.title, item.body)) else {
            continue;
        };
        if !relevant_event(&item, route) {
            continue;
        }
        let key = dedupe_key(&item);
        match db.claim_news(&key, item.source, &now) {
            Ok(true) if deliver => {
                let fallback_body = if item.body.is_empty() {
                    item.title.clone()
                } else {
                    item.body.chars().take(240).collect()
                };
                prepared.push(PreparedNews {
                    item,
                    matches,
                    route,
                    key,
                    fallback_body,
                });
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
    let cutoff = (Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    if let Err(error) = db.purge_news_before(&cutoff) {
        log::warn!("[news] 清理过期去重记录失败：{error}");
    }
    let (fast, announcements) = tokio::join!(fetch_fast(), fetch_announcements());
    let mut prepared = prepare_source(db, &targets, "news_flash_initialized", fast);
    prepared.extend(prepare_source(
        db,
        &targets,
        "news_announcement_initialized",
        announcements,
    ));
    if prepared.is_empty() {
        return;
    }
    let mode = db.get_setting("news_notification_mode").ok().flatten().unwrap_or_else(|| "direct".into());
    let db = Arc::clone(db);
    let app = app.clone();
    // AI 可以花数分钟，资讯抓取必须继续按分钟运行，避免集中公告时漏掉后续页。
    tauri::async_runtime::spawn(async move {
        deliver_prepared_news(&db, &app, prepared, &mode).await;
    });
}

fn news_payload(news: PreparedNews, summary: Option<&crate::agent::NewsSummaryItem>, ai_mode: bool) -> Value {
    let codes = news_codes(&news);
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
    let mut payload = serde_json::json!({
        "signal_id": format!("news:{}", stable_hash(&news.key)),
        "signal_tag": news.route.kind,
        "signal_kind": "news",
        "occurred_at": Utc::now().to_rfc3339(),
        "title": format!("资讯 · {} · {}", news.route.kind, title_subject),
        "body": news.fallback_body,
        "alert_type": "news",
        "news_source": news.item.source_label,
        "news_source_id": news.item.source_id,
        "original_title": news.item.title,
        "original_body": news.item.body,
        "severity": if news.route.popup { "high" } else { "record" },
        "symbols": codes,
        "watchlist_names": names,
        "watchlist_match": !news.matches.is_empty(),
        "news_mode": if ai_mode { "ai" } else { "direct" },
        "agent_summary": false,
    });
    if let Some(summary) = summary { apply_news_summary(&mut payload, summary); }
    payload
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

#[tauri::command]
pub fn get_news_ai_usage(db: tauri::State<'_, Arc<Database>>) -> Result<Value, String> {
    let day = local_day();
    let used = db.news_auto_ai_used(&day).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"day": day, "used": used, "limit": auto_ai_budget(&db)}))
}

async fn deliver_prepared_news(
    db: &Database,
    app: &tauri::AppHandle,
    prepared: Vec<PreparedNews>,
    mode: &str,
) {
    static AI_BATCHES: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let ai_permit = (mode == "hybrid")
        .then(|| AI_BATCHES.get_or_init(|| tokio::sync::Semaphore::new(2)).try_acquire().ok())
        .flatten();
    let selected = if ai_permit.is_some() { choose_auto_ai(db, &prepared, mode) } else { HashSet::new() };
    let (ai_news, direct_news): (Vec<_>, Vec<_>) = prepared.into_iter().partition(|news| selected.contains(news.key.as_str()));
    for news in direct_news {
        if db.get_setting("news_notifications_enabled").ok().flatten().as_deref() != Some("1") { return; }
        crate::notifications::publish(app, news_payload(news, None, false));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if ai_news.is_empty() { return; }
    for news in &ai_news {
        if db.get_setting("news_notifications_enabled").ok().flatten().as_deref() != Some("1") { return; }
        let mut payload = news_payload(PreparedNews {
            item: news.item.clone(), matches: news.matches.clone(), route: news.route,
            key: news.key.clone(), fallback_body: news.fallback_body.clone(),
        }, None, false);
        payload["news_mode"] = "hybrid".into();
        crate::notifications::publish(app, payload);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let mut summaries = HashMap::new();
    let mut pending = ai_news.into_iter();
    loop {
            let batch = pending.by_ref().take(20).collect::<Vec<_>>();
            if batch.is_empty() { break; }
            let chunk = batch.into_iter().filter_map(|news| {
                let id = format!("news:{}", stable_hash(&news.key));
                AiRunGuard::claim(id).ok().map(|guard| (news, guard))
            }).collect::<Vec<_>>();
            if chunk.is_empty() { continue; }
            if db.get_setting("news_notifications_enabled").ok().flatten().as_deref() != Some("1") {
                return;
            }
            let agent_items = chunk.iter().map(|(news, _guard)| crate::agent::NewsAgentItem {
                id: news.key.clone(),
                source: news.item.source_label.into(),
                title: news.item.title.chars().take(240).collect(),
                body: news.item.body.chars().take(500).collect(),
                symbols: news_codes(news),
                rule_kind: news.route.kind.into(),
                severity: if news.route.popup { "high" } else { "record" }.into(),
            }).collect::<Vec<_>>();
            match crate::agent::summarize_news(db, &agent_items).await {
                Ok(items) => summaries.extend(items.into_iter().map(|item| (item.id.clone(), item))),
                Err(error) => log::warn!("[news] Agent 解读降级为原文：{error}"),
            }
            for (news, _guard) in chunk {
                if db.get_setting("news_notifications_enabled").ok().flatten().as_deref() != Some("1") {
                    return;
                }
                let summary = summaries.get(&news.key);
                if let Some(summary) = summary {
                    let id = format!("news:{}", stable_hash(&news.key));
                    if let Ok(Some(mut payload)) = db.news_archive_item(&id) {
                        apply_news_summary(&mut payload, summary);
                        if db.update_news_analysis(&id, &payload).is_ok() {
                            crate::notifications::news_analysis_updated(app, &payload, true);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
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
    fn hybrid_routes_major_watchlist_and_keywords_but_caps_claude_calls() {
        let dir = std::env::temp_dir().join(format!("bull-news-hybrid-{}-{}", std::process::id(), Utc::now().timestamp_micros()));
        let db = Database::open(dir.clone()).unwrap();
        db.set_setting("news_ai_daily_limit", "2").unwrap();
        db.set_setting("news_ai_keywords", "机器人,半导体").unwrap();
        let make = |key: &str, title: &str, popup: bool, matches: Vec<(String, String)>| PreparedNews {
            item: RawNews { source: "test", source_label: "测试", source_id: key.into(), title: title.into(), body: String::new(), codes: vec![] },
            matches, route: Route { kind: "测试", popup }, key: key.into(), fallback_body: title.into(),
        };
        let news = vec![
            make("ordinary", "普通资讯", false, vec![]),
            make("keyword", "机器人相关公告", false, vec![]),
            make("watch", "自选股消息", false, vec![("600000".into(), "自选".into())]),
            make("major", "重大风险", true, vec![]),
        ];
        let selected = choose_auto_ai(&db, &news, "hybrid");
        assert_eq!(selected.len(), 2);
        assert!(selected.contains("major"));
        assert!(selected.contains("watch"));
        assert!(!selected.contains("ordinary"));
        assert!(!selected.contains("keyword"));
        assert!(choose_auto_ai(&db, &news, "direct").is_empty());
        assert_eq!(db.news_auto_ai_used(&local_day()).unwrap(), 2);
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
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
        };
        let targets = HashMap::new();
        assert!(prepare_source(&db, &targets, "news_flash_initialized", Ok(vec![item.clone()])).is_empty());
        let mut fresh = item;
        fresh.source_id = "new".into();
        let rows = prepare_source(&db, &targets, "news_flash_initialized", Ok(vec![fresh.clone()]));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].matches.is_empty());
        let payload = news_payload(rows.into_iter().next().unwrap(), None, false);
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
            item: RawNews { source: "test", source_label: "测试", source_id: "1".into(), title: "公司利润预亏".into(), body: String::new(), codes: vec!["600001".into()] },
            matches: vec![("600001".into(), "测试自选".into())],
            route: classify("利润预亏").unwrap(),
            key: "test:1".into(),
            fallback_body: "公司利润预亏".into(),
        };
        let summary = crate::agent::NewsSummaryItem {
            id: "test:1".into(), summary: "公司预亏".into(), viewpoint: "盈利压力可能偏利空，影响仍需核实".into(), sentiment: "negative".into(), impact_level: "medium".into(), industries: vec!["制造业".into()], industry_basis: "inferred".into(), stocks: vec!["600001".into()], stock_names: vec![], confidence: 60, evidence: "利润预亏".into(),
        };
        let payload = news_payload(news, Some(&summary), true);
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
        };
        assert_eq!(dedupe_key(&item), dedupe_key(&item));
        assert!(dedupe_key(&item).starts_with("test:body:"));
    }
}
