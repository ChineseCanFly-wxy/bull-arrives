//! Daily StockDB updates use China time and durable completion records.
use crate::db::Database;
use chrono::{DateTime, FixedOffset, Timelike, Utc};
use serde::{Deserialize, Serialize};

pub const RECORD_KEY: &str = "local_history_update_record";
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateRecord {
    pub last_success_day: Option<String>,
    pub last_success_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_attempt_day: Option<String>,
    pub last_finished_at: Option<String>,
    pub running_owner: Option<u32>,
    pub last_error: Option<String>,
    pub data_as_of: Option<String>,
    pub failures: u32,
    pub failure_notice_day: Option<String>,
}
#[derive(Clone, Debug)]
pub struct UpdateConfig {
    pub enabled: bool,
    pub stockdb_enabled: bool,
    pub time: String,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub enabled: bool,
    pub time: String,
    pub timezone: String,
    pub state: String,
    pub message: String,
    pub last_success_day: Option<String>,
    pub last_success_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_error: Option<String>,
    pub data_as_of: Option<String>,
    pub failures: u32,
}
pub fn validate_time(value: &str) -> Result<u32, String> {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
    {
        return Err("自动更新时间须为HH:mm".into());
    }
    let hour = value[..2].parse::<u32>().map_err(|_| "自动更新时间无效")?;
    let minute = value[3..].parse::<u32>().map_err(|_| "自动更新时间无效")?;
    if hour > 23 || minute > 59 {
        return Err("自动更新时间须在00:00—23:59之间".into());
    }
    Ok(hour * 60 + minute)
}
pub fn day(now: DateTime<Utc>) -> String {
    now.with_timezone(&FixedOffset::east_opt(28800).unwrap())
        .date_naive()
        .to_string()
}
pub fn config(db: &Database) -> Result<UpdateConfig, String> {
    Ok(UpdateConfig {
        enabled: db
            .get_setting("local_history_auto_update_enabled")
            .map_err(|e| e.to_string())?
            .as_deref()
            != Some("0"),
        stockdb_enabled: db
            .get_setting("local_history_enabled")
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("1"),
        time: db
            .get_setting("local_history_auto_update_time")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "09:00".into()),
    })
}
pub fn owner_alive(owner: u32) -> bool {
    if owner == std::process::id() {
        return true;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, owner) {
            let mut code = 0;
            let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == 259;
            let _ = CloseHandle(handle);
            return alive;
        }
    }
    false
}
pub fn due(
    config: &UpdateConfig,
    record: &UpdateRecord,
    now: DateTime<Utc>,
    _startup: bool,
    owner_is_alive: bool,
) -> Result<&'static str, String> {
    if !config.enabled {
        return Ok("paused");
    }
    if !config.stockdb_enabled {
        return Ok("stockdb_disabled");
    }
    let target = validate_time(&config.time)?;
    let local = now.with_timezone(&FixedOffset::east_opt(28800).unwrap());
    match crate::datasource::trading_calendar::is_trading_day_at(now, local.date_naive()) {
        Ok(true) => {},
        Ok(false) => return Ok("closed"),
        Err(_) => return Ok("waiting_calendar"),
    }
    if local.hour() * 60 + local.minute() < target {
        return Ok("waiting_time");
    }
    if record.last_success_day.as_deref() == Some(day(now).as_str()) {
        return Ok("complete");
    }
    if record.running_owner.is_some() && owner_is_alive {
        return Ok("running");
    }
    // A durable attempt consumes today's automatic update, even after a crash.
    // Manual updates can still be explicitly claimed below.
    if record.last_attempt_day.as_deref() == Some(day(now).as_str()) {
        return Ok("failed");
    }
    Ok("due")
}
pub fn status(db: &Database, now: DateTime<Utc>) -> Result<UpdateStatus, String> {
    let cfg = config(db)?;
    let record = db.stockdb_update_record()?;
    let state = due(
        &cfg,
        &record,
        now,
        false,
        record.running_owner.is_some_and(owner_alive),
    )?;
    let message = match state {
        "paused" => "自动更新已暂停",
        "stockdb_disabled" => "本地历史服务关闭，自动更新暂停",
        "waiting_time" => "等待交易日更新时间",
        "closed" => "A股休市，今天不执行自动更新",
        "waiting_calendar" => "交易日历证据不足，取得证据后再自动更新",
        "complete" => "今天已更新，重启应用不会重复执行",
        "running" => "StockDB正在自动更新",
        "failed" => "今天已执行一次，失败或中断后不自动重试；可手动更新或等待下一交易日",
        _ => "已到更新时间，程序将自动更新",
    };
    Ok(UpdateStatus {
        enabled: cfg.enabled,
        time: cfg.time,
        timezone: "Asia/Shanghai".into(),
        state: state.into(),
        message: message.into(),
        last_success_day: record.last_success_day,
        last_success_at: record.last_success_at,
        last_attempt_at: record.last_attempt_at,
        last_error: record.last_error,
        data_as_of: record.data_as_of,
        failures: record.failures,
    })
}

/// Claim in the settings transaction so two app instances cannot start updates together.
pub fn claim(
    db: &Database,
    cfg: &UpdateConfig,
    now: DateTime<Utc>,
    startup: bool,
    manual: bool,
) -> Result<bool, String> {
    db.change_stockdb_update_record(|record| {
        if record.running_owner.is_some_and(owner_alive) {
            return if manual {
                Err("StockDB更新正在另一任务中执行".into())
            } else {
                Ok(false)
            };
        }
        if record.last_attempt_day.as_deref() != Some(day(now).as_str()) {
            record.failures = 0;
        } else if record.running_owner.is_some() {
            record.failures = record.failures.saturating_add(1);
            record.last_error = Some("上次更新异常中断，今天不自动重试；可手动更新或等待下一交易日".into());
        }
        record.running_owner = None;
        if !manual && due(cfg, record, now, startup, false)? != "due" {
            return Ok(false);
        }
        record.running_owner = Some(std::process::id());
        record.last_attempt_day = Some(day(now));
        record.last_attempt_at = Some(now.to_rfc3339());
        Ok(true)
    })
}
pub fn finish(
    db: &Database,
    attempt: DateTime<Utc>,
    completed: DateTime<Utc>,
    result: Result<Option<String>, String>,
) -> Result<(), String> {
    db.change_stockdb_update_record(|record| {
        if record.running_owner != Some(std::process::id())
            || record.last_attempt_at.as_deref() != Some(attempt.to_rfc3339().as_str())
        {
            return Err("StockDB更新记录所有者已改变".into());
        }
        record.running_owner = None;
        record.last_finished_at = Some(completed.to_rfc3339());
        match result {
            Ok(as_of) => {
                record.last_success_day = Some(day(attempt));
                record.last_success_at = Some(completed.to_rfc3339());
                record.last_error = None;
                record.failures = 0;
                record.data_as_of = as_of;
            }
            Err(error) => {
                record.failures = record.failures.saturating_add(1);
                record.last_error = Some(error);
            }
        }
        Ok(())
    })
}
pub fn claim_failure_notice(db: &Database, now: DateTime<Utc>) -> Result<Option<String>, String> {
    db.change_stockdb_update_record(|record| {
        let today = day(now);
        if record.last_attempt_day.as_deref() != Some(&today)
            || record.failures == 0
            || record.failure_notice_day.as_deref() == Some(&today)
        {
            return Ok(None);
        }
        record.failure_notice_day = Some(today);
        Ok(Some(
            record
                .last_error
                .clone()
                .unwrap_or_else(|| "未知更新错误".into()),
        ))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 29, hour - 8, minute, 0)
            .unwrap()
    }
    fn cfg() -> UpdateConfig {
        UpdateConfig {
            enabled: true,
            stockdb_enabled: true,
            time: "09:00".into(),
        }
    }
    #[test]
    fn clock_is_china_time_and_accepts_only_valid_times() {
        assert_eq!(
            due(&cfg(), &UpdateRecord::default(), at(8, 59), false, false).unwrap(),
            "waiting_time"
        );
        assert_eq!(
            due(&cfg(), &UpdateRecord::default(), at(9, 0), false, false).unwrap(),
            "due"
        );
        for value in ["9:00", "24:00", "09:60", "ab:cd", "09:00:00"] {
            assert!(validate_time(value).is_err());
        }
        assert_eq!(validate_time("23:59").unwrap(), 1439);
    }
    #[test]
    fn each_day_runs_once_even_after_failure_or_interruption() {
        let mut record = UpdateRecord {
            last_success_day: Some("2026-09-29".into()),
            ..Default::default()
        };
        assert_eq!(
            due(&cfg(), &record, at(19, 0), true, false).unwrap(),
            "complete"
        );
        record.last_success_day = None;
        record.last_attempt_day = Some("2026-09-29".into());
        record.last_attempt_at = Some(at(9, 0).to_rfc3339());
        record.last_error = Some("网络暂不可用".into());
        assert_eq!(
            due(&cfg(), &record, at(9, 0), false, false).unwrap(),
            "failed"
        );
        assert_eq!(due(&cfg(), &record, at(9, 1), false, false).unwrap(), "failed");
        assert_eq!(due(&cfg(), &record, at(19, 0), true, false).unwrap(), "failed");
        record.running_owner = Some(1);
        assert_eq!(
            due(&cfg(), &record, at(10, 0), true, true).unwrap(),
            "running"
        );
        assert_eq!(due(&cfg(), &record, at(10, 0), true, false).unwrap(), "failed");
        let tomorrow = at(9, 0) + chrono::Duration::days(1);
        assert_eq!(due(&cfg(), &record, tomorrow, true, false).unwrap(), "due");
    }
    #[test]
    fn failure_blocks_automatic_restarts_but_allows_manual_updates_and_next_day() {
        let root =
            std::env::temp_dir().join(format!("bull-stockdb-clock-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let other = Database::open(root.clone()).unwrap();
        let first = at(9, 0);
        assert!(claim(&db, &cfg(), first, true, false).unwrap());
        assert!(!claim(&other, &cfg(), first, true, false).unwrap());
        finish(&db, first, at(9, 2), Err("断网".into())).unwrap();
        assert_eq!(
            due(
                &cfg(),
                &db.stockdb_update_record().unwrap(),
                at(9, 2),
                true,
                false
            )
            .unwrap(),
            "failed"
        );
        for minute in 3..=6 {
            assert!(!claim(&other, &cfg(), at(9, minute), false, false).unwrap());
        }
        assert_eq!(db.stockdb_update_record().unwrap().failures, 1);
        assert!(!claim(&db, &cfg(), at(9, 8), true, false).unwrap());
        assert!(claim_failure_notice(&db, at(9, 8)).unwrap().is_some());
        drop(other);
        let reopened = Database::open(root.clone()).unwrap();
        assert!(!claim(&reopened, &cfg(), at(9, 9), true, false).unwrap());
        assert!(claim_failure_notice(&reopened, at(9, 9)).unwrap().is_none());
        let retry = at(9, 10);
        assert!(claim(&reopened, &cfg(), retry, false, true).unwrap());
        finish(&reopened, retry, retry, Err("手动更新失败".into())).unwrap();
        assert!(!claim(&db, &cfg(), at(9, 11), false, false).unwrap());
        let manual = at(9, 12);
        assert!(claim(&reopened, &cfg(), manual, false, true).unwrap());
        finish(&reopened, manual, manual, Ok(Some("2026-09-30".into()))).unwrap();
        assert!(!claim(&db, &cfg(), at(19, 0), true, false).unwrap());
        assert_eq!(db.stockdb_update_record().unwrap().failures, 0);
        let tomorrow = at(9, 0) + chrono::Duration::days(1);
        assert!(claim(&db, &cfg(), tomorrow, true, false).unwrap());
        assert_eq!(db.stockdb_update_record().unwrap().failures, 0);
        drop(db);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_update_is_not_relaunched_after_restart() {
        let root = std::env::temp_dir().join(format!(
            "bull-stockdb-interrupted-{}",
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(root.clone()).unwrap();
        db.change_stockdb_update_record(|record| {
            record.last_attempt_day = Some(day(at(9, 0)));
            record.last_attempt_at = Some(at(9, 0).to_rfc3339());
            record.running_owner = Some(u32::MAX);
            Ok(())
        })
        .unwrap();
        assert!(!claim(&db, &cfg(), at(9, 2), true, false).unwrap());
        let record = db.stockdb_update_record().unwrap();
        assert!(record.running_owner.is_none());
        assert_eq!(record.failures, 1);
        assert!(record.last_error.unwrap().contains("不自动重试"));
        assert!(claim_failure_notice(&db, at(9, 2)).unwrap().is_some());
        assert!(!claim(&db, &cfg(), at(19, 0), true, false).unwrap());
        assert_eq!(db.stockdb_update_record().unwrap().failures, 1);
        let tomorrow = at(9, 0) + chrono::Duration::days(1);
        assert!(claim(&db, &cfg(), tomorrow, true, false).unwrap());
        assert_eq!(db.stockdb_update_record().unwrap().failures, 0);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn holidays_and_unknown_calendar_do_not_claim_automatic_updates() {
        let root = std::env::temp_dir().join(format!("bull-stockdb-calendar-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let holiday = Utc.with_ymd_and_hms(2026, 10, 5, 1, 0, 0).unwrap();
        assert_eq!(due(&cfg(), &UpdateRecord::default(), holiday, true, false).unwrap(), "closed");
        assert!(!claim(&db, &cfg(), holiday, true, false).unwrap());
        assert!(db.stockdb_update_record().unwrap().last_attempt_at.is_none());
        let unknown = Utc.with_ymd_and_hms(2099, 1, 5, 1, 0, 0).unwrap();
        assert_eq!(due(&cfg(), &UpdateRecord::default(), unknown, true, false).unwrap(), "waiting_calendar");
        assert!(!claim(&db, &cfg(), unknown, true, false).unwrap());
        assert!(db.stockdb_update_record().unwrap().running_owner.is_none());
        drop(db); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disabled_stockdb_never_schedules_external_work() {
        let mut config = cfg();
        config.stockdb_enabled = false;
        assert_eq!(
            due(&config, &UpdateRecord::default(), at(19, 0), true, false).unwrap(),
            "stockdb_disabled"
        );
        config.stockdb_enabled = true;
        config.enabled = false;
        assert_eq!(
            due(&config, &UpdateRecord::default(), at(19, 0), true, false).unwrap(),
            "paused"
        );
    }
}
