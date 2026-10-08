//! 本次运行的自动操作记录：复用应用日志入口，不发送系统通知。
use log::{Level, LevelFilter, Log, Metadata, Record};
use serde::Serialize;
use simplelog::{Config, SharedLogger};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tauri::State;

const LIMIT: usize = 200;
pub const SETTING: &str = "automatic_operations_log_enabled";

#[derive(Clone, Serialize)]
pub struct Entry {
    id: u64,
    at: i64,
    level: String,
    source: String,
    message: String,
    repeats: u64,
}
#[derive(Serialize)]
pub struct Snapshot {
    enabled: bool,
    version: u64,
    entries: Vec<Entry>,
}
struct Inner {
    enabled: bool,
    version: u64,
    next_id: u64,
    entries: VecDeque<Entry>,
}
pub struct OperationsLog(Mutex<Inner>);
impl Default for OperationsLog {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            enabled: true,
            version: 0,
            next_id: 0,
            entries: VecDeque::new(),
        }))
    }
}
impl OperationsLog {
    pub fn set_enabled(&self, enabled: bool) {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.enabled = enabled;
        inner.version += 1;
    }
    fn snapshot(&self) -> Snapshot {
        let inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            enabled: inner.enabled,
            version: inner.version,
            entries: inner.entries.iter().cloned().collect(),
        }
    }
    fn clear(&self) -> Snapshot {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.entries.clear();
        inner.version += 1;
        Snapshot {
            enabled: inner.enabled,
            version: inner.version,
            entries: Vec::new(),
        }
    }
    fn record(&self, source: &str, level: Level, message: &str) {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.enabled {
            return;
        }
        let message: String = message.chars().take(1000).collect();
        let level = match level {
            Level::Error => "失败",
            Level::Warn => "注意",
            _ => "正常",
        }
        .to_string();
        let now = chrono::Utc::now().timestamp_millis();
        inner.version += 1;
        if let Some(previous) = inner
            .entries
            .front_mut()
            .filter(|row| row.source == source && row.level == level && row.message == message)
        {
            previous.at = now;
            previous.repeats = previous.repeats.saturating_add(1);
            return;
        }
        inner.next_id += 1;
        let id = inner.next_id;
        inner.entries.push_front(Entry {
            id,
            at: now,
            level,
            source: source.into(),
            message,
            repeats: 1,
        });
        inner.entries.truncate(LIMIT);
    }
}
fn source(target: &str) -> Option<&'static str> {
    match target {
        "automation::data" => Some("数据更新"),
        "automation::model" => Some("自动模型"),
        "automation::mainline" => Some("市场主线"),
        "automation::research" => Some("自动研究"),
        _ => None,
    }
}
pub struct OperationsLogger(pub Arc<OperationsLog>);
impl Log for OperationsLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Info && source(metadata.target()).is_some()
    }
    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.record(
                source(record.target()).unwrap(),
                record.level(),
                &record.args().to_string(),
            );
        }
    }
    fn flush(&self) {}
}
impl SharedLogger for OperationsLogger {
    fn level(&self) -> LevelFilter {
        LevelFilter::Info
    }
    fn config(&self) -> Option<&Config> {
        None
    }
    fn as_log(self: Box<Self>) -> Box<dyn Log> {
        self
    }
}
#[tauri::command]
pub fn get_automatic_operations_log(log: State<'_, Arc<OperationsLog>>) -> Snapshot {
    log.snapshot()
}
#[tauri::command]
pub fn clear_automatic_operations_log(log: State<'_, Arc<OperationsLog>>) -> Snapshot {
    log.clear()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_log_only_records_task_execution() {
        let state = Arc::new(OperationsLog::default());
        let logger = OperationsLogger(state.clone());
        for target in ["automation::news", "automation::trading", "notifications", "network"] {
            logger.log(&Record::builder().args(format_args!("提醒不重复进入任务记录")).level(Level::Info).target(target).build());
        }
        assert!(state.snapshot().entries.is_empty());
        for target in ["automation::data", "automation::model", "automation::mainline", "automation::research"] {
            logger.log(&Record::builder().args(format_args!("后台任务已执行")).level(Level::Info).target(target).build());
        }
        assert_eq!(state.snapshot().entries.len(), 4);
    }
    #[test]
    fn automatic_log_is_independent_bounded_and_clear_does_not_disable_it() {
        let state = Arc::new(OperationsLog::default());
        let logger = OperationsLogger(state.clone());
        logger.log(
            &Record::builder()
                .args(format_args!("hidden unrelated message"))
                .level(Level::Info)
                .target("network")
                .build(),
        );
        assert!(state.snapshot().entries.is_empty());
        logger.log(&Record::builder().args(format_args!("资讯采集不进入自动操作")).level(Level::Info).target("automation::news").build());
        assert!(state.snapshot().entries.is_empty());
        state.set_enabled(false);
        state.record("自动模型", Level::Info, "should not appear");
        assert!(state.snapshot().entries.is_empty());
        state.set_enabled(true);
        state.record("自动模型", Level::Warn, "等候有效行情");
        state.record("自动模型", Level::Warn, "等候有效行情");
        assert_eq!(state.snapshot().entries.len(), 1);
        assert_eq!(state.snapshot().entries[0].repeats, 2);
        for index in 0..LIMIT + 1 {
            state.record("自动模型", Level::Info, &format!("检查 {index}"));
        }
        assert_eq!(state.snapshot().entries.len(), LIMIT);
        let before = state.snapshot();
        let cleared = state.clear();
        assert!(cleared.enabled && cleared.entries.is_empty());
        assert!(cleared.version > before.version);
        state.record("模拟交易", Level::Info, &"字".repeat(1200));
        assert_eq!(state.snapshot().entries[0].message.chars().count(), 1000);
        assert!(state.snapshot().entries[0].id > before.entries[0].id);
    }
}
