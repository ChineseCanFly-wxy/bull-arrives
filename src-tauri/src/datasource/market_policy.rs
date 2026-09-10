use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, Utc, Weekday};
use serde_json::Value;

const CST_OFFSET_SECONDS: i32 = 8 * 60 * 60;
const DEFAULT_SESSIONS: [(&str, &str); 2] = [("09:15", "11:30"), ("13:00", "15:00")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradingWindow {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketRequestPolicy {
    windows: Vec<TradingWindow>,
    closed_dates: Vec<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketGateDecision {
    Allowed,
    Paused { reason: String },
}

impl Default for MarketRequestPolicy {
    fn default() -> Self {
        Self {
            windows: DEFAULT_SESSIONS
                .iter()
                .map(|(start, end)| TradingWindow {
                    start: parse_time(start).expect("default start time is valid"),
                    end: parse_time(end).expect("default end time is valid"),
                })
                .collect(),
            closed_dates: Vec::new(),
        }
    }
}

impl MarketRequestPolicy {
    /// 配置损坏时停止请求，不能静默扩大允许获取行情的时间范围。
    pub fn paused() -> Self {
        Self { windows: Vec::new(), closed_dates: Vec::new() }
    }

    /// Parses the `settings.quote_schedule` JSON value. Empty input selects defaults.
    /// Supported shape:
    /// `{ "sessions": [{"start":"09:15","end":"11:30"}, ...],
    ///    "closed_dates": ["2026-10-01", ...] }`.
    /// A session may also be written as `"09:15-11:30"`.
    pub fn from_quote_schedule_json(json: Option<&str>) -> Result<Self, String> {
        let Some(json) = json.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::default());
        };
        let value: Value = serde_json::from_str(json)
            .map_err(|error| format!("quote_schedule 不是有效 JSON: {error}"))?;
        let object = value
            .as_object()
            .ok_or_else(|| "quote_schedule 必须是 JSON 对象".to_string())?;

        let windows = match object.get("sessions") {
            None => Self::default().windows,
            Some(Value::Array(items)) if !items.is_empty() => items
                .iter()
                .enumerate()
                .map(|(index, item)| parse_window(item, index))
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::Array(_)) => return Err("quote_schedule.sessions 不能为空".to_string()),
            Some(_) => return Err("quote_schedule.sessions 必须是数组".to_string()),
        };
        validate_windows(&windows)?;

        let mut closed_dates = match object.get("closed_dates") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let text = item.as_str().ok_or_else(|| {
                        format!("quote_schedule.closed_dates[{index}] 必须是 YYYY-MM-DD 字符串")
                    })?;
                    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| {
                        format!("quote_schedule.closed_dates[{index}] 日期无效: {text}")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err("quote_schedule.closed_dates 必须是数组".to_string()),
        };
        closed_dates.sort_unstable();
        closed_dates.dedup();

        Ok(Self {
            windows,
            closed_dates,
        })
    }

    pub fn decision_now(&self) -> MarketGateDecision {
        self.decision_at(Utc::now())
    }

    pub fn decision_at(&self, now: DateTime<Utc>) -> MarketGateDecision {
        let offset = FixedOffset::east_opt(CST_OFFSET_SECONDS).expect("UTC+8 is valid");
        let local = now.with_timezone(&offset);
        let date = local.date_naive();

        if matches!(local.weekday(), Weekday::Sat | Weekday::Sun) {
            return MarketGateDecision::Paused {
                reason: format!("北京时间 {date} 为周末，行情请求已暂停"),
            };
        }
        if self.closed_dates.binary_search(&date).is_ok() {
            return MarketGateDecision::Paused {
                reason: format!("北京时间 {date} 已配置为休市日，行情请求已暂停"),
            };
        }
        if self
            .windows
            .iter()
            .any(|window| local.time() >= window.start && local.time() < window.end)
        {
            return MarketGateDecision::Allowed;
        }

        MarketGateDecision::Paused {
            reason: format!(
                "当前北京时间 {} 不在行情请求时段（{}），行情请求已暂停",
                local.format("%Y-%m-%d %H:%M:%S"),
                self.schedule_text()
            ),
        }
    }

    pub fn ensure_request_allowed(&self) -> Result<(), String> {
        match self.decision_now() {
            MarketGateDecision::Allowed => Ok(()),
            MarketGateDecision::Paused { reason } => Err(reason),
        }
    }

    pub fn schedule_text(&self) -> String {
        self.windows
            .iter()
            .map(|window| {
                format!(
                    "{}-{}",
                    window.start.format("%H:%M"),
                    window.end.format("%H:%M")
                )
            })
            .collect::<Vec<_>>()
            .join("、")
    }
}

fn parse_window(value: &Value, index: usize) -> Result<TradingWindow, String> {
    let (start, end) = if let Some(text) = value.as_str() {
        text.split_once('-').ok_or_else(|| {
            format!("quote_schedule.sessions[{index}] 必须采用 HH:MM-HH:MM 格式")
        })?
    } else if let Some(object) = value.as_object() {
        let start = object
            .get("start")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("quote_schedule.sessions[{index}].start 缺失或格式错误"))?;
        let end = object
            .get("end")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("quote_schedule.sessions[{index}].end 缺失或格式错误"))?;
        (start, end)
    } else {
        return Err(format!(
            "quote_schedule.sessions[{index}] 必须是时段对象或字符串"
        ));
    };

    Ok(TradingWindow {
        start: parse_time(start).map_err(|message| format!("sessions[{index}].start: {message}"))?,
        end: parse_time(end).map_err(|message| format!("sessions[{index}].end: {message}"))?,
    })
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| format!("时间无效: {value}（应为 HH:MM）"))
}

fn validate_windows(windows: &[TradingWindow]) -> Result<(), String> {
    for (index, window) in windows.iter().enumerate() {
        if window.start >= window.end {
            return Err(format!("quote_schedule.sessions[{index}] 开始时间必须早于结束时间"));
        }
        if index > 0 && windows[index - 1].start > window.start {
            return Err("quote_schedule.sessions 必须按开始时间升序排列".to_string());
        }
        if index > 0 && windows[index - 1].end > window.start {
            return Err("quote_schedule.sessions 不能重叠".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn default_policy_allows_auction_and_both_sessions() {
        let policy = MarketRequestPolicy::default();
        assert_eq!(policy.decision_at(utc(2026, 9, 9, 1, 15)), MarketGateDecision::Allowed);
        assert_eq!(policy.decision_at(utc(2026, 9, 9, 3, 29)), MarketGateDecision::Allowed);
        assert_eq!(policy.decision_at(utc(2026, 9, 9, 5, 0)), MarketGateDecision::Allowed);
        assert_eq!(policy.decision_at(utc(2026, 9, 9, 6, 59)), MarketGateDecision::Allowed);
    }

    #[test]
    fn default_policy_blocks_boundaries_lunch_and_weekends() {
        let policy = MarketRequestPolicy::default();
        assert!(matches!(policy.decision_at(utc(2026, 9, 9, 3, 30)), MarketGateDecision::Paused { .. }));
        assert!(matches!(policy.decision_at(utc(2026, 9, 9, 7, 0)), MarketGateDecision::Paused { .. }));
        assert!(matches!(policy.decision_at(utc(2026, 9, 12, 2, 0)), MarketGateDecision::Paused { .. }));
    }

    #[test]
    fn configured_sessions_and_closed_dates_are_enforced() {
        let policy = MarketRequestPolicy::from_quote_schedule_json(Some(
            r#"{"sessions":["09:30-11:00",{"start":"13:30","end":"14:30"}],"closed_dates":["2026-09-09"]}"#,
        ))
        .unwrap();
        assert!(matches!(policy.decision_at(utc(2026, 9, 9, 2, 0)), MarketGateDecision::Paused { .. }));
        assert_eq!(policy.decision_at(utc(2026, 9, 10, 1, 30)), MarketGateDecision::Allowed);
        assert!(matches!(policy.decision_at(utc(2026, 9, 10, 1, 29)), MarketGateDecision::Paused { .. }));
    }

    #[test]
    fn invalid_or_overlapping_config_is_rejected() {
        assert!(MarketRequestPolicy::from_quote_schedule_json(Some(
            r#"{"sessions":[{"start":"11:00","end":"10:00"}]}"#
        ))
        .is_err());
        assert!(MarketRequestPolicy::from_quote_schedule_json(Some(
            r#"{"sessions":["09:15-11:30","11:00-12:00"]}"#
        ))
        .is_err());
        assert!(MarketRequestPolicy::from_quote_schedule_json(Some("not-json")).is_err());
    }
}
