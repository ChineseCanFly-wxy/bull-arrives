use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{sync_channel, SyncSender, TrySendError},
        Mutex,
    },
};
use tauri::{Emitter, Manager, State};
#[cfg(not(target_os = "windows"))]
use tauri_plugin_notification::NotificationExt;

const HISTORY_LIMIT: usize = 50;
const DELIVERY_QUEUE_LIMIT: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryStatus {
    pub id: String,
    pub history_version: u64,
    pub native: String,
    pub native_error: Option<String>,
    pub desktop: String,
    pub desktop_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationHistorySnapshot {
    pub version: u64,
    pub entries: Vec<Value>,
}

struct HistoryInner {
    started_at: i64,
    version: u64,
    entries: VecDeque<Value>,
}
impl Default for HistoryInner {
    fn default() -> Self {
        Self {
            started_at: chrono::Utc::now().timestamp_millis(),
            version: 0,
            entries: VecDeque::new(),
        }
    }
}
#[derive(Default)]
pub struct NotificationHistory(Mutex<HistoryInner>);

struct DeliveryJob {
    id: String,
    title: String,
    body: String,
    history_version: u64,
    force_desktop: bool,
    important_delivered: bool,
    response: Option<tokio::sync::oneshot::Sender<DeliveryStatus>>,
}

pub struct NotificationDelivery {
    sender: SyncSender<DeliveryJob>,
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn setting_enabled(app: &tauri::AppHandle, key: &str, default: bool) -> bool {
    app.try_state::<std::sync::Arc<crate::db::Database>>()
        .and_then(|db| db.get_setting(key).ok().flatten())
        .map(|value| value == "1")
        .unwrap_or(default)
}

#[cfg(target_os = "windows")]
fn send_native(title: &str, body: &str) -> Result<(), String> {
    notify_rust::Notification::new()
        .app_id(crate::notification_identity::APP_USER_MODEL_ID)
        .summary(title)
        .body(body)
        .show()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "windows"))]
fn send_native_with_app(app: &tauri::AppHandle, title: &str, body: &str) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

fn execute_job(app: &tauri::AppHandle, job: &DeliveryJob) -> DeliveryStatus {
    #[cfg(target_os = "windows")]
    let native_result = send_native(&job.title, &job.body);
    #[cfg(not(target_os = "windows"))]
    let native_result = send_native_with_app(app, &job.title, &job.body);

    let desktop_needed = !job.important_delivered && (job.force_desktop || native_result.is_err());
    let desktop_result = if desktop_needed {
        crate::desktop_toast::enqueue(
            app,
            crate::desktop_toast::DesktopToastPayload {
                id: job.id.clone(),
                title: job.title.clone(),
                body: job.body.clone(),
            },
        )
    } else {
        Ok(())
    };

    DeliveryStatus {
        id: job.id.clone(),
        history_version: job.history_version,
        native: if native_result.is_ok() {
            "accepted"
        } else {
            "failed"
        }
        .into(),
        native_error: native_result.err(),
        desktop: if job.important_delivered {
            "queued"
        } else if !desktop_needed {
            "not-requested"
        } else if desktop_result.is_ok() {
            "queued"
        } else {
            "failed"
        }
        .into(),
        desktop_error: desktop_result.err(),
    }
}

impl NotificationDelivery {
    pub fn new(app: tauri::AppHandle) -> Self {
        let (sender, receiver) = sync_channel::<DeliveryJob>(DELIVERY_QUEUE_LIMIT);
        std::thread::Builder::new()
            .name("notification-delivery".into())
            .spawn(move || {
                while let Ok(mut job) = receiver.recv() {
                    let status = execute_job(&app, &job);
                    update_delivery_status(&app, &status);
                    if let Some(response) = job.response.take() {
                        let _ = response.send(status);
                    }
                }
            })
            .expect("failed to start notification delivery worker");
        Self { sender }
    }
}

fn update_delivery_status(app: &tauri::AppHandle, status: &DeliveryStatus) {
    let history = app.state::<NotificationHistory>();
    let mut emitted = false;
    {
        let mut inner = history.0.lock().unwrap_or_else(|e| e.into_inner());
        // Completion never inserts. Version + id prevent pre-clear work from
        // resurrecting a record after the clear watermark advances.
        if inner.version == status.history_version {
            if let Some(entry) = inner
                .entries
                .iter_mut()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(&status.id))
            {
                entry["delivery"] = serde_json::to_value(status).unwrap_or(Value::Null);
                emitted = true;
            }
        }
    }
    if emitted {
        let _ = app.emit("notification-delivery-status", status);
    }
    if let Some(error) = status.native_error.as_ref() {
        log::warn!("原生通知发送失败: {error}");
    }
    if let Some(error) = status.desktop_error.as_ref() {
        log::warn!("桌面提醒发送失败: {error}");
    }
}

fn queue_failure(id: String, history_version: u64, message: &str) -> DeliveryStatus {
    DeliveryStatus {
        id,
        history_version,
        native: "failed".into(),
        native_error: Some(message.into()),
        desktop: "failed".into(),
        desktop_error: Some(message.into()),
    }
}

fn record_history(app: &tauri::AppHandle, payload: &mut Value, pending: bool) -> (String, u64) {
    let id = format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    payload["id"] = Value::String(id.clone());
    stamp_notification(payload,chrono::Utc::now().timestamp_millis());
    let version = {
        let history = app.state::<NotificationHistory>();
        let mut inner = history.0.lock().unwrap_or_else(|e| e.into_inner());
        payload["history_version"] = Value::Number(inner.version.into());
        payload["delivery"] = if pending {
            serde_json::json!({ "id": id, "history_version": inner.version, "native": "pending", "desktop": "pending" })
        } else {
            serde_json::json!({ "id": id, "history_version": inner.version, "native": "not-requested", "desktop": "not-requested" })
        };
        inner.entries.push_front(payload.clone());
        inner.entries.truncate(HISTORY_LIMIT);
        inner.version
    };
    archive_news_payload(app, payload);
    let _ = app.emit("price-alert-triggered", &payload);
    (id, version)
}
fn stamp_notification(payload:&mut Value,now:i64){
    payload["delivered_at"]=Value::Number(now.into());
    if payload.get("received_at").is_none_or(Value::is_null){payload["received_at"]=Value::Number(now.into());}
}

fn archive_news_payload(app: &tauri::AppHandle, payload: &Value) {
    if let Some(db) = app.try_state::<std::sync::Arc<crate::db::Database>>() {
        let result = match payload["signal_kind"].as_str() {
            Some("news") => db.archive_news(payload),
            Some("price" | "risk" | "research" | "system") => db.archive_brief_alert(payload),
            _ => Ok(()),
        };
        if let Err(error) = result {
            log::warn!("[news] archive: {error}");
        }
    }
}

fn is_model_notice(payload: &Value) -> bool {
    payload["signal_kind"] == "research"
        && (payload.get("model_snapshot").is_some_and(Value::is_object)
            || payload.get("intraday_snapshot").is_some_and(Value::is_object)
            || payload.get("condition_event").is_some_and(Value::is_object))
}

pub fn notice_category(payload: &Value) -> &'static str {
    if payload["signal_kind"] == "news" { return "news"; }
    if payload["stockdb_update_alert"]["schema"] == "stockdb-update-failed-v1" { return "data"; }
    if payload["signal_kind"] == "research" {
        if payload["sector_code"].as_str().is_some_and(|s| !s.is_empty()) { return "mainline"; }
        if payload["model_snapshot"]["follow_account_id"].is_number() { return "trades"; }
        if payload["condition_event"].is_object() || payload["condition_events"].as_array().is_some_and(|rows| !rows.is_empty()) { return "conditions"; }
        if payload["intraday_snapshot"].is_object() { return "intraday"; }
        return "research";
    }
    if payload["signal_kind"] == "risk" { "risk" } else { "price" }
}
fn notification_setting_key(category: &str) -> &'static str {
    match category {
        "news" => "news_notifications_enabled",
        "mainline" => "mainline_notifications_enabled",
        "trades" => "model_trade_notifications_enabled",
        "conditions" => "model_condition_notifications_enabled",
        "intraday" => "intraday_notifications_enabled",
        "research" => "research_notifications_enabled",
        "risk" => "risk_notifications_enabled",
        "data" => "data_notifications_enabled",
        _ => "alerts_enabled",
    }
}
fn is_important(category: &str) -> bool {
    matches!(category, "mainline" | "trades" | "conditions" | "intraday" | "price" | "risk")
}
fn delivery_policy(payload: &Value, enabled: bool, desktop_always: bool) -> Option<bool> {
    enabled.then(|| is_model_notice(payload) || notice_category(payload) == "data" || desktop_always)
}

fn news_after_start(payload: &Value, started_at: i64, now: i64) -> bool {
    payload["published_at"].as_str()
        .and_then(|value|chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|published|published.timestamp_millis()>started_at && published.timestamp_millis()<=now)
}
fn current_news(app: &tauri::AppHandle, payload: &Value) -> bool {
    let history=app.state::<NotificationHistory>();
    let started_at=history.0.lock().unwrap_or_else(|e|e.into_inner()).started_at;
    news_after_start(payload,started_at,chrono::Utc::now().timestamp_millis())
}
pub fn publish(app: &tauri::AppHandle, mut payload: Value) {
    // 关机期间的积压和时间未核实的原文只归档，不进入本次提醒或通知队列。
    if payload["signal_kind"]=="news" && !current_news(app,&payload) {
        archive_news_payload(app,&payload); return;
    }
    let category = notice_category(&payload);
    payload["notification_category"] = category.into();
    let Some(mut force_desktop) = delivery_policy(&payload,
        setting_enabled(app, notification_setting_key(category), category != "news"),
        setting_enabled(app, "notification_desktop_always", false)) else {
        record_history(app, &mut payload, false);
        return;
    };
    let title = payload
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("行情提醒")
        .to_string();
    let body = payload
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("股票已达到提醒条件")
        .to_string();
    let (id, version) = record_history(app, &mut payload, true);
    let mut important_delivered = false;
    if is_important(category) && setting_enabled(app, "important_alerts_enabled", true) {
        match crate::important_alerts::enqueue(app, payload.clone()) {
            Ok(()) => important_delivered = true,
            Err(error) => { force_desktop = true; log::warn!("重要悬浮提醒失败，转用系统及桌面通知：{error}"); },
        }
        if important_delivered && !setting_enabled(app, "important_alerts_native_enabled", false) {
            update_delivery_status(app, &DeliveryStatus { id, history_version: version, native: "not-requested".into(), native_error: None, desktop: "queued".into(), desktop_error: None });
            return;
        }
    }
    if category == "news" && !setting_enabled(app, "news_system_notifications_enabled", true) {
        update_delivery_status(app, &DeliveryStatus { id, history_version: version, native: "not-requested".into(), native_error: None, desktop: "not-requested".into(), desktop_error: None });
        return;
    }
    let job = DeliveryJob {
        id: id.clone(),
        title,
        body,
        history_version: version,
        force_desktop,
        important_delivered,
        response: None,
    };
    if let Err(error) = app.state::<NotificationDelivery>().sender.try_send(job) {
        let message = match error {
            TrySendError::Full(_) => "通知投递队列已满",
            TrySendError::Disconnected(_) => "通知投递服务不可用",
        };
        // The bounded worker cannot accept this item. Record the explicit failure;
        // never block the market scheduler or silently report success.
        let mut status = queue_failure(id, version, message);
        if important_delivered { status.desktop = "queued".into(); status.desktop_error = None; }
        update_delivery_status(app, &status);
    }
}

/// Replace one archived news entry in the current session and optionally push its AI interpretation.
pub fn news_analysis_updated(app: &tauri::AppHandle, payload: &Value, push: bool) {
    let signal_id = payload["signal_id"].as_str();
    let mut updated = payload.clone();
    let mut delivery_target = None;
    {
        let history = app.state::<NotificationHistory>();
        let mut inner = history.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = inner.entries.iter_mut().find(|entry| entry["signal_id"].as_str() == signal_id) {
            for key in ["id", "received_at", "delivered_at", "history_version", "delivery"] {
                updated[key] = entry[key].clone();
            }
            if let (Some(id), Some(version)) = (updated["id"].as_str(), updated["history_version"].as_u64()) {
                delivery_target = Some((id.to_owned(), version));
            }
            *entry = updated.clone();
        }
    }
    let _ = app.emit("news-analysis-updated", &updated);
    if push && current_news(app,&updated) {
        let (id, history_version) = delivery_target.unwrap_or_else(|| (
            updated["signal_id"].as_str().unwrap_or("news-analysis").to_string(), u64::MAX,
        ));
        let job = DeliveryJob {
            id, history_version,
            title: format!("AI 解读 · {}", updated["title"].as_str().unwrap_or("资讯")),
            body: updated["body"].as_str().unwrap_or_default().to_string(),
            force_desktop: setting_enabled(app, "notification_desktop_always", false),
            important_delivered: false,
            response: None,
        };
        if let Err(error) = app.state::<NotificationDelivery>().sender.try_send(job) {
            log::warn!("[news] AI 解读通知排队失败：{error}");
        }
    }
}

#[tauri::command]
pub async fn test_notification(app: tauri::AppHandle, research: Option<bool>, category: Option<String>) -> Result<DeliveryStatus, String> {
    let id = format!(
        "test-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    if let Some(category) = category.as_deref() {
        if !matches!(category, "news" | "mainline" | "trades" | "conditions" | "intraday" | "research" | "price" | "risk" | "data" | "floating") { return Err("提醒类别无效".into()); }
        if (is_important(category) || category == "floating") && setting_enabled(&app, "important_alerts_enabled", true) {
            crate::important_alerts::enqueue(&app, serde_json::json!({"id":id,"notification_category":category,"title":"重要操作提醒测试","body":"这是测试消息，不代表买入、卖出或成交。拖动标题栏移动窗口，拖动右下角调整大小；确认已读后移除此条。","received_at":chrono::Utc::now().timestamp_millis()}))?;
            return Ok(DeliveryStatus { id, history_version: u64::MAX, native: "not-requested".into(), native_error: None, desktop: "queued".into(), desktop_error: None });
        }
    }
    let (response, receiver) = tokio::sync::oneshot::channel();
    let job = DeliveryJob {
        id: id.clone(),
        title: if research == Some(true) { "研究中心提醒测试" } else { "Bull Arrives 通知测试" }.into(),
        body: if research == Some(true) { "研究提醒会显示模型观察及模拟买卖结果；本次仅测试通知，不创建委托或调整账户。" } else { "收到此通知说明系统已受理通知；是否显示横幅仍受勿扰和系统策略影响。" }.into(),
        history_version: u64::MAX,
        // Test both channels, including the desktop channel required by model notices.
        force_desktop: true,
        important_delivered: false,
        response: Some(response),
    };
    app.state::<NotificationDelivery>()
        .sender
        .try_send(job)
        .map_err(|error| match error {
            TrySendError::Full(_) => String::from("通知投递队列已满"),
            TrySendError::Disconnected(_) => String::from("通知投递服务不可用"),
        })?;
    tokio::time::timeout(std::time::Duration::from_secs(8), receiver)
        .await
        .map_err(|_| "通知测试超时".to_string())?
        .map_err(|_| "通知投递服务未返回结果".to_string())
}

#[tauri::command]
pub fn get_alert_archive(app: tauri::AppHandle, db: State<'_, std::sync::Arc<crate::db::Database>>) -> Result<Vec<Value>, String> {
    let now = chrono::Utc::now();
    let mut rows: Vec<Value> = db.brief_alerts_between(&(now-chrono::Duration::days(14)).to_rfc3339(), &now.to_rfc3339())?.into_iter().take(500).collect();
    for pending in crate::important_alerts::get_important_alerts(app).entries {
        if !pending["id"].as_str().is_some_and(|id| id.starts_with("test-")) && !rows.iter().any(|row| row["id"] == pending["id"]) { rows.push(pending); }
    }
    Ok(rows)
}
#[tauri::command]
pub fn get_notification_history(
    history: State<'_, NotificationHistory>,
) -> NotificationHistorySnapshot {
    let inner = history.0.lock().unwrap_or_else(|e| e.into_inner());
    NotificationHistorySnapshot {
        version: inner.version,
        entries: inner.entries.iter().cloned().collect(),
    }
}

#[tauri::command]
pub fn clear_notification_history(
    app: tauri::AppHandle,
    history: State<'_, NotificationHistory>,
) -> Result<u64, String> {
    let version = {
        let mut inner = history.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.entries.clear();
        inner.version = inner.version.wrapping_add(1);
        inner.version
    };
    app.emit("notification-history-cleared", version)
        .map_err(|e| format!("广播提醒记录清空失败: {e}"))?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_notification_boundary_uses_publication_not_receipt_and_resets_each_start() {
        let start=chrono::DateTime::parse_from_rfc3339("2026-10-07T01:00:00Z").unwrap().timestamp_millis();
        let now=start+120_000;
        let payload=|published: &str|serde_json::json!({"published_at":published,"received_at":now});
        assert!(!news_after_start(&payload("2026-10-07T08:59:59+08:00"),start,now));
        assert!(!news_after_start(&payload("2026-10-07T09:00:00+08:00"),start,now));
        let fresh=payload("2026-10-07T09:00:01+08:00");
        assert!(news_after_start(&fresh,start,now));
        assert!(!news_after_start(&fresh,now,now+120_000),"重启不补发旧资讯");
        assert!(!news_after_start(&payload("2026-10-07T09:03:00+08:00"),start,now));
        assert!(!news_after_start(&payload("not-a-time"),start,now));
        assert!(!news_after_start(&serde_json::json!({"published_date":"2026-10-07","received_at":now}),start,now));
    }
    #[test]
    fn model_popups_only_apply_to_model_and_intraday_observations() {
        assert!(is_model_notice(&serde_json::json!({"signal_kind":"research","model_snapshot":{}})));
        assert!(is_model_notice(&serde_json::json!({"signal_kind":"research","intraday_snapshot":{}})));
        assert!(is_model_notice(&serde_json::json!({"signal_kind":"research","condition_event":{"schema":"model-condition-event-v1"}})));
        assert!(!is_model_notice(&serde_json::json!({"signal_kind":"news","model_snapshot":{}})));
        assert!(!is_model_notice(&serde_json::json!({"signal_kind":"research","model_snapshot":null})));
        assert!(!is_model_notice(&serde_json::json!({"signal_kind":"research"})));
    }

    #[test]
    fn research_delivery_has_its_own_switch_and_requests_visible_desktop_channel() {
        let model = serde_json::json!({"signal_kind":"research","model_snapshot":{}});
        assert_eq!(delivery_policy(&model, false, true), None);
        assert_eq!(delivery_policy(&model, false, false), None);
        assert_eq!(delivery_policy(&model, true, false), Some(true));
        assert_eq!(delivery_policy(&model, true, true), Some(true));
        let other_research = serde_json::json!({"signal_kind":"research"});
        assert_eq!(delivery_policy(&other_research, false, true), None);
        assert_eq!(delivery_policy(&other_research, true, false), Some(false));
        let data_error=serde_json::json!({"signal_kind":"system","stockdb_update_alert":{"schema":"stockdb-update-failed-v1"}});
        assert_eq!(delivery_policy(&data_error,false,false),None);
        assert_eq!(delivery_policy(&data_error,true,false),Some(true));
        let news = serde_json::json!({"signal_kind":"news"});
        assert_eq!(delivery_policy(&news, false, false), None);
        assert_eq!(delivery_policy(&news, true, true), Some(true));
    }

    #[test]
    fn category_switches_and_important_channels_do_not_mix_news_with_mainline_or_trades() {
        let cases = [
            (serde_json::json!({"signal_kind":"news","sector_code":"BK0001"}), "news", false),
            (serde_json::json!({"signal_kind":"research","sector_code":"BK0001"}), "mainline", true),
            (serde_json::json!({"signal_kind":"research","model_snapshot":{"follow_account_id":1}}), "trades", true),
            (serde_json::json!({"signal_kind":"research","condition_events":[{}]}), "conditions", true),
            (serde_json::json!({"signal_kind":"research","intraday_snapshot":{}}), "intraday", true),
            (serde_json::json!({"signal_kind":"research","model_snapshot":{}}), "research", false),
            (serde_json::json!({"signal_kind":"price"}), "price", true),
            (serde_json::json!({"signal_kind":"risk"}), "risk", true),
        ];
        for (payload, category, important) in cases {
            assert_eq!(notice_category(&payload), category);
            assert_eq!(is_important(category), important);
            assert_eq!(delivery_policy(&payload, false, true), None);
        }
        assert_ne!(notification_setting_key("mainline"), notification_setting_key("research"));
        assert_ne!(notification_setting_key("trades"), notification_setting_key("conditions"));
        assert_ne!(notification_setting_key("price"), notification_setting_key("risk"));
    }
    #[test]
    fn clear_watermark_rejects_old_delivery() {
        let mut inner = HistoryInner::default();
        inner.entries.push_back(serde_json::json!({"id": "old"}));
        let old = inner.version;
        inner.entries.clear();
        inner.version += 1;
        assert_ne!(old, inner.version);
        assert!(inner.entries.is_empty());
    }
    #[test]
    fn delivery_keeps_source_receipt_and_research_identity(){
        let mut payload=serde_json::json!({"received_at":"2026-09-30T07:20:00Z","signal_kind":"research","sector_code":"SW801080","fingerprint":"frozen"});
        stamp_notification(&mut payload,42);assert_eq!(payload["received_at"],"2026-09-30T07:20:00Z");assert_eq!(payload["delivered_at"],42);
        assert_eq!(payload["sector_code"],"SW801080");assert_eq!(payload["fingerprint"],"frozen");
        let mut empty=serde_json::json!({});stamp_notification(&mut empty,43);assert_eq!(empty["received_at"],43);
    }

    #[test]
    fn history_is_limited_to_fifty() {
        let mut inner = HistoryInner::default();
        for id in 0..60 {
            inner.entries.push_front(serde_json::json!({"id": id}));
            inner.entries.truncate(HISTORY_LIMIT);
        }
        assert_eq!(inner.entries.len(), 50);
    }
}
