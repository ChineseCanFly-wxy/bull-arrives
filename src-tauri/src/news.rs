//! 规则版盘中资讯：独立轮询、关注池匹配、白名单分级与持久化去重。

use crate::datasource::market_policy::MarketRequestPolicy;
use crate::db::Database;
use chrono::{Timelike, Utc};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

const FAST_URL: &str = "https://np-weblist.eastmoney.com/comm/web/getFastNewsList";
const ANNOUNCEMENT_URL: &str = "https://np-anotice-stock.eastmoney.com/api/security/ann";
const POLL_SECONDS: u64 = 60;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineStage {
    PreOpen,
    PostClose,
    Night,
}

impl TimelineStage {
    fn key(self) -> &'static str {
        match self {
            Self::PreOpen => "preopen",
            Self::PostClose => "postclose",
            Self::Night => "night",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PreOpen => "盘前策略卡",
            Self::PostClose => "盘后复盘",
            Self::Night => "夜间统计",
        }
    }
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
                source_label: "东方财富上市公司快讯",
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
            let columns = item
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|column| column.get("column_name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("、");
            Some(RawNews {
                source: "eastmoney_announcement",
                source_label: "东方财富公司公告",
                source_id: string(item.get("art_code")),
                body: columns,
                title,
                codes,
            })
        })
        .collect())
}

async fn fetch_fast() -> Result<Vec<RawNews>, String> {
    let trace = Utc::now().timestamp_millis().to_string();
    let value: Value = client()
        .get(FAST_URL)
        .query(&[
            ("client", "web"),
            ("biz", "web_724"),
            ("fastColumn", "103"),
            ("sortEnd", ""),
            ("pageSize", "50"),
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
    parse_fast(&value)
}

async fn fetch_announcements(codes: &[String]) -> Result<Vec<RawNews>, String> {
    let stock_list = codes.join(",");
    let value: Value = client()
        .get(ANNOUNCEMENT_URL)
        .query(&[
            ("sr", "-1"),
            ("page_size", "50"),
            ("page_index", "1"),
            ("ann_type", "A"),
            ("stock_list", stock_list.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("公告请求失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("公告 HTTP 错误：{error}"))?
        .json()
        .await
        .map_err(|error| format!("公告 JSON 解析失败：{error}"))?;
    parse_announcements(&value)
}

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| text.contains(word))
}

fn classify(text: &str) -> Option<Route> {
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
        &["业绩预告", "业绩快报", "年度报告", "半年度报告", "季度报告"],
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
            "担保",
            "质押",
            "投资者关系",
        ],
    ) {
        return Some(Route {
            kind: "公司事项",
            popup: false,
        });
    }
    None
}

fn target_pool(db: &Database) -> Result<HashMap<String, String>, String> {
    let mut targets = HashMap::new();
    for item in db.get_watchlist().map_err(|error| error.to_string())? {
        if let Some(code) = normalize_code(&item.code) {
            targets.entry(code).or_insert(item.name);
        }
    }
    for item in db.get_monitors().map_err(|error| error.to_string())? {
        if item.enabled {
            if let Some(code) = normalize_code(&item.code) {
                targets.entry(code).or_insert(item.name);
            }
        }
    }
    Ok(targets)
}

fn related(item: &RawNews, targets: &HashMap<String, String>) -> Vec<(String, String)> {
    let text = format!("{} {}", item.title, item.body);
    let mut rows: Vec<_> = targets
        .iter()
        .filter(|(code, name)| {
            item.codes.contains(code) || (!name.is_empty() && text.contains(name.as_str()))
        })
        .map(|(code, name)| (code.clone(), name.clone()))
        .collect();
    rows.sort();
    rows.dedup();
    rows
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
        if matches.is_empty() {
            continue;
        }
        let Some(route) = classify(&format!("{} {}", item.title, item.body)) else {
            continue;
        };
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

fn policy(db: &Database) -> Option<MarketRequestPolicy> {
    let value = db.get_setting("quote_schedule").ok().flatten();
    match MarketRequestPolicy::from_quote_schedule_json(value.as_deref()) {
        Ok(policy) => Some(policy),
        Err(error) => {
            log::warn!("[news] 交易日配置无效，资讯与时序任务暂停：{error}");
            None
        }
    }
}

fn should_poll_at(now: chrono::DateTime<Utc>, policy: &MarketRequestPolicy) -> bool {
    if !policy.is_trading_day_at(now) {
        return false;
    }
    let offset = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 is valid");
    let local = now.with_timezone(&offset);
    let minute = local.hour() * 60 + local.minute();
    (8 * 60 + 30..11 * 60 + 30).contains(&minute) || (13 * 60..15 * 60).contains(&minute)
}

fn timeline_stage_at(
    now: chrono::DateTime<Utc>,
    policy: &MarketRequestPolicy,
) -> Option<TimelineStage> {
    if !policy.is_trading_day_at(now) {
        return None;
    }
    let offset = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 is valid");
    let minute = now
        .with_timezone(&offset)
        .time()
        .num_seconds_from_midnight()
        / 60;
    match minute {
        540..565 => Some(TimelineStage::PreOpen),
        905..1020 => Some(TimelineStage::PostClose),
        1200..1440 => Some(TimelineStage::Night),
        _ => None,
    }
}

fn publish_timeline(db: &Database, app: &tauri::AppHandle, stage: TimelineStage) {
    let offset = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 is valid");
    let day = Utc::now()
        .with_timezone(&offset)
        .format("%Y-%m-%d")
        .to_string();
    let setting = format!("timeline_{}_day", stage.key());
    if db.get_setting(&setting).ok().flatten().as_deref() == Some(day.as_str()) {
        return;
    }
    let watch_count = db
        .get_watchlist()
        .map(|items| items.len())
        .unwrap_or_default();
    let monitors = db.get_monitors().unwrap_or_default();
    let monitor_count = monitors.iter().filter(|item| item.enabled).count();
    let alerts = db.get_all_price_alerts().unwrap_or_default();
    let alert_count = alerts.iter().filter(|item| item.enabled).count();
    let triggered_today = alerts
        .iter()
        .filter(|item| item.last_triggered_day.as_deref() == Some(day.as_str()))
        .count();
    let body = match stage {
        TimelineStage::PreOpen => format!(
            "今日关注 {watch_count} 只，启用智能监控 {monitor_count} 条、价格规则 {alert_count} 条；盘中只在代码规则或事件白名单命中时提醒。"
        ),
        TimelineStage::PostClose => format!(
            "今日价格规则触发 {triggered_today} 条；当前关注 {watch_count} 只、启用智能监控 {monitor_count} 条。"
        ),
        TimelineStage::Night => format!(
            "夜间基线：关注 {watch_count} 只、价格规则 {alert_count} 条、智能监控 {monitor_count} 条；未命中规则不调用 Agent。"
        ),
    };
    if let Err(error) = db.set_setting(&setting, &day) {
        log::warn!("[news] 时序任务去重记录失败：{error}");
        return;
    }
    crate::notifications::record_only(
        app,
        serde_json::json!({
            "signal_id": format!("timeline:{day}:{}", stage.key()),
            "signal_tag": stage.label(),
            "signal_kind": "timeline",
            "occurred_at": Utc::now().to_rfc3339(),
            "title": format!("时序盯盘 · {}", stage.label()),
            "body": body,
            "alert_type": "timeline",
            "severity": "record",
            "symbols": [],
        }),
    );
}

async fn poll_once(db: &Database, app: &tauri::AppHandle) {
    let targets = match target_pool(db) {
        Ok(targets) if !targets.is_empty() => targets,
        Ok(_) => return,
        Err(error) => {
            log::warn!("[news] 读取关注池失败：{error}");
            return;
        }
    };
    let cutoff = (Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    if let Err(error) = db.purge_news_before(&cutoff) {
        log::warn!("[news] 清理过期去重记录失败：{error}");
    }
    let mut codes: Vec<_> = targets.keys().cloned().collect();
    codes.sort();
    let (fast, announcements) = tokio::join!(fetch_fast(), fetch_announcements(&codes));
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
    let agent_items = prepared
        .iter()
        .take(20)
        .map(|news| crate::agent::NewsAgentItem {
            id: news.key.clone(),
            source: news.item.source_label.into(),
            title: news.item.title.chars().take(240).collect(),
            body: news.item.body.chars().take(500).collect(),
            symbols: news.matches.iter().map(|(code, _)| code.clone()).collect(),
            rule_kind: news.route.kind.into(),
            severity: if news.route.popup { "high" } else { "record" }.into(),
        })
        .collect::<Vec<_>>();
    let summaries = crate::agent::summarize_news(db, &agent_items)
        .await
        .unwrap_or_else(|error| {
            log::warn!("[news] Agent 摘要降级为原文：{error}");
            Vec::new()
        })
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<HashMap<_, _>>();
    let Some(current_policy) = policy(db) else {
        return;
    };
    if !current_policy.is_trading_day_at(Utc::now()) {
        return;
    }
    for news in prepared {
        let names = news
            .matches
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        let summary = summaries.get(&news.key);
        let body = summary
            .map(|item| format!("{}（依据：{}）", item.summary, item.evidence))
            .unwrap_or(news.fallback_body);
        let payload = serde_json::json!({
            "signal_id": format!("news:{}", stable_hash(&news.key)),
            "signal_tag": news.route.kind,
            "signal_kind": "news",
            "occurred_at": Utc::now().to_rfc3339(),
            "title": format!("资讯 · {} · {}", news.route.kind, names),
            "body": body,
            "alert_type": "news",
            "news_source": news.item.source_label,
            "news_source_id": news.item.source_id,
            "severity": if news.route.popup { "high" } else { "record" },
            "symbols": news.matches.iter().map(|(code, _)| code).collect::<Vec<_>>(),
            "agent_summary": summary.is_some(),
            "sentiment": summary.map(|item| item.sentiment.as_str()),
            "confidence": summary.map(|item| item.confidence),
        });
        if news.route.popup {
            crate::notifications::publish(app, payload);
        } else {
            crate::notifications::record_only(app, payload);
        }
    }
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
                if let Some(policy) = policy(&db) {
                    let now = Utc::now();
                    if let Some(stage) = timeline_stage_at(now, &policy) {
                        publish_timeline(&db, &app, stage);
                    }
                    if should_poll_at(now, &policy) {
                        poll_once(&db, &app).await;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(POLL_SECONDS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn whitelist_and_target_matching_reject_unrelated_news() {
        assert!(classify("董事会一般决议").is_none());
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
    }

    #[test]
    fn polling_only_runs_preopen_and_trading_sessions() {
        let policy = MarketRequestPolicy::default();
        let utc = |hour, minute| {
            Utc.with_ymd_and_hms(2026, 9, 18, hour, minute, 0)
                .single()
                .unwrap()
        };
        assert!(!should_poll_at(utc(0, 29), &policy)); // 北京 08:29
        assert!(should_poll_at(utc(0, 30), &policy)); // 北京 08:30
        assert!(!should_poll_at(utc(3, 30), &policy)); // 午休
        assert!(should_poll_at(utc(5, 0), &policy)); // 北京 13:00
        assert!(!should_poll_at(utc(7, 0), &policy)); // 收盘

        let closed = MarketRequestPolicy::from_quote_schedule_json(Some(
            r#"{"closed_dates":["2026-09-18"]}"#,
        ))
        .unwrap();
        assert!(!should_poll_at(utc(1, 30), &closed));
        assert_eq!(timeline_stage_at(utc(1, 0), &closed), None);
        assert_eq!(
            timeline_stage_at(utc(1, 0), &policy),
            Some(TimelineStage::PreOpen)
        );
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
